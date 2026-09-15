# Spec: Keep an accepted `run_agents` action alive when children need GitHub auth (REMOTE-2616)

## Summary

When a `run_agents` action fans out remote children and one or more children block on GitHub
authentication, the parent action today resolves as failed (or partially failed) even though the
user can recover the launch by completing OAuth. This spec keeps the accepted action alive in a new
nonterminal `AIActionStatus::BlockedOnUserAction` state, retains the pending child requests across
the human-auth wait, retries each recoverable child exactly once per process-wide OAuth completion,
and renders one grouped remediation card. The change is client-only: no proto, server, or
`RunAgentsResult` wire changes. This spec supersedes the CODE-1822 TUI stance of
"report a failed child outcome and never automatically retry"
(`specs/code-1822-tui-cloud-children/TECH.md`, design step 4.7) for the auth-recovery flow.

## Current behavior and evidence

- `AIActionStatus` is derived from queue membership, not stored per action
  (`app/src/ai/blocklist/action_model.rs:74`, `:616-648`): front of `pending_actions` with no
  running entry → `Blocked`; in `running_actions` → `RunningAsync`; else `Finished`/`Preprocessing`.
- A pending-approval action stamps the parent `ConversationStatus::Blocked { blocked_action }` and
  emits `ActionBlockedOnUserConfirmation` (`action_model.rs:809-832`). This is the approval
  semantic, not a runtime-remediation semantic.
- `RunAgentsExecutor` dispatches children through `StartAgentExecutor::dispatch` and aggregates one
  `StartAgentOutcome` per child sequentially in input order with a 30-second `SPAWN_TIMEOUT` per
  slot (`execute/run_agents.rs:47`, `:222-378`). `StartAgentOutcome` is single-shot:
  `Started { agent_id } | Error(String)` (`execute/start_agent.rs:13-19`).
- When a remote child's spawn fails with a recoverable GitHub-auth blocker, the GUI child pane sets
  `Status::NeedsGithubAuth` and the child conversation to
  `ConversationStatus::Blocked { blocked_action }`
  (`terminal/view/ambient_agent/model.rs:1395-1436`, `terminal/view/ambient_agent/view_impl.rs:300-315`).
  `start_agent_error_message_for_status` maps that `Blocked` status to an error, so
  `StartAgentExecutor` resolves the pending request as a failure
  (`execute/start_agent.rs:297-330`). The action then completes with per-child `Failed` outcomes.
  Only terminal `Error`/`Cancelled` states clean up the child chip;
  blocked children keep it (`execute/start_agent.rs:286-295`).
- The structured blocker already exists and is frontend-neutral:
  `CloudAgentStartupBlocker::GitHubAuthRequired { message, auth_url }` with the normalized URL from
  `github_auth_url::cloud_setup_auth_url_with_next`
  (`app/src/ai/orchestration/remote_child.rs:135-137`, `:325-333`), plus
  `CloudAgentStartupAuthFlow::RetryRetainedRequest` (`remote_child.rs:179-185`).
- A process-wide OAuth completion signal exists: `GitHubAuthNotifier::AuthCompleted`
  (`app/src/ai/ambient_agents/github_auth_notifier.rs`), fired from the OAuth redirect handler
  (`app/src/uri/mod.rs:1148-1200`). The GUI pane model already re-spawns a retained initial run on
  this signal for user-initiated cloud runs
  (`terminal/view/ambient_agent/model.rs:208-212`, `:1438-1448`) — but not for orchestrated
  children without double-spawning, because the executor resolves the pending request first.
- RunAgents consumers that assume `Blocked` means "awaiting approval": TUI
  `tool_call_display_state` + "(awaiting approval)" suffix
  (`crates/warp_tui/src/tool_call_labels.rs:122-157`, `:189`), TUI permission-card gating
  (`crates/warp_tui/src/tui_generic_tool_call_view.rs:353-369`), TUI orchestration block
  interactivity (`crates/warp_tui/src/orchestration_block.rs:467-476`,
  `orchestration_block/render.rs:234-241`), TUI input-focus source
  (`crates/warp_tui/src/agent_block.rs:965-1005`), and OSC `permission_request` publishing
  (`crates/warp_tui/src/cli_agent_osc_event_publisher.rs:99-137`).

## Decisions

### D1. Status shape: new variant (chosen)

