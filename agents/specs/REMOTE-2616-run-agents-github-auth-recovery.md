*Spec: keep an accepted `run_agents` action alive while remote children recover from GitHub authentication (REMOTE-2616)*

== PRODUCT ==
*Summary:* When a user accepts a `run_agents` confirmation card and one or more remote children are rejected by the server with "GitHub authentication required", the whole batch resolves immediately as failed. The user authenticates, the hidden child pane silently retries and succeeds, but the parent action already finished, so the orchestrator is told its children failed and the recovered children are orphaned. This change keeps the accepted action alive in a new nonterminal state while any child is recoverably auth-blocked, resolves each child exactly once when authentication completes, and produces one `RunAgentsResult` that reflects the recovered outcome. The change is client-only.

*Key design choices:*
1. Add a distinct nonterminal action status, `AIActionStatus::BlockedOnUserAction`, instead of reusing the pending-approval `AIActionStatus::Blocked`. The action stays in `running_actions`; it is never requeued, re-drained, or re-approved, and the parent `ConversationStatus` stays `InProgress`.
2. Carry the blocker as typed data (public message plus normalized auth URL) on a new per-child `StartAgent` update stream. The current one-shot `StartAgentOutcome` channel cannot express "blocked, still alive".
3. Resolve the race surface with explicit counters: a GitHub auth *completion generation* on the notifier and a per-child *attempt number*. Every callback-before-blocker, duplicate callback, repeated blocker, timeout, and cancellation race retries exactly once when a retry is warranted, and never otherwise.
4. Present one grouped call to action. Per-child blocker state is retained internally, but the card shows grouped launched/blocked counts and a single "Authenticate with GitHub" button, with individual rows only for distinct permanent failures.
5. Keep the post-confirmation card visually identical to the existing cloud-agent status card family. No resolved run-wide parameters (model, harness, environment, runner, host) are rendered after acceptance.