- **Option A (chosen): add a distinct nonterminal `AIActionStatus::BlockedOnUserAction`.**
  Advantages: expresses a third state (running, but paused on human remediation) without touching
  the meaning of any existing state; the action stays in `running_actions`, so the executor's
  aggregation, cancellation, and result-ordering paths keep working unchanged; each consumer opts
  in explicitly during the consumer audit. Disadvantages: every `match` on `AIActionStatus` in both
  frontends must be extended (exhaustive matches make the work mechanical and compiler-checked).
- **Option B (rejected): project the existing `Blocked` variant from a running action.**
  `Blocked` is derived from being at the front of `pending_actions` with nothing running
  (`action_model.rs:616-628`); a running action cannot naturally produce it. Every consumer
  already binds `Blocked` to approval semantics (confirmation cards, permission prompts, input
  focus theft, OSC permission events, notification summaries). Overloading it would force each
  consumer to disambiguate "approval-blocked vs. remediation-blocked" with ad-hoc running-state
  checks — the exact ambiguity this spec removes.
- **Option C (rejected): move the action back to the pending queue.** Re-queueing would re-run
  autoexecute/confirmation policy, re-drain preprocessing, re-participate in
  `try_to_execute_available_actions`, stamp the parent `ConversationStatus::Blocked`, and re-sort
  results via `action_order` — re-entering code paths designed for actions that have not started.
  It also invites double-launch: the executor still holds in-flight child dispatches.

### D2. Retry ownership: shared `StartAgentExecutor` (chosen)

- **Chosen:** the shared executor retains the full `StartAgentRequest` in its pending entry and
  re-dispatches on OAuth completion. This gives one retry path for both frontends (the TUI child has
  no `AmbientAgentViewModel` to re-spawn), and it is where the pending request already lives.
- **Rejected:** per-pane retry for GUI children (exists today for user-initiated runs) — the TUI has
  no equivalent, and leaving the GUI path on would double-spawn once the shared path also retries.
  The pane-level auto-retry is suppressed for orchestrated (child-agent) conversations.

### D3. Timeout semantics: per automated attempt, paused on human wait (chosen)

The existing 30-second `SPAWN_TIMEOUT` bounds one automated launch attempt. While a blocker is
outstanding, no timer runs (the human may take minutes). Each retry re-arms a fresh 30-second
timer. The alternative — a single action-level deadline — would kill a recoverable batch while the
user is mid-OAuth, which is the bug class this spec fixes.

### D4. Grouped CTA (chosen)

One card on the parent surface shows one "Authenticate with GitHub" CTA, launched and blocked
counts, and rows only for distinct permanent failures. Per-child rows for blocked children add
noise for what is one remediation; permanent failures are the only rows a user can act on
individually (via the linked agent id in the failure text). Per-child state stays internal to the
executor.

## Product behavior

1. After the user accepts a `run_agents` action, if one or more remote children report a recoverable
   GitHub-auth blocker, the parent conversation stays `InProgress` and the action becomes
   `BlockedOnUserAction`. The transcript shows one grouped card: "GitHub Authentication Required",
   the blocker's public message, an "Authenticate with GitHub" CTA opening the normalized URL,
   and counts in the form "N running · M awaiting authentication".
2. While blocked, no timeout fires for a blocked child. Launched siblings keep running; launching,
   retrying, and failed siblings continue their own transitions.
3. When OAuth completes, every blocked child retries automatically with its retained request.
   If the retry launches, the card returns to the spawning state ("Spawning N agents…"); counts
   update.
4. If a retried child blocks again, the card returns to the grouped blocked state and waits for the
   next OAuth completion. Each completion produces at most one retry per child.
5. If a retried child fails permanently (error or 30-second attempt timeout), the card shows a row
   for that distinct failure and drops it from the blocked count.
6. When no blocked child remains, the action returns to `RunningAsync` while automated work
   continues, then completes with the normal `RunAgentsResult::Launched` (per-child
   `Launched`/`Failed` outcomes, input order). A batch whose children all fail permanently still
   resolves as today: `Launched` with zero launched children rendering as "Failed to spawn N
   agents".
7. Cancelling the parent conversation while blocked: clears all retained requests, ignores late
   child updates, cancels a raced server task, and completes the action as `Cancelled`. Already
   launched siblings keep running.
8. Rejecting or confirming never appears: the card has no Accept/Reject buttons, does not steal
   keyboard focus or replace the input, and no `permission_request` OSC event or "awaiting
   approval" label is produced for this state.
9. In autonomous (non-interactive) execution, a blocked child resolves as a permanent failure with
   the blocker message; the action never waits on a user who cannot act (see Assumptions).
10. The blocked state is not persisted. Restored transcripts render the terminal result as today.

## Technical design

### Status and action-model changes (`app/src/ai/blocklist/action_model.rs`)

- Add `AIActionStatus::BlockedOnUserAction` after `RunningAsync` (line 74). It is nonterminal and
  non-serializing (`AIActionStatus` is derived in memory; nothing persists it).
- Add accessors `is_blocked_on_user_action()`; leave `is_running()` bound to `RunningAsync`.
- Extend `get_action_status` derivation (lines 616-648): the executor records, per action id, a
  user-action blocker overlay (`HashMap<AIAgentActionId, ()>` is sufficient — payload lives in the
  executor). While an action is in `running_actions` and the overlay is present, return
  `BlockedOnUserAction`; when the overlay is cleared, the existing `RunningAsync` derivation
  resumes. The action never leaves `running_actions` during the blocked window, so
  `handle_action_result` bookkeeping (lines 1301-1407) is untouched.
- Do not emit `ActionBlockedOnUserConfirmation` from this path, and do not call
  `update_conversation_status` with `Blocked`. The parent `ConversationStatus` stays `InProgress`.
- No requeue, no `execute_run_agents`/`deny_run_agents` reuse, no preprocessing re-run.

### Typed start-agent update stream (`execute/start_agent.rs`)

- Extend `StartAgentOutcome` (lines 13-19) with
  `BlockedOnUserAction { blocker: CloudAgentStartupBlocker }`, carrying the structured public
  message and the normalized auth URL produced by `classify_cloud_agent_startup_error`
  (`remote_child.rs:325-333`).
- `PendingStartAgent` (lines 44-49) gains:
  - `request: StartAgentRequest` — the retained request for retries.
  - `attempt: u32` — incremented on each automated retry.
  - `blocked: Option<CloudAgentStartupBlocker>` — the outstanding blocker, if any.
  - `dispatch_generation: u64` / `blocked_generation: u64` — notifier generations (below).
  - The outcome channel becomes a multi-update stream (unbounded channel or re-registered
    oneshot); it stays open across blockers and closes only on `Started`, permanent `Error`,
    attempt timeout, or cancellation.
- `start_agent_error_message_for_status` (lines 297-330) stops treating the child conversation's
  `Blocked` status as terminal for the start path: a `Blocked` child status no longer resolves the
  pending request; the executor instead records the outstanding blocker (the GUI pane still stamps
  the child conversation `Blocked` via `view_impl.rs:300-315`, unchanged). Permanent statuses
  (`Error`, `Cancelled`) still resolve as errors, and `should_cleanup_failed_child_launch`
  (lines 286-295) is unchanged.
- The executor subscribes to `GitHubAuthNotifier` and owns retry:
  - `GitHubAuthNotifier::AuthCompleted` gains a process-wide monotonic `generation: u64`;
    subscribers ignore generations they have already seen (dedupes the multi-path URI handler in
    `app/src/uri/mod.rs:1148-1200`).
  - On a new generation: for every pending entry in the blocked state with
    `blocked_generation < generation`, re-dispatch the retained request (attempt + 1,
    fresh 30-second timer, `dispatch_generation = generation`, clear `blocked`).
  - On blocker arrival: record it and set `blocked_generation` to the last observed generation. If
    any completion with generation > `dispatch_generation` already fired during this attempt
    (callback-before-blocker), retry immediately instead of waiting for the next completion.
  - Retry re-dispatch emits a new `StartAgentExecutorEvent::RetryAgent(StartAgentRequest)` carrying
    the retained child conversation id. Frontends re-drive the spawn into the existing child
    surface: the GUI looks up the child pane (`PaneGroup::child_agent_panes`) and calls
    `AmbientAgentViewModel::spawn_agent_with_request` again; the TUI re-calls
    `AIClient::spawn_agent` for the retained conversation (`crates/warp_tui/src/orchestration_model.rs`).
    Neither path creates a second conversation or pane.
  - The GUI pane's own `handle_github_auth_completed` auto-retry
    (`terminal/view/ambient_agent/model.rs:1438-1448`) is suppressed for child-agent conversations
    so exactly one retry per completion exists.