*Product behavior* (numbered, testable invariants from the consumer's view):
1. A user accepts a `run_agents` card whose children are remote. The server rejects one child with a GitHub-auth blocker. The action does not finish. The card shows a blocked state with a single "Authenticate with GitHub" button, and the parent agent receives no tool result yet.
2. The card groups its counts: it states how many agents launched and how many are waiting for GitHub authentication. It does not list one row per blocked child.
3. When the batch contains launched, failed, launching, and blocked children at the same time, the action is still blocked. The presence of any one recoverably blocked child is sufficient.
4. Distinct permanent failures are listed as individual rows on the card. Two children that failed with the same message collapse into one row with a count.
5. One GitHub authentication completion resolves every eligible blocked child in the batch, across every blocked batch in the process. The user authenticates once, not once per child.
6. Each blocked child retries at most once per authentication completion. A duplicate or repeated completion callback does not start a second retry for the same child and the same completion.
7. If a child's retry is itself rejected with a GitHub-auth blocker again, the child returns to the blocked state and is eligible for the next authentication completion.
8. Once no blocker remains, the card returns to its "Spawning N agents…" appearance while automated work continues, and the action's status returns to running.
9. When every child has reached a terminal outcome, the action finishes once with a single `RunAgentsResult::Launched`. Children that recovered report `Launched` with their real agent id; children that never recovered report `Failed`.
10. The results in that single result preserve the original `agent_run_configs` input order, regardless of the order in which children recovered.
11. A child that is blocked and then never resolved does not stall the batch indefinitely on its own: the batch finishes when the user cancels, and the 30-second per-attempt spawn timeout still bounds each automated attempt.
12. The 30-second spawn timeout does not consume the time the user spends authenticating. A child blocked for five minutes and then retried gets a full fresh 30-second attempt.
13. Cancelling the parent conversation while the action is blocked finishes the action as cancelled, drops every unresolved retained child request, and ignores any child update that arrives afterwards. Children that already launched keep running on the server; they are not killed.
14. The blocked state is not an approval prompt. The terminal input is not hidden or captured for confirmation, Enter and Ctrl-C are not rebound to accept/reject, no CLI-agent `permission_request` notification is emitted, and no transcript label reads "awaiting approval".
15. Both front-ends show the blocked state. The GUI shows the grouped card with a clickable button; the TUI shows a blocked tool-call section with the auth URL to open.
16. Nothing outside `run_agents` changes. Every other tool call keeps its existing queued, blocked-on-approval, running, and finished behavior and appearance.

== TECH ==
*Context:* File and line references are pinned to the working checkout at commit `aa9f3a43`. The change is confined to the `warp` repository; `warp-server` and `warp-proto-apis` are untouched.

How an auth-blocked remote child is lost today, end to end:
- The accepted card dispatches through `BlocklistAIActionModel::execute_run_agents` (`app/src/ai/blocklist/action_model.rs:685-712`), which calls `execute_action` → `start_pending_action_by_id` (`:859-914`). The `RunAgents` action executes async, so the model calls `add_running_action` and the action lands in `running_actions` (`:229`, `:446-461`).
- `RunAgentsExecutor::dispatch_children_for_prepared_request` (`app/src/ai/blocklist/action_model/execute/run_agents.rs:222-378`) builds one `ChildSlot` per config and calls `StartAgentExecutor::dispatch` for each (`:272-283`). It then spawns one aggregator that walks the slots **sequentially** (`:296-334`), awaiting each child's single `StartAgentOutcome` against a 30-second `SPAWN_TIMEOUT` (`:47`, `:300-330`).
- `StartAgentExecutor::dispatch` (`app/src/ai/blocklist/action_model/execute/start_agent.rs:246-278`) registers a `PendingStartAgent` with a one-shot `async_channel::Sender<StartAgentOutcome>` and emits `StartAgentExecutorEvent::CreateAgent`.
- The GUI materializes the child in a hidden ambient pane: `launch_remote_child` (`app/src/pane_group/pane/terminal_pane.rs:1876-1969`) → `AmbientAgentViewModel::spawn_agent_with_request` (`app/src/terminal/view/ambient_agent/model.rs:1089-1120`). The TUI materializes it through `TuiOrchestrationModel::register_remote_child_session` (`crates/warp_tui/src/orchestration_model.rs:433-473`).
- The server rejection is classified by `classify_cloud_agent_startup_error` (`app/src/ai/orchestration/remote_child.rs:325-370`) into `CloudAgentStartupIssue::Blocked(CloudAgentStartupBlocker::GitHubAuthRequired { message, auth_url })`, with `auth_url` already normalized by `github_auth_url::cloud_setup_auth_url_with_next` (`:331`).
- **Where the child is lost.** Each front-end converts that blocker into `ConversationStatus::Blocked { blocked_action: <string> }` — GUI at `app/src/terminal/view/ambient_agent/view_impl.rs:300-315` using the constant `CHILD_AGENT_GITHUB_AUTH_REQUIRED_BLOCKED_ACTION` (`:41-42`), TUI at `crates/warp_tui/src/orchestration_model.rs:529-544` using `blocker.message()`. `StartAgentExecutor::handle_history_event` observes `UpdatedConversationStatus`, and `start_agent_error_message_for_status` (`start_agent.rs:297-330`) maps `ConversationStatus::Blocked` to `Some(message)`, so `complete_pending_as_error` (`:130-154`) **removes the pending entry** and sends `StartAgentOutcome::Error`. The aggregator records `RunAgentsAgentOutcomeKind::Failed`, and once every slot resolves, the action finishes.
- The hidden pane is deliberately retained: `should_cleanup_failed_child_launch` (`start_agent.rs:286-295`) returns `false` for `Blocked`, and `AmbientAgentViewModel` already subscribes to the process-wide `GitHubAuthNotifier` (`model.rs:208-212`) and retries via `handle_github_auth_completed` → `spawn_internal` (`model.rs:1438-1448`, `:1137-1145`). The retry works. It just has nowhere to report: the `PendingStartAgent` entry is gone and the action already produced a result.
- `GitHubAuthNotifier` (`app/src/ai/ambient_agents/github_auth_notifier.rs`) is a stateless singleton that emits `GitHubAuthEvent::AuthCompleted`. It is fired from the URI callback at `app/src/uri/mod.rs:408`, `:1186`, and `:1192`.

Status plumbing that must be audited:
- `AIActionStatus` (`action_model.rs:72-153`) has five variants and eleven boolean helpers.
- `BlocklistAIActionModel::get_action_status` (`:615-648`) resolves a status by scanning the pending queue first (front-of-queue, not running, not view-only ⇒ `Blocked`), then `running_actions` ⇒ `RunningAsync`, then finished results, then preprocessing. It takes `&self` and an action id only — no `AppContext`.
- `blocked_action_for_conversation` (`:424-435`) returns `None` whenever `running_actions` has an entry for the conversation. This is the single gate that keeps confirmation focus/input plumbing (`get_pending_action`, `get_pending_or_running_action_id`) away from running actions.
- `ActionBlockedOnUserConfirmation` is emitted only from `handle_not_executed_action` (`:809-832`) and the test-only `queue_confirmation_action` (`:962-979`). That event is what drives the TUI's CLI-agent `permission_request` OSC notification (`crates/warp_tui/src/cli_agent_osc_event_publisher.rs:99-137`).
- Cancellation of a running action routes `cancel_action_with_id` (`:1061-1090`) → `executor.cancel_running_async_action` → `RunAgentsExecutor::cancel_execution` (`run_agents.rs:108-122`), which **only** handles the `PendingRunAgents::Publishing` phase and no-ops during `Spawning`.
- Exhaustive `AIActionStatus` match sites (the compiler will flag all of these): GUI `action_icon` (`app/src/ai/blocklist/block/view_impl/output.rs:3826-3871`), the per-tool renderers in the same file (`:1486-1630`, `:1993`, `:2126`, `:2311`, `:2428`, `:2536`, `:2640`, `:2741`, `:2858`, `:2935`, `:3250`), `requested_command.rs` (`:1185-1227`, `:1378-1415`, `:1519-1625`, `:1935-1976`), `code_diff_view.rs:617-618`, `search_codebase.rs:459-487`, `ask_user_question_view.rs:362-389`, `run_agents_card_view.rs:796-797`/`:1229-1278`, `block.rs` (`:3449`, `:3562`, `:3766`, `:4073`, `:4569`, `:4718-4827`, `:5628`), `block/cli.rs:1128`, `block/view_impl/orchestration.rs:449-457`, `block/model/helper.rs:154-156`, `handoff/pipeline.rs:442-448`; TUI `tool_call_labels.rs:122-197`, `orchestration_block/render.rs:234-241`, `tui_generic_tool_call_view.rs:358`, `agent_block.rs:977`, `tui_file_edits_view.rs:779`, `tui_shell_command_view.rs:439`, `cli_agent_osc_event_publisher.rs:131`.

*Decisions:*

**D1 — How an accepted-but-blocked action is represented. Compared: a new status variant, projecting the existing `Blocked`, and moving the action back to the pending queue.**
- *A — New nonterminal `AIActionStatus::BlockedOnUserAction` (CHOSEN).* Advantages: the action stays in `running_actions`, so the pending queue, the approval gate (`blocked_action_for_conversation`), the drain loop (`try_to_execute_available_actions`), and the parent `ConversationStatus` all keep their current meaning untouched. Every exhaustive match site becomes a compile error, so the audit is mechanical rather than a manual hunt. The state is self-describing in logs and telemetry. Disadvantages: touches roughly thirty match sites, and every boolean helper (`is_blocked`, `is_running`) must be given an explicit answer for the new variant.
- *B — Project the existing `AIActionStatus::Blocked` from a running action (REJECTED).* Advantages: no new variant, no match-site churn, and the GUI/TUI already have blocked styling. Disadvantages: `Blocked` currently means "front of the pending queue, awaiting the user's approval, nothing is running". Every consumer that reads it — the approval card gate in `orchestration_block/render.rs:236-238`, the "(awaiting approval)" label suffix in `tool_call_labels.rs:188-196`, the `permission_request` CLI notification, the Enter/Ctrl-C bindings in `run_agents_card_view.rs:80-96`, and `ConversationStatus::Blocked` derivation — would silently treat a runtime remediation as a pending approval. Reject/accept keybindings would re-enter `execute_run_agents` on an action that is already running. This is exactly the conflation invariant 14 forbids, and it fails silently rather than at compile time.
- *C — Move the action back to the pending queue while blocked (REJECTED).* Advantages: reuses the existing "action is waiting" machinery wholesale, and the existing `Blocked` projection then becomes truthful. Disadvantages: the action is genuinely still executing — children are launched, a server task may be in flight, and `RunAgentsExecutor::pending` still holds the action id. Requeueing makes `has_unfinished_actions` and the drain loop believe the action can be started again, and `start_pending_action_by_id` would re-dispatch children and duplicate launches. It also lets an unrelated queued action jump ahead and start while `run_agents` is mid-flight, violating the serial-phase barrier. Recovering from the queue requires a bespoke "resume, do not restart" path, which is strictly more machinery than A, not less.
- *Why A:* it is the only option where the blocked state cannot be mistaken for an approval, and the only one where the compiler enumerates the audit for us. The cost is a bounded, mechanical set of match arms.

**D2 — How the blocker reaches the `RunAgents` aggregator. Compared: parsing the conversation status string vs. a typed update stream.**
- *Typed `StartAgentUpdate` stream (CHOSEN).* `StartAgentExecutor` reports the blocker as structured data carrying the public message and the already-normalized auth URL. The aggregator can render a CTA without re-deriving a URL, and the retained request survives the blocker.
- *Recognize the auth blocker by matching `ConversationStatus::Blocked { blocked_action }` against known strings (REJECTED).* The GUI and TUI already write two different strings for the same condition (`view_impl.rs:41-42` vs `orchestration_model.rs:539`). String matching would be a silent-drift trap and could not carry the auth URL at all.

**D3 — Aggregator concurrency. Compared: keep the sequential slot loop vs. process updates concurrently.**
- *Concurrent processing, ordering preserved only for the final result (CHOSEN).* The current loop (`run_agents.rs:296-334`) awaits slot 0 to completion before reading slot 1. If slot 0 blocks on auth for five minutes, slot 1's independent progress is invisible and its timeout does not even start. Concurrency is required for invariant 3 and for a correct per-attempt timeout. Input order is restored once at the end by zipping outcomes back against `agent_run_configs`, which is what `:338-349` already does.
- *Keep it sequential and special-case the blocked child (REJECTED).* Would make the progress snapshot wrong for every sibling behind a blocked child, and would make the 30-second timeout mean "30 seconds after the previous child finished".

**D4 — Card presentation while blocked. Compared: per-child rows with resolved run-wide parameters vs. one grouped CTA.**
- *One grouped CTA, grouped counts, per-child rows only for distinct permanent failures (CHOSEN).* A single GitHub identity gates every child in the process, so N identical "Authenticate with GitHub" buttons are N ways to do one thing. Grouping matches the existing terminal-state copy, which already summarizes as "Spawned {launched} of {total} agents" (`run_agents_card_view.rs:1604-1633`).
- *One row per child, each with its resolved model/harness/environment (REJECTED).* Resolved run-wide parameters are deliberately absent from every post-confirmation cloud-agent surface: `render_status_only_card` (`:1681-1712`) is a single icon-plus-label row, and `render_spawning_card`/`render_terminal_state` both delegate to it. Reintroducing parameter rendering here would make the blocked card the only post-acceptance surface that re-displays configuration the user can no longer change.

**D5 — Retry exactly-once discipline. Compared: retry on every `AuthCompleted` vs. generation + attempt counters.**
- *Completion generations plus per-child attempt numbers (CHOSEN).* `GitHubAuthNotifier` is a process-wide singleton fired from three URI paths (`uri/mod.rs:408`, `:1186`, `:1192`), and one user action can fire it more than once. A monotonic generation makes "have I already consumed this completion?" a total order comparison rather than a timing guess, and it makes the callback-before-blocker race well-defined: a child that records a stale generation at the moment it blocks retries immediately, exactly once.
- *Retry on every `AuthCompleted` with a debounce window (REJECTED).* A time window cannot distinguish a duplicate callback from a genuine second authentication, so it either double-launches or drops a real retry depending on the window.

*Proposed changes:*

**1. New status variant — `app/src/ai/blocklist/action_model.rs:72-153`.**
```rust
pub enum AIActionStatus {
    Preprocessing,
    Queued,
    /// Front of the pending queue, awaiting the user's approval.
    Blocked,
    /// Executing, but paused on a runtime remediation the user must perform.
    /// Never an approval prompt.
    BlockedOnUserAction(BlockedOnUserActionKind),
    RunningAsync,
    Finished(Arc<AIAgentActionResult>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedOnUserActionKind {
    GitHubAuthRequired,
}
```
- `is_blocked()` stays strictly `Blocked` and `is_running()` stays strictly `RunningAsync`. Add `is_blocked_on_user_action()` and `is_in_flight()` (`RunningAsync | BlockedOnUserAction`). Do not widen an existing helper: every call site must be chosen explicitly.
- `get_action_status` (`:615-648`) returns `BlockedOnUserAction(GitHubAuthRequired)` in place of `RunningAsync` when the action id is in the model's blocked-action set (see change 5). Its signature is unchanged.

**2. Typed start-agent update stream — `app/src/ai/blocklist/action_model/execute/start_agent.rs`.**
- Replace the one-shot `async_channel::Sender<StartAgentOutcome>` in `PendingStartAgent` (`:44-49`) with a multi-message sender:
```rust
pub enum StartAgentUpdate {
    /// Recoverable: the request stays retained and may be retried.
    Blocked { attempt: u32, blocker: StartAgentBlocker },
    /// An automated attempt has begun; the caller re-arms its attempt timeout.
    Retrying { attempt: u32 },
    Started { agent_id: String },
    /// Permanent. The request is dropped.
    Failed { error: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartAgentBlocker {
    GitHubAuthRequired {
        /// Public, user-facing text. Safe to render verbatim.
        message: String,
        /// Already normalized via `github_auth_url::cloud_setup_auth_url_with_next`.
        auth_url: String,
    },
}
```
- Extend `PendingStartAgent` with `attempt: u32` and `blocked: Option<StartAgentBlocker>`. Keep `StartAgentOutcome` only if another caller still needs the one-shot shape; otherwise delete it and update `RunAgentsExecutor`, which is the sole consumer today.
- Retention rule: the pending entry is removed **only** on `Started`, permanent `Failed`, attempt timeout exhaustion reported by the caller, or parent cancellation. It is **not** removed on `Blocked`.
- Both front-ends must report the typed blocker rather than leaving `StartAgentExecutor` to infer it from a status string. Add:
```rust
impl StartAgentExecutor {
    pub fn record_child_startup_blocker(
        &mut self,
        child_conversation_id: AIConversationId,
        blocker: StartAgentBlocker,
        ctx: &mut ModelContext<Self>,
    );
    /// The child left the blocked state under its own retry; re-arm the caller's timeout.
    pub fn record_child_startup_retry(
        &mut self,
        child_conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    );
}
```
  Call `record_child_startup_blocker` from the GUI at `app/src/terminal/view/ambient_agent/view_impl.rs:300-315` (alongside the existing `ConversationStatus::Blocked` write, which stays for pill-bar and agent-list display) and from the TUI at `crates/warp_tui/src/orchestration_model.rs:529-544`. Call `record_child_startup_retry` from `AmbientAgentViewModel::handle_github_auth_completed` (`model.rs:1438-1448`) and from the TUI's equivalent retry path.
- `start_agent_error_message_for_status` (`:297-330`): the `ConversationStatus::Blocked` arm must no longer produce an error message. Change its return type to a small enum so the three cases are explicit rather than encoded in `Option<String>`:
```rust
enum ChildStartupSignal {
    /// Permanent; complete the pending request as an error.
    Failed(String),
    /// Recoverable; emit `Blocked` and retain the pending request.
    Blocked,
    /// Still in flight; do nothing.
    InFlight,
}
```
  `Error` and `Cancelled` map to `Failed`; `Blocked` maps to `Blocked`; `InProgress`, `TransientError`, `Success`, and `WaitingForEvents` map to `InFlight`. `should_cleanup_failed_child_launch` (`:286-295`) already returns `false` for `Blocked` and needs no change.
- Guard: a `Blocked` conversation status for which no typed blocker was recorded (an unexpected blocked reason, not GitHub auth) is treated as `Failed`, preserving today's behavior for any non-auth blocker. Log it through `report_error!`.

**3. Per-child slot state machine and concurrent aggregation — `app/src/ai/blocklist/action_model/execute/run_agents.rs`.**
- Replace `ChildSlot` (`:480-483`) with a state machine the executor owns:
```rust
enum ChildLaunchState {
    Launching { attempt: u32, deadline: Instant },
    Blocked { attempt: u32, blocker: StartAgentBlocker, auth_generation: u64 },
    Launched { agent_id: String },
    Failed { error: String },
}
```
  `Launching` and `Blocked` alternate; `Launched` and `Failed` are terminal. `ChildSlot::Failed(err)` produced during request construction (`:260`, `:267-270`) starts directly in `Failed`.
- Replace the sequential `for slot in slots` loop (`:296-334`) with concurrent consumption of every child's update stream (for example a `FuturesUnordered` / `select_all` over `(child_index, receiver)` pairs), driven on the executor so each update mutates the executor-owned state. Input ordering is restored exactly once, where `:338-349` already zips outcomes back against `agent_run_configs_for_result`.
- The executor holds the authoritative snapshot:
```rust
pub struct RunAgentsProgressSnapshot {
    pub total: usize,
    pub launching: usize,
    pub blocked: usize,
    pub launched: usize,
    pub failed: usize,
    /// The grouped CTA target. `None` when nothing is blocked.
    pub blocker: Option<StartAgentBlocker>,
    /// Distinct permanent failures, with the number of children sharing each message.
    pub distinct_failures: Vec<(String, usize)>,
}

impl RunAgentsExecutor {
    pub fn progress_snapshot(&self, action_id: &AIAgentActionId)
        -> Option<RunAgentsProgressSnapshot>;
}
```
  Extend `RunAgentsExecutorEvent` (`:76-84`) with `ProgressChanged { action_id }`. The event carries no state: every consumer re-reads `progress_snapshot`. `RunAgentsSpawningSnapshot` (`:51-54`) is superseded by `RunAgentsProgressSnapshot`; keep `SpawningStarted`/`SpawningFinished` for their existing lifecycle role.
- The grouped `blocker` is the blocker of the lowest-indexed blocked child. All GitHub-auth blockers in a process resolve to the same normalized URL; assert equality in a debug assertion rather than merging text.
- Terminal aggregation is unchanged in shape: when no child is in `Launching` or `Blocked`, build `RunAgentsResult::Launched` exactly as `:337-377` does today, remove the action from `pending`, emit `SpawningFinished`, and send the result once. Sending is idempotent: the aggregator must send at most one result per action id.

**4. Timeout semantics — `run_agents.rs:47`.**
- `SPAWN_TIMEOUT` (30 s) becomes a **per automated attempt** deadline held in `ChildLaunchState::Launching { deadline }`. Entering `Blocked` clears the deadline; human authentication wait is untimed. `StartAgentUpdate::Retrying { attempt }` re-arms a fresh 30-second deadline.
- A deadline that fires while `Launching` moves that child to `Failed` with the existing timeout message (`:322-328`) and removes its retained pending request. A deadline must never fire for a child in `Blocked`.
- Rationale for keeping 30 s rather than raising it: the timeout's purpose (a missing harness binary or a dead server call) is unchanged; the defect was that the clock ran during human wait, not that the budget was too small.

**5. Status projection into the action model — `app/src/ai/blocklist/action_model.rs`.**
- `BlocklistAIActionModel` subscribes to `RunAgentsExecutorEvent::ProgressChanged` and, on each event, **re-reads** `RunAgentsExecutor::progress_snapshot` for that action id and updates a `HashSet<AIAgentActionId>` of actions with `blocked > 0`. The set is derived from the snapshot, never from an event payload, so the executor remains the single source of truth.
- `get_action_status` (`:615-648`) checks that set in the `running_actions` branch: a member returns `BlockedOnUserAction(GitHubAuthRequired)`, a non-member returns `RunningAsync`. The action is never removed from `running_actions` while blocked.
- Explicitly unchanged: `pending_actions`, `blocked_action_for_conversation` (`:424-435`), `try_to_execute_available_actions` (`:468-509`), `handle_not_executed_action` (`:809-832`), and `update_conversation_in_progress_status` (`:794-807`). No `ActionBlockedOnUserConfirmation` event is emitted, and the parent conversation's `ConversationStatus` stays `InProgress`.

**6. GitHub auth completion generations — `app/src/ai/ambient_agents/github_auth_notifier.rs`.**
- Give the notifier a monotonic counter:
```rust
pub struct GitHubAuthNotifier {
    completion_generation: u64,
}

impl GitHubAuthNotifier {
    pub fn completion_generation(&self) -> u64;
    pub fn notify_auth_completed(&mut self, ctx: &mut ModelContext<Self>) {
        self.completion_generation += 1;
        ctx.emit(GitHubAuthEvent::AuthCompleted {
            generation: self.completion_generation,
        });
    }
}
```
  `notify_auth_completed` becomes `&mut self`; update the three URI call sites (`app/src/uri/mod.rs:408`, `:1186`, `:1192`) and `AmbientAgentViewModel`'s subscription (`model.rs:208-212`).
- Race rules, all keyed on the generation recorded when a child enters `Blocked`:
  - *Normal:* a completion with `generation > recorded` triggers exactly one retry for that child.
  - *Callback before blocker:* the child records the current generation at the moment it blocks. If that generation is already greater than the generation in effect when the attempt started, a completion landed during the in-flight attempt; retry once immediately on entering `Blocked`.
  - *Duplicate callback:* a completion whose generation is not greater than the recorded one is ignored.
  - *Repeated blocker:* a retry that blocks again records the new current generation and becomes eligible for the next completion. Attempt numbers increment monotonically and are the idempotency key for `Retrying`/`Blocked` updates.
  - *Timeout race:* a retry that has already been failed by the attempt deadline is terminal; a later completion does not revive it.
  - *Cancellation race:* once the action is cancelled, the notifier subscription is dropped and any later completion or child update for that action is ignored.
- Every child's own `AmbientAgentViewModel` retry (`handle_github_auth_completed`) stays as the retry mechanism. The generation check lives with the retained `PendingStartAgent` state so one completion fans out to every eligible blocked child in the process, satisfying invariant 5.

**7. Cancellation — `run_agents.rs:108-122` and `app/src/ai/blocklist/action_model.rs:1061-1090`.**
- Extend `RunAgentsExecutor::cancel_execution` to handle `PendingRunAgents::Spawning`: drop the retained `PendingStartAgent` entry for every child still in `Launching` or `Blocked`, mark the action cancelled so late updates are discarded, emit `SpawningFinished`, and send `RunAgentsResult::Cancelled` once.
- Cancel a raced in-flight server spawn task: a child whose `spawn_agent` future is still pending has its task aborted, and if the server response arrives anyway it is discarded rather than recorded as `Launched`.
- Children already in `Launched` are **not** cancelled or killed: their runs continue server-side, matching the existing contract that a launched child outlives the tool call.

**8. GUI presentation — `app/src/ai/blocklist/inline_action/run_agents_card_view.rs` and `block/view_impl/output.rs`.**
- `RunAgentsCardView::render` (`:1224-1316`) gains a branch, placed after the `Finished` branch and before the spawning branch: when `get_action_status` is `BlockedOnUserAction(GitHubAuthRequired)`, read `RunAgentsExecutor::progress_snapshot` and render the grouped blocked card.
- The grouped blocked card is built from the same primitives as `render_status_only_card` (`:1681-1712`) — the attention icon plus one label row — extended with:
  - a grouped count line: launched count and blocked count out of the total (for example `1 launched · 2 waiting for GitHub authentication`);
  - the blocker's public `message` verbatim;
  - one "Authenticate with GitHub" button opening the snapshot's normalized `auth_url`, reusing `CloudAgentStartupPresentation::github_auth(auth_url, CloudAgentStartupAuthFlow::RetryRetainedRequest)` (`app/src/ai/orchestration/remote_child.rs:206-221`) for title, detail, and action label. `RetryRetainedRequest` is the correct flow here: the request is retained, so the copy must be "Please authenticate with GitHub to continue", not "…then run the orchestration request again".
  - one row per **distinct** permanent failure from `distinct_failures`, with a count when more than one child shares the message.
- The card renders **no** resolved run-wide parameters. `render_editor` (`:1714-1773`), the mode toggle, and the picker row are pre-acceptance only and must not appear in this branch.
- The card is not interactive as an approval: the `Accept`/`Reject` fixed bindings (`:80-96`) must not act in this state. `handle_accept` already returns early when `self.spawning.is_some()` (`:713-716`); extend the same guard to the blocked state, and make `RunAgentsCardViewAction::Reject` route to the cancellation path (change 7) rather than emitting `RejectRequested`.
- `action_icon` (`output.rs:3826-3871`): the new variant maps to `icons::yellow_stop_icon` (attention, static), distinct from `Blocked`'s identical glyph only in that it is reached from a running action, and distinct from `RunningAsync`'s `yellow_running_icon`.
- Every other exhaustive match site in `output.rs`, `requested_command.rs`, `code_diff_view.rs`, `search_codebase.rs`, `ask_user_question_view.rs`, `block.rs`, `block/cli.rs`, `block/view_impl/orchestration.rs`, `block/model/helper.rs`, and `handoff/pipeline.rs` adds the new variant **to the existing `RunningAsync` arm**. Only `RunAgents` actions can reach the state, so those renderers keep their in-flight behavior. Do not add a wildcard arm (`AGENTS.md`, "Exhaustive Matching").

**9. TUI presentation — `crates/warp_tui/src/`.**
- `tool_call_labels.rs`: add `ToolCallDisplayState::BlockedOnUserAction` (`:51-64`) mapped from the new status in `tool_call_display_state` (`:122-157`). Glyph `■` with `attention_glyph_style` (same as `Blocked`); `label_style` matches `Blocked`. In `tool_call_label_with_server` (`:176-197`) the new state must **not** receive the `" (awaiting approval)"` suffix; it receives `" (waiting for GitHub authentication)"`.
- `orchestration_block/render.rs:234-241`: the `interactive` gate stays `matches!(status, Some(AIActionStatus::Blocked))`, so a blocked-on-user-action run_agents block falls through to `render_fallback_tool_call_section`. Extend that fallback for this state to append the blocker's public message and the `auth_url` on its own line so the user can open it. The acceptance/configuring card and its Enter/Ctrl-E/Ctrl-C footer (`render.rs:210-230`) never appear in this state.
- `cli_agent_osc_event_publisher.rs:99-137`: unchanged by construction, because no `ActionBlockedOnUserConfirmation` is emitted. Add an explicit regression test asserting no `permission_request` notification fires for this state. Emitting a distinct remediation notification is deliberately out of scope (see Out of scope).
- `agent_block.rs:977`, `tui_generic_tool_call_view.rs:358`, `tui_file_edits_view.rs:779`, `tui_shell_command_view.rs:439`: add the new variant to the existing running arm.

**10. No server or protocol change.** `warp-server`, `warp-proto-apis`, `SpawnAgentRequest`, `RunAgentsResult`, and `RunAgentsAgentOutcomeKind` are unchanged. The recovered child reports `Launched { agent_id }` through the existing wire shape.

*Assumptions:*
- **A1.** Every GitHub-auth blocker in one client process resolves to the same normalized auth URL, because `cloud_setup_auth_url_with_next` (`app/src/ai/ambient_agents/github_auth_url.rs:53-59`) derives it from process-wide channel state. Recorded as an assumption and enforced with a debug assertion; if it turns out false, the grouped CTA must fall back to per-child rows.
- **A2.** `CloudAgentStartupBlocker::GitHubAuthRequired` is the only recoverable startup blocker today (`remote_child.rs:134-137`). `BlockedOnUserActionKind` is an enum rather than a unit so a second remediation can be added without another status variant, but only the GitHub-auth kind is implemented.
- **A3.** The blocker `message` returned by `ClientError.error` and surfaced through `CloudAgentStartupBlocker::message()` is already user-facing and safe to render verbatim; it is rendered as-is and not reformatted.
- **A4.** Computer-use visual proof was not opted into for this change, by the request or by review. The validation criteria below therefore require only automated tests and the repository's documented checks. Both front-ends do change user-visible surfaces, so this is an explicit question for the reviewer: if visual proof is wanted, a recording of the blocked card and the authenticate-and-recover flow should be added as a validation criterion before implementation starts.
- **A5.** Local (non-remote) children never reach this state, because the GitHub-auth blocker originates from the server spawn path used only by remote children. Mixed local/remote batches therefore block only on their remote members.

*Out of scope:*
- Any `warp-server` or proto change. The recovery is entirely client-side, so the server keeps returning the same `auth_url`-bearing client error.
- A new CLI-agent OSC notification for runtime remediation. This change only guarantees that the existing `permission_request` event is **not** emitted for it; adding a `remediation_required` event is a separate product decision about the CLI-agent notification contract.
- Recovering a child that failed **permanently** (capacity, out of credits, server overloaded, other). `CloudAgentStartupFailure` (`remote_child.rs:160-165`) stays terminal and eligible for `CleanupFailedChildLaunch`; only `CloudAgentStartupIssue::Blocked` is recoverable.
- Recovering an auth blocker that appears on a **follow-up** run or mid-run, rather than at initial child launch. `handle_needs_github_auth` already clears the retained request for non-initial startup kinds (`model.rs:1424-1426`); that behavior is unchanged.
- A user-visible retry button. Recovery is driven by the authentication completion, not by a manual retry control.
- Raising or making configurable the 30-second attempt budget.
- Auto-opening the browser on behalf of the user. The CTA remains an explicit click (GUI) or a printed URL (TUI).

*Risks / blast radius:*
- The new status variant touches roughly thirty match sites across both front-ends. All are exhaustive matches, so omissions fail to compile rather than misbehave; the real risk is the boolean helpers (`is_blocked`, `is_running`), which is why neither is widened and each call site is decided explicitly.
- Making the aggregator concurrent changes the ordering in which `RunAgentsAgentOutcome`s are produced. Input order is restored by the existing zip against `agent_run_configs_for_result`; criterion 6 below pins that.
- Retaining `PendingStartAgent` entries across a blocker creates a leak surface if a resolution path is missed. The retention rule has exactly four removal triggers (start, permanent failure, timeout, cancellation), and criterion 10 asserts no retained request survives cancellation.
- `notify_auth_completed` becomes `&mut self`, which is a small mechanical ripple through three URI call sites.
- The blocked card and TUI section are new user-visible surfaces on the orchestration hot path. Both are built from existing primitives (`render_status_only_card`, `render_fallback_tool_call_section`) to keep the visual blast radius small.

*Validation criteria* (must ALL pass before merge):
1. **Regression — a blocked child does not finish the action.** New test in `app/src/ai/blocklist/action_model/execute/run_agents_tests.rs`, `run_agents_blocked_child_keeps_action_running`: dispatch a two-child remote batch, drive one child to `ConversationStatus::Blocked` with a recorded `StartAgentBlocker::GitHubAuthRequired`, and assert no `RunAgentsResult` was sent and `get_action_status` is `BlockedOnUserAction(GitHubAuthRequired)`. Fails today (the action finishes with a `Failed` child). Verifies invariant 1.
2. **Status shape — the action stays running and is never requeued.** `run_agents_blocked_child_stays_in_running_actions`: assert the action id is still in `running_actions`, `get_pending_action` / `blocked_action_for_conversation` return `None` for the conversation, no `BlocklistAIActionEvent::ActionBlockedOnUserConfirmation` was emitted, and the parent `ConversationStatus` is `InProgress`. Verifies invariants 1 and 14.
3. **Partial batch projection.** `run_agents_partial_batch_projects_blocked`: a four-child batch with one `Launched`, one `Failed`, one `Launching`, one `Blocked` projects `BlockedOnUserAction`. Then resolve the blocked child and assert the status returns to `RunningAsync` while the `Launching` child is still outstanding. Verifies invariants 3 and 8.
4. **One completion resolves every eligible child.** `github_auth_completion_resolves_all_blocked_children`: with three blocked children across two actions, fire one `GitHubAuthEvent::AuthCompleted` and assert each child issues exactly one retry. Verifies invariant 5.
5. **Race matrix — exactly-once retry.** Six named tests in `run_agents_tests.rs`, each asserting the retry count for the affected child: `auth_callback_before_blocker_retries_once`, `duplicate_auth_callback_does_not_retry_twice`, `repeated_blocker_retries_on_next_completion`, `attempt_timeout_after_blocker_is_terminal`, `completion_after_timeout_does_not_revive_child`, `completion_after_cancellation_is_ignored`. Verifies invariants 6, 7, 11 and decision D5.
6. **Input ordering preserved.** `run_agents_result_preserves_input_order`: children recover in reverse order; assert the `agents` vec in the emitted `RunAgentsResult::Launched` matches `agent_run_configs` order by name. Verifies invariant 10.
7. **Recovered child reports Launched.** `recovered_child_reports_launched_outcome`: a child that blocks, recovers, and receives an agent id reports `RunAgentsAgentOutcomeKind::Launched { agent_id }` in the single final result, and exactly one result is sent. Verifies invariant 9.
8. **Timeout pauses during human wait.** `attempt_timeout_pauses_while_blocked`: advance the test clock well past 30 s while the child is `Blocked` and assert it has not failed; then emit `Retrying` and assert a fresh 30-second deadline is armed and the child fails only after that fresh deadline elapses. Verifies invariant 12.
9. **Permanent failures stay terminal and are deduped.** `permanent_failures_are_not_retried_and_are_deduped`: two children fail with the same message and one with a different message; assert `progress_snapshot().distinct_failures` has two entries with counts `2` and `1`, and that no `AuthCompleted` retries any of them. Verifies invariants 4 and the out-of-scope permanent-failure rule.
10. **Cancellation.** `cancelling_blocked_run_agents_clears_retained_requests`: cancel the parent while one child is blocked and one is launched; assert a single `RunAgentsResult::Cancelled`, zero retained `PendingStartAgent` entries, that a late child update is ignored, that a raced in-flight spawn task is aborted and its late response discarded, and that no cancel request is issued for the already-launched child. Verifies invariant 13.
11. **GUI card.** New tests in `app/src/ai/blocklist/inline_action/run_agents_card_view_tests.rs`: `blocked_card_renders_grouped_cta` (one authenticate button, grouped launched/blocked counts, no per-blocked-child rows), `blocked_card_lists_distinct_permanent_failures`, `blocked_card_renders_no_resolved_run_wide_parameters` (asserts the editor, mode toggle, and picker row are absent), and `blocked_card_accept_binding_is_inert` (Enter does not re-enter `execute_run_agents`; Ctrl-C routes to cancellation). Verifies invariants 2, 4, 14, 15 and decision D4.
12. **GUI icon.** `action_icon_uses_attention_glyph_for_blocked_on_user_action` in the `output.rs` test module. Verifies invariant 15.
13. **TUI label and section.** In `crates/warp_tui/src/tool_call_labels_tests.rs`, `blocked_on_user_action_label_has_no_awaiting_approval_suffix` asserts the label ends with `(waiting for GitHub authentication)` and contains no `awaiting approval`. In `crates/warp_tui/src/orchestration_block_tests.rs`, `blocked_on_user_action_renders_fallback_with_auth_url` asserts the rendered lines contain the blocker message and the auth URL and do **not** contain the acceptance footer (`to accept`, `to reject`). Verifies invariants 14 and 15.
14. **CLI permission events.** `crates/warp_tui/src/cli_agent_osc_event_publisher_tests.rs`, `blocked_on_user_action_emits_no_permission_request`: assert no `permission_request` notification is published for a run_agents action in the new state. Verifies invariant 14.
15. **No collateral behavior change for other tools.** The existing suites for `requested_command`, `code_diff_view`, `search_codebase`, `ask_user_question`, `tui_shell_command_view`, `tui_file_edits_view`, and `tui_generic_tool_call_view` pass unchanged. Verifies invariant 16.
16. **Exhaustiveness.** No wildcard `_` arm is added to any `AIActionStatus` match (`AGENTS.md`, "Exhaustive Matching"); confirmed by review of the diff and by the crates compiling with `-D warnings`.
17. **Repository gate.** From the `warp` repo root: `./script/format`, `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`, `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`, and a full `./script/presubmit` before the PR is marked ready. New tests live in separate `*_tests.rs` files per the repo's no-inline-test-modules rule.
18. **Manual end-to-end (original repro).** With a client whose GitHub integration is not authorized, accept a `run_agents` card with two remote children, observe the grouped blocked card, complete GitHub OAuth, and confirm both children launch and the parent agent receives one `Launched` result naming both agent ids. Evidence: the run link and the resulting tool result attached to the ticket. This criterion cannot be satisfied by unit tests alone because it exercises the real server rejection and the real URI callback.