### RunAgents aggregation (`execute/run_agents.rs`)

- Replace the sequential per-slot await (lines 293-334) with concurrent per-slot update processors
  (`join_all`); each processor loops on its slot's outcome stream until a terminal outcome, tracking
  its own timer and blocker. The final `RunAgentsResult::Launched.agents` is still zipped against
  `agent_run_configs` in input order, preserving today's ordering contract
  (`crates/ai/src/agent/action_result/mod.rs:1364-1382`).
- Slot transitions: `Dispatching → Launched | Failed(permanent) | BlockedOnUserAction`;
  `BlockedOnUserAction → Dispatching` on retry; timeout applies only to an automated attempt.
- Executor-held grouped state, exposed for rendering (snapshot, not events):
  `RunAgentsExecutor::user_action_blocker(action_id) -> Option<RunAgentsUserActionBlocker>` with
  `message: String`, `auth_url: String`, `launched_count: usize`, `blocked_count: usize`, and
  `failures: Vec<(name, error)>` (distinct permanent failures only).
- `RunAgentsExecutorEvent` gains `BlockerChanged { action_id }`. Events only invalidate views; the
  snapshot is authoritative (mirrors how `RunAgentsSpawningSnapshot` already works with
  `SpawningStarted`/`SpawningFinished`).
- `cancel_execution` (lines 108-122) currently cancels only the `Publishing` phase. Extend it to
  every phase: clear retained pending child requests in `StartAgentExecutor`, drop slot processors
  so late updates are ignored, emit `CleanupFailedChildLaunch` for a child conversation whose
  raced spawn completes after cancellation, and let the existing `FinishedAction(Cancelled)` path
  complete the action. Launched siblings are untouched (they are separate conversations).

### GUI ownership (`app/src/ai/blocklist/inline_action/run_agents_card_view.rs`)

- `render` (lines 1224-1316) gains a `BlockedOnUserAction` branch ahead of the `RunningAsync`
  branch: render the grouped remediation card from the executor snapshot — title
  "GitHub Authentication Required", the public message, one "Authenticate with GitHub" button
  opening `auth_url`, and the counts line. No `render_editor`, no run-wide parameter rows, no
  Accept/Reject: after confirmation the card matches the post-confirmation cloud-agent status UI
  (status-only rows as in `render_status_only_card`). All resolved run-wide parameter rendering is
  dropped from this state.
- No focus change, no input locking, no modal auto-pop from this state
  (`maybe_auto_open_create_modal` treats the state as non-interactive).
- The child hidden pane keeps its existing `NeedsGithubAuth` footer and retains its chip.

### TUI ownership (`crates/warp_tui`)

- `tool_call_display_state` (tool_call_labels.rs:122-157): map
  `AIActionStatus::BlockedOnUserAction` to a new `ToolCallDisplayState::BlockedOnUserAction` —
  running glyph ("●") with attention style, never the "■"/"(awaiting approval)" treatment. The
  RunAgents label for this state reads "GitHub authentication required: M of N agents waiting"
  (tool_call_labels.rs:592-625).
- `TuiGenericToolCallView::render` (tui_generic_tool_call_view.rs:353-369): the new state must not
  render a permission card; it renders the fallback tool-call row.
- `TuiOrchestrationBlock` (`orchestration_block.rs:467-476`, `orchestration_block/render.rs:234-241`):
  `is_awaiting_confirmation` stays false; the block renders a non-interactive blocker callout with
  the auth link (reuse the `TuiCloudRunView` blocked-callout pattern from
  `specs/code-1822-tui-cloud-children/TECH.md`), not the configuration pages.
- `agent_block::active_blocking_input_source` (`agent_block.rs:965-1005`): must return `None` when
  the status is `BlockedOnUserAction` — remediation never steals input or replaces the prompt.
- `CliAgentOscEventPublisher` (cli_agent_osc_event_publisher.rs:99-137): publish no
  `permission_request` for this state; `ActionBlockedOnUserConfirmation` remains the only trigger.
  Parent conversation status is unchanged, so the status-line OSC summary keeps reporting "working".

### Failure and compatibility behavior

- `RunAgentsResult` and the proto oneof are unchanged; the server sees the same `Launched` result
  it would see from an uncontested batch. Transcript serialization, the input interceptor, and
  convert-back paths need no changes.
- A child that never unblocks (user abandons OAuth) holds the action open until the parent
  conversation is cancelled; there is no time-based action failure (D3).
- Session restore: in-flight blocked state is lost, matching today's restored-card behavior
  (`run_agents_card_view.rs:1254-1264`); the terminal result renders when present.
- View-only shared sessions: viewers never execute actions; unchanged.
- WASM: the shared changes (all under `app/src/ai/...`; `crates/ai` is untouched) compile for
  `wasm32-unknown-unknown`; TUI-only consumers stay behind the `tui` feature.
- Autonomous mode: preserved failure semantics (Product behavior 9) — no interactive wait.

## Tests

- Executor (`run_agents_tests.rs`, new start-agent tests):
  - Blocker update retains the pending request and keeps the outcome stream open.
  - One completion generation retries every blocked child exactly once; duplicate generations are
    ignored.
  - Callback-before-blocker: a completion that fires between dispatch and the blocker's arrival
    triggers an immediate single retry.
  - Repeated blockers: block → retry → block → retry on the next generation; attempt numbers
    increase monotonically.
  - Attempt timeout: fires 30 seconds into an automated attempt, does not fire while blocked,
    re-arms after retry, and resolves the slot as permanently failed.
  - Cancellation: retained requests cleared, late updates ignored, raced server task cleaned up,
    launched siblings untouched, action completes `Cancelled`.
  - Concurrent updates with input-ordered final result (3 slots resolving out of order).
- Action model (`action_model_tests.rs`): `BlockedOnUserAction` projection while the action remains
  in `running_actions`; no parent `ConversationStatus::Blocked`; no
  `ActionBlockedOnUserConfirmation`; return to `RunningAsync`; final partial-batch result.
- TUI: `tool_call_display_state` mapping; orchestration-block non-interactivity + blocker callout
  (orchestration_block_tests.rs, orchestration_model_tests.rs); no input steal
  (agent_block_tests.rs); no `permission_request` OSC event (cli publisher test).
- GUI: card renders the grouped blocked state with counts and CTA and without the run-wide editor;
  post-retry return to the spawning state; distinct permanent-failure rows.

## Assumptions

- Autonomous (non-interactive) runs resolve a blocked child as a permanent failure with the blocker
  message rather than waiting, mirroring the existing rule that autonomous agents cannot present
  interactive cards (`run_agents.rs:530-544`). No user answer confirmed this; it preserves today's
  no-hang property.
- The Linear issue REMOTE-2616 was not retrievable in this environment; the audit conclusions in the
  brief are treated as the authoritative issue context.
- One grouped card per `run_agents` action is sufficient even when multiple conversations run
  concurrent `run_agents` actions; each action's card is scoped to its own action id.

## Out of scope

- Server, proto, or wire-format changes; the retry is a client-side re-dispatch of the retained
  request.
- Fixing the sequential-slot aggregation beyond what recovery requires (the concurrent-processor
  change is required by the timeout-pause semantics, not a general rewrite).
- Non-GitHub blockers: `CloudAgentStartupBlocker` has one variant today; the design carries the
  enum, so new blockers reuse the state without a new status shape.
- Persisting or restoring the blocked state across app restarts.
- Changes to the GUI pane's own user-initiated cloud-run auth UX (settings-driven flows).

## Validation criteria

- `cargo nextest run -p warp run_agents` — executor and action-model behaviors above pass.
- `cargo nextest run -p warp action_model` — status projection and cancellation tests pass.
- `cargo nextest run -p warp_tui orchestration` — TUI mapping, callout, input, and OSC tests pass.
- `./script/format` and `git diff --check` clean.
- `cargo clippy -p warp --all-targets --tests -- -D warnings` and
  `cargo clippy -p warp_tui --all-targets --tests -- -D warnings` clean.
- `cargo clippy --target wasm32-unknown-unknown --profile release-wasm-debug_assertions --no-deps`
  clean (shared app code).
- `./script/presubmit` before merge.

## Approval gate

Implementation must not start until a reviewer approves:

1. The status shape: a distinct nonterminal `AIActionStatus::BlockedOnUserAction` kept in
   `running_actions` (D1), including the accessor and derivation plan.
2. The partial-batch semantics: blocking only the blocked children while launched, failed,
   launching, and retrying siblings continue (Product behavior 1-6), including the grouped-CTA
   counts contract (D4) and the per-completion retry-once rule (Technical design).
