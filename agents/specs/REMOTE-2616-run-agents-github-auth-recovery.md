# Keep accepted `run_agents` alive during remote GitHub auth recovery (REMOTE-2616)

## Summary
After the user accepts a `run_agents` action, one or more remote children can require GitHub authentication before they start. Today that recoverable pause is treated as a launch failure: `StartAgentExecutor` completes the pending child as `Error` when the child conversation becomes `ConversationStatus::Blocked`, or the 30-second spawn timer fires while the child is waiting on OAuth. The accepted parent action then finishes, the confirmation card is gone, and authenticating GitHub cannot resume the original batch. This change keeps the accepted action in `running_actions`, introduces a distinct nonterminal `AIActionStatus::BlockedOnUserAction`, retries every eligible blocked initial child from one process-wide OAuth completion, and returns a single `RunAgentsResult` only after every child has started, failed permanently, timed out, or been cancelled. The change is client-only.

## Product behavior
1. After the user accepts `run_agents`, the card never returns to the pre-accept confirmation editor. Resolved run-wide parameter pickers (model, harness, environment, runner, API key) are not shown again for that action.
2. While any remote child in that accepted batch is recoverably blocked on GitHub authentication, the parent action stays alive and the card shows a grouped GitHub-auth remediation surface. The parent conversation stays `ConversationStatus::InProgress`.
3. The grouped surface uses the existing cloud-agent GitHub-auth copy: title `GitHub Authentication Required`, detail `Please authenticate with GitHub to continue`, CTA label `Authenticate with GitHub`. The CTA opens the normalized `cloud_setup` auth URL. There is one CTA for the whole batch, not one per blocked child.
4. The grouped surface reports launched and blocked counts. It shows individual child rows only for distinct permanent failures. Launching, retrying, and recoverably blocked children are not listed as separate failure rows.
5. A mixed batch is still recoverably blocked when at least one child is waiting on GitHub auth, even if siblings have already launched, failed permanently, are still launching, or are retrying.
6. Completing GitHub OAuth once in the process retries every eligible blocked initial child from that batch (and from any other in-flight accepted `run_agents` batch in the same process). The user does not re-accept the card and does not re-run the tool call.
7. After OAuth, the card returns to the ordinary spawning surface only when no recoverably blocked child remains and automated spawn work is still in progress. When every child has a terminal launch outcome, the action finishes with the existing `RunAgentsResult::Launched` mixed-success contract.
8. Canceling the parent `run_agents` action while it is waiting on GitHub auth cancels unresolved retained child requests, ignores late start-agent updates, and cancels a raced server spawn that has not launched. Children that already launched keep running.
9. Runtime GitHub-auth remediation is not permission approval. Enter/Esc, yes/no permission prompts, confirmation-card focus, CLI `permission_request` events, and `is_blocked()` consumers continue to mean pending approval only.
10. Local children are unchanged. Permanent remote startup failures (capacity, credits, overload, other) remain terminal per child and do not enter the GitHub-auth wait.

## Technical design
### How the area works today
- `AIActionStatus` has five variants (`app/src/ai/blocklist/action_model.rs:74-93`). `Blocked` is projected only for the front pending action when the conversation has no `running_actions` (`get_action_status` at `action_model.rs:615-647`). Any action in `running_actions` is always `RunningAsync`.
- Accepting `run_agents` removes the action from `pending_actions` and puts it in `running_actions` (`start_pending_action_by_id` at `action_model.rs:859-897`). Parent status is set to `ConversationStatus::InProgress` (`action_model.rs:794-806`). `handle_not_executed_action` is the only path that sets parent `ConversationStatus::Blocked` (`action_model.rs:809-831`), and it is pending-approval only.
- `RunAgentsExecutor` fans out through `StartAgentExecutor::dispatch` and waits sequentially on a one-shot `StartAgentOutcome` with `SPAWN_TIMEOUT` of 30 seconds (`run_agents.rs:44-47`, `293-377`). The wait treats `StartAgentOutcome::Error` and timeout as permanent per-child failures, then sends one `RunAgentsResult` and leaves `running_actions`.
- `StartAgentOutcome` is only `Started` or `Error` (`start_agent.rs:13-19`). `start_agent_error_message_for_status` maps child `ConversationStatus::Blocked` to a terminal error string (`start_agent.rs:297-318`). That mapping is the TUI failure: `TuiOrchestrationModel::finish_remote_child_launch` writes `ConversationStatus::Blocked` for `CloudAgentStartupIssue::Blocked` (`orchestration_model.rs:529-543`), and the pending start request is then completed as `Error`.
- The GUI remote path creates a hidden ambient pane and calls `spawn_agent_with_request` (`terminal_pane.rs:1876-1968`). GitHub auth is classified by `classify_cloud_agent_startup_error` (`remote_child.rs:325-332`) and stored as `AmbientAgentViewModel::Status::NeedsGithubAuth` (`ambient_agent/model.rs:1394-1436`). The pending start request is not completed from that status. The 30-second `run_agents` timer still runs, so the parent action fails while the child pane is waiting on OAuth. Comment at `remote_child.rs:129-133` records the current contract: the child surface is retained, but the original launch still resolves as failed.
- `GitHubAuthNotifier` emits a payload-less `AuthCompleted` (`github_auth_notifier.rs:11-33`). `AmbientAgentViewModel` retries only if it is already `NeedsGithubAuth` and still holds `request` (`ambient_agent/model.rs:1438-1448`). A completion that arrives before the blocker is dropped. Duplicate completions retry again. TUI children do not use this path and currently tell the user to rerun orchestration (`cloud_run_view.rs:234-247` uses `CloudAgentStartupAuthFlow::RerunOrchestrationRequest`).
- Status consumers treat `AIActionStatus::Blocked` as pending approval: GUI card render (`run_agents_card_view.rs:1266-1285`), TUI `is_awaiting_confirmation` (`orchestration_block.rs:466-475`), TUI `active_blocking_input_source` (`agent_block.rs:960-980`), generic/file/shell permission prompts (`tui_generic_tool_call_view.rs:105-111`, `tui_permission_prompt.rs:167-173`), CLI OSC `permission_request` (`cli_agent_osc_event_publisher.rs:100-135`), GUI `is_blocked_on_user_confirmation` (`block.rs:4565-4571`), and post-finish focus steal (`block.rs:4800-4804`).

### Proposed changes
Keep the accepted action in `running_actions` for the whole recovery. Do not requeue it, do not drain it back onto `pending_actions`, and do not change parent `ConversationStatus` away from `InProgress` until the action finishes.

**1. Add `AIActionStatus::BlockedOnUserAction`.**

In `app/src/ai/blocklist/action_model.rs`:

```rust
pub enum AIActionStatus {
    Preprocessing,
    Queued,
    Blocked,
    /// Accepted action is running, but at least one child needs user
    /// remediation (GitHub auth) before automated work can finish.
    BlockedOnUserAction,
    RunningAsync,
    Finished(Arc<AIAgentActionResult>),
}
```

- `is_blocked()` stays `matches!(self, AIActionStatus::Blocked)` only.
- Add `is_blocked_on_user_action()` for the new variant.
- `is_running()` is true for both `RunningAsync` and `BlockedOnUserAction` (the action is still in `running_actions`).
- `get_action_status` for an id in `running_actions` reads the executor progress snapshot. If any child is recoverably auth-blocked, return `BlockedOnUserAction`. Otherwise return `RunningAsync`. Finished, pending, and preprocessing projection is unchanged.

**2. Project `BlockedOnUserAction` from the executor snapshot, not from events.**

Extend `RunAgentsSpawningSnapshot` so it is the authoritative in-flight picture:

```rust
pub struct RunAgentsSpawningSnapshot {
    pub agent_count: usize,
    pub children: Vec<RunAgentsChildProgress>, // input order of agent_run_configs
}

pub enum RunAgentsChildProgress {
    Launching { attempt: u32 },
    BlockedOnGitHubAuth { message: String, auth_url: String, attempt: u32 },
    Retrying { attempt: u32 },
    Launched { agent_id: String },
    Failed { error: String },
    Cancelled,
}
```

`RunAgentsExecutorEvent::SpawningStarted` still carries the snapshot. Additional executor events only invalidate views; they do not own status. The GUI/TUI card re-reads the snapshot on invalidation.

Projection rule:

- If any child is `BlockedOnGitHubAuth` → parent action is `BlockedOnUserAction`.
- Else if any child is `Launching` or `Retrying` → `RunningAsync`.
- Else every child is `Launched`, `Failed`, or `Cancelled` → finish the action with `RunAgentsResult` in `agent_run_configs` order.

This rule includes partial batches.

**3. Typed start-agent update stream.**

Replace the one-shot `Receiver<StartAgentOutcome>` with a multi-message stream. Pending child requests stay in `StartAgentExecutor::pending` across a recoverable blocker.

```rust
pub enum StartAgentUpdate {
    BlockedOnUserAction { blocker: CloudAgentStartupBlocker },
    Started { agent_id: String },
    Failed { error: String },
}
```

Close a pending request only on `Started`, permanent `Failed`, spawn-attempt timeout, or parent cancellation. Do not close it on `ConversationStatus::Blocked` or on `CloudAgentStartupIssue::Blocked`.

Change `start_agent_error_message_for_status` so `ConversationStatus::Blocked` is not a terminal start error. When the GUI ambient model or TUI launch path observes `CloudAgentStartupIssue::Blocked`, send `StartAgentUpdate::BlockedOnUserAction` carrying the already-normalized URL from `classify_cloud_agent_startup_error` (`github_auth_url::cloud_setup_auth_url_with_next`). Do not encode the URL in `ConversationStatus::Blocked.blocked_action`.

Extend `PendingStartAgent` with the retained launch identity needed to retry without creating a second conversation or pane: `child_conversation_id`, original `StartAgentRequest` fields, current attempt number, optional current blocker, and the completion generation that was current when this attempt blocked (see below). `should_cleanup_failed_child_launch` already keeps `ConversationStatus::Blocked` children; leave that.

Add `StartAgentExecutorEvent::RetrySpawn { request_id: StartAgentRequestId }`. GUI `launch_remote_child` and TUI `register_remote_child_session` handle it by reusing the existing child conversation/pane/session and calling spawn again with the retained `SpawnAgentRequest`. They must not insert a new pane or call `start_new_child_conversation` again.

**4. Concurrent child updates and timeout.**

In `RunAgentsExecutor::dispatch_children_for_prepared_request`, wait on all pending child streams concurrently. Do not iterate slots sequentially. Update the snapshot as each update arrives. Preserve `agent_run_configs` order only when building the final `RunAgentsResult`.

Timeout:

- 30 seconds per automated attempt (`SPAWN_TIMEOUT` stays 30 seconds).
- Pause the attempt timer when that child enters `BlockedOnGitHubAuth`.
- Re-arm a fresh 30-second timer when that child starts a retry attempt.
- A timeout during an automated attempt is a permanent per-child `Failed` for that child. It does not fail siblings and does not finish the parent action if other children are still launching, retrying, or auth-blocked.

**5. Process-wide OAuth completion with generations and attempt numbers.**

Change `GitHubAuthNotifier`:

```rust
pub enum GitHubAuthEvent {
    AuthCompleted { generation: u64 },
}

impl GitHubAuthNotifier {
    pub fn notify_auth_completed(&mut self, ctx: &mut ModelContext<Self>) {
        self.generation = self.generation.wrapping_add(1);
        ctx.emit(GitHubAuthEvent::AuthCompleted { generation: self.generation });
    }
    pub fn current_generation(&self) -> u64 { self.generation }
}
```

`StartAgentExecutor` subscribes to the notifier. A child is eligible to retry when all of these are true:

- it is an in-flight initial remote start (not a follow-up);
- it currently has a GitHub-auth blocker;
- it has not been cancelled;
- `completion_generation > blocked_at_generation`;
- it has not already consumed this `completion_generation`.

Retry exactly once per (child, completion generation). Increment `attempt` on retry. If a completion generation is already newer than `blocked_at_generation` at the moment the blocker is recorded (callback-before-blocker), retry immediately without waiting for another OAuth. Ignore duplicate deliveries of the same generation. A later blocker after a retry stores the new `blocked_at_generation` and waits for a newer generation.

For orchestrated remote children, `StartAgentExecutor` owns retry. `AmbientAgentViewModel::handle_github_auth_completed` must not retry when `conversation_id` is an orchestrated remote child; otherwise GUI children double-spawn. Standalone cloud-mode panes keep the ambient retry path, but that path must also honor generations so callback-before-blocker and duplicates are fixed there too.

**6. Grouped card: GUI and TUI.**

GUI `RunAgentsCardView::render` (`run_agents_card_view.rs:1224-1316`):

- `Finished` → existing terminal card.
- `BlockedOnUserAction` → grouped GitHub-auth card built from `CloudAgentStartupPresentation::github_auth(url, CloudAgentStartupAuthFlow::RetryRetainedRequest)` plus launched/blocked counts from the snapshot. Permanent-failure rows only. No picker editor.
- `RunningAsync` or `spawning` snapshot with no blocker → existing `render_spawning_card`.
- `Blocked` (pending approval only) → existing confirmation editor.
- Do not treat `BlockedOnUserAction` as `Blocked`.

TUI `TuiOrchestrationBlock`:

- `is_awaiting_confirmation` remains `AIActionStatus::Blocked` only.
- Render `BlockedOnUserAction` as the grouped auth surface (same presentation helper). Switch the TUI cloud-child auth flow from `RerunOrchestrationRequest` to `RetryRetainedRequest` for this retained-launch path (`cloud_run_view.rs:234-247`).
- Enter/Esc on the orchestration block must not accept or reject the already-accepted action while it is `BlockedOnUserAction`. Opening the auth URL is the only remediation input.

**7. Consumer audit: runtime remediation vs approval.**

Every consumer that currently matches `AIActionStatus::Blocked` or `is_blocked()` must be updated so `BlockedOnUserAction` does not take the approval path. Required sites:

- `AIActionStatus::is_blocked` / new `is_blocked_on_user_action` (`action_model.rs:105-115`).
- `get_action_status` (`action_model.rs:615-647`).
- GUI `RunAgentsCardView::render` and `maybe_auto_open_create_modal` (`run_agents_card_view.rs:781-800`, `1224-1285`).
- GUI `AIBlock::is_blocked_on_user_confirmation` (`block.rs:4565-4571`).
- GUI confirmation focus: `handle_ask_user_question_stream_update` (`block.rs:3763-3770`) and post-finish `try_steal_focus` (`block.rs:4800-4804`).
- TUI `TuiOrchestrationBlock::is_awaiting_confirmation` (`orchestration_block.rs:466-475`).
- TUI `TuiAIBlock::active_blocking_input_source` (`agent_block.rs:960-980`).
- TUI `TuiGenericToolCallView::is_blocked`, `TuiPermissionPrompt::is_active`, file-edits and shell-command equivalents (`tui_generic_tool_call_view.rs:105-111`, `tui_permission_prompt.rs:167-173`).
- TUI `tool_call_display_state` / `tool_call_label` (`tool_call_labels.rs:122-196`): `Blocked` keeps `(awaiting approval)`. `BlockedOnUserAction` is not approval; label it as a running wait (for `run_agents`, the orchestration block owns the grouped copy).
- CLI OSC `ActionBlockedOnUserConfirmation` / `permission_request` (`cli_agent_osc_event_publisher.rs:100-135`). Do not emit `permission_request` for `BlockedOnUserAction`. A new non-permission event is allowed but not required; omitting the approval event is required.
- Cancellation routing: `cancel_action_with_id` on a `BlockedOnUserAction` action must use the running-action path, not the pending-queue path (`action_model.rs:1076-1088`). Extend `RunAgentsExecutor::cancel_execution` so it also cancels `PendingRunAgents::Spawning`, asks `StartAgentExecutor` to drop unresolved retained requests, ignores late updates, and cancels a raced server spawn that has not produced a run id. Already launched siblings stay running. `cancel_all_pending_actions` already cancels running async actions; keep that.

Exhaustive matches on `AIActionStatus` in GUI, TUI, and tests must add the new variant. A new variant that falls through to the `Blocked` arm is a spec violation.

**8. Failure and compatibility.**

- Permanent per-child failures stay `RunAgentsAgentOutcomeKind::Failed { error }` and use `CloudAgentStartupPresentation::failure`.
- If every child fails permanently, keep today's terminal card: `Failed to spawn agent` / `Failed to spawn N agents` (`run_agents_card_view.rs:1604-1633`).
- Mixed launched + failed, with no remaining blocker, finishes immediately as `Spawned X of Y agents`.
- Restored-from-history cards stay cancelled (`run_agents_card_view.rs:1254-1263`). In-flight recovery state is not persisted across app restart.
- WASM local children remain unsupported. Remote GitHub-auth recovery applies to remote children on every client that can spawn them.
- No proto, server, or `ConversationStatus` variant is added.

## Decisions
### Status shape for an accepted action waiting on GitHub auth
- **Option A — new `AIActionStatus::BlockedOnUserAction`, action stays in `running_actions` (chosen).** Approval and runtime remediation stay separate. `is_blocked()`, confirmation focus, Enter/Esc, CLI `permission_request`, and pending-queue drain keep their current meaning. The executor snapshot can project the wait without re-queuing. Parent `ConversationStatus` stays `InProgress`.
- **Option B — project existing `Blocked` from a running action.** Smaller enum change. Every `is_blocked()` consumer becomes an approval-vs-remediation bug unless audited perfectly. `get_action_status` today cannot return `Blocked` for a running action (`action_model.rs:623-637`). GUI/TUI cards would reopen the confirmation editor (`run_agents_card_view.rs:1269-1285`, `orchestration_block.rs:472-475`). Parent status would tend to flip to `ConversationStatus::Blocked` via the approval path (`action_model.rs:820-828`), which this spec forbids.
- **Option C — move the action back onto `pending_actions`.** Reuses the pending-approval pipeline. It re-drains, re-enables Accept, and races with `execute_run_agents` which only mutates pending actions (`action_model.rs:685-697`). Launched siblings would sit behind a new confirmation. Rejected because it undoes the user's accept.

### Partial-batch projection
- **Option A — `BlockedOnUserAction` whenever any child is recoverably auth-blocked (chosen).** The user still has work to do. Launched siblings stay running. Permanent failures stay visible as rows. The card does not look finished.
- **Option B — stay `RunningAsync` until every child is blocked or terminal.** Hides the CTA while any sibling is still spawning. The user cannot authenticate until the last inflight spawn settles. Rejected because OAuth should start as soon as the first recoverable blocker is known.
- **Option C — finish the action for launched/failed children and leave blocked children as a new pending action.** Splits one accepted batch into two tool results. The model would see a completed `run_agents` while children are still starting. Rejected.

### Who retries after OAuth
- **Option A — `StartAgentExecutor` owns orchestrated-child retry; ambient notifier retry is disabled for those children (chosen).** One generation-gated path covers GUI and TUI. Prevents double-spawn with `AmbientAgentViewModel::handle_github_auth_completed`.
- **Option B — each frontend retries from `GitHubAuthNotifier` independently.** GUI already does this for standalone cloud panes, but TUI does not, and callback-before-blocker is dropped. Rejected as the orchestrated-child owner.
- **Option C — require the user to rerun `run_agents`.** Matches current TUI copy (`RerunOrchestrationRequest`). Rejected; the accepted action must stay alive.

### Card chrome after accept
- **Option A — drop resolved run-wide parameter rendering; match post-confirmation cloud-agent UI (chosen).** After accept, the card is a status surface plus one grouped CTA. Reuses `CloudAgentStartupPresentation`.
- **Option B — keep the confirmation editor and overlay an auth banner.** Leaves Accept enabled and shows stale pickers. Rejected.

## Assumptions
- GitHub authentication is the only recoverable runtime blocker in this change. `CloudAgentStartupBlocker` currently has one arm, `GitHubAuthRequired` (`remote_child.rs:135-137`). A future blocker kind would reuse `BlockedOnUserAction` but is out of scope.
- "Eligible blocked initial child" means a child from an accepted `run_agents` (or `start_agent`) initial remote launch that is currently `BlockedOnGitHubAuth` and not cancelled. Cloud follow-ups keep today's ambient behavior: they drop `request` unless `SessionStartupKind::InitialRun` (`ambient_agent/model.rs:1424-1426`).
- The public CTA copy is the existing `RetryRetainedRequest` strings, not the raw server error in `ClientError.error`. The structured server message still travels on the update stream for logs and tests.
- Visual proof with the computer-use tool is not required. The request did not opt in.
- App restart during an in-flight GitHub-auth wait does not resume the wait. Restored cards stay cancelled, as they do today.

## Out of scope
- Server, proto, and `ConversationStatus` schema changes. Child conversations may still show `ConversationStatus::Blocked` locally for iconography; the parent conversation must not.
- Local child GitHub auth. Local children do not use `classify_cloud_agent_startup_error`.
- Changing permanent failure classification (capacity, credits, overload, unresolved skills, missing parent `run_id`).
- Persisting in-flight recovery across app restart.
- Multi-level children presenting their own confirmation cards. Child conversations still auto-execute `run_agents` (`run_agents.rs:439-450`).
- Replacing the standalone cloud-mode GitHub-auth screen, except the generation/attempt race fix needed so orchestrated children and standalone panes share a safe notifier.

## Validation criteria
Each criterion names how it is checked. No computer-use capture is required.

1. **Status shape — `get_action_status` never returns `Blocked` for a running accepted `run_agents` action.** Unit test on `BlocklistAIActionModel::get_action_status`: put the action in `running_actions`, mark one child `BlockedOnGitHubAuth`, assert `BlockedOnUserAction`. Same action on `pending_actions` with empty `running_actions` still returns `Blocked`. Checked by: new test in `app/src/ai/blocklist/action_model` tests (separate `*_tests.rs` file).
2. **Partial batch — any recoverable blocker projects `BlockedOnUserAction`.** Snapshot with one `Launched`, one `Failed`, one `Launching` or `Retrying`, and one `BlockedOnGitHubAuth` asserts parent `BlockedOnUserAction` and that the action is still in `running_actions`. Removing the blocker while `Launching`/`Retrying` remains asserts `RunningAsync`. When only `Launched`/`Failed` remain, the action finishes and `get_action_status` is `Finished`. Checked by: `run_agents` executor tests in `app/src/ai/blocklist/action_model/execute/run_agents_tests.rs`.
3. **Start-agent stream — pending request survives a blocker.** Drive `StartAgentExecutor` with a child `CloudAgentStartupIssue::Blocked`. Assert the pending map still contains the request, the receiver got `StartAgentUpdate::BlockedOnUserAction` with public message plus normalized `cloud_setup` URL, and no `Failed`/`Started` was sent. Then complete start and assert `Started` closes the pending entry. Checked by: new `start_agent` tests (separate `*_tests.rs`).
4. **`ConversationStatus::Blocked` is not a terminal start error.** `start_agent_error_message_for_status` returns `None` for `ConversationStatus::Blocked`. A TUI `finish_remote_child_launch` blocker must not complete the pending request as `Error`. Checked by: unit test next to `start_agent.rs` helpers and a TUI orchestration-model test around `finish_remote_child_launch`.
5. **OAuth races retry exactly once when appropriate.** Table-driven tests on generation + attempt:
   - callback-before-blocker: completion generation N, then blocker with `blocked_at_generation < N` → one immediate retry, attempt becomes 2;
   - duplicate `AuthCompleted { generation: N }` → still one retry;
   - second blocker after retry waits for generation N+1;
   - timeout during `BlockedOnGitHubAuth` does not fire; timeout after retry re-arm does;
   - cancel after blocker, then late `AuthCompleted` and late `Started` are ignored;
   - cancel of a raced server spawn that has no run id does not kill already-launched siblings.
   Checked by: `start_agent` / `run_agents` executor tests plus a `GitHubAuthNotifier` generation test.
6. **Concurrent updates preserve input order only in the final result.** Three children complete out of order (fail, auth-block then start, start). Final `RunAgentsResult.agents` matches `agent_run_configs` order. Checked by: `run_agents_tests.rs`.
7. **Grouped card, GUI.** For `BlockedOnUserAction`, `RunAgentsCardView` does not render the confirmation editor or pickers. It shows one Authenticate CTA, launched and blocked counts, and permanent-failure rows only. `RunningAsync` still uses `render_spawning_card`. `Blocked` still uses the confirmation editor. Checked by: `app/src/ai/blocklist/inline_action/run_agents_card_view_tests.rs`.
8. **Grouped card, TUI.** `is_awaiting_confirmation` is false for `BlockedOnUserAction`. `active_blocking_input_source` does not return the orchestration block as a permission prompt. The auth surface uses `RetryRetainedRequest` copy. Checked by: `crates/warp_tui/src/orchestration_block_tests.rs` and `cloud_run_view_tests.rs`.
9. **Consumer audit.** Tests or exhaustive matches prove `is_blocked()` is false for `BlockedOnUserAction` at: `TuiPermissionPrompt::is_active`, `TuiGenericToolCallView::is_blocked`, `AIBlock::is_blocked_on_user_confirmation`, `tool_call_label` (no `(awaiting approval)` suffix), and CLI OSC (no `permission_request`). Checked by: existing label/OSC tests extended in `tool_call_labels_tests.rs` and `cli_agent_osc_event_publisher_tests.rs`, plus GUI block tests if present.
10. **Cancellation routing.** Cancelling a `BlockedOnUserAction` `run_agents` action yields `RunAgentsResult::Cancelled` (or per-child cancelled outcomes for unresolved requests), does not requeue the action, leaves launched siblings running, and does not set parent `ConversationStatus::Blocked`. Checked by: `run_agents_tests.rs` and an action-model cancel test.
11. **Compatibility / no collateral.** Existing `run_agents` tests keep passing: duplicate-launch denial, autoexecute, empty configs, OpenCode+remote rejection, sequential local dispatch. `./script/presubmit` from the `warp` repo root is green before merge. New tests live in `*_tests.rs` files (`./script/check_no_inline_test_modules`). Checked by: `./script/format --check`, `./script/check_no_inline_test_modules`, clippy as in repo docs, and `cargo nextest run -p warp --no-fail-fast` covering `action_model`, `run_agents`, `start_agent`, `remote_child`, `run_agents_card_view`, and `warp_tui` orchestration/cloud-run/label/OSC tests.

## Approval gate
Reviewers must explicitly approve:

1. **Status shape:** new nonterminal `AIActionStatus::BlockedOnUserAction`, action remains in `running_actions`, `is_blocked()` stays pending-approval only, parent `ConversationStatus` stays `InProgress`.
2. **Partial-batch semantics:** project `BlockedOnUserAction` whenever any child is recoverably auth-blocked, including mixed launched / failed / launching / retrying siblings; return to `RunningAsync` only when no blocker remains and automated work continues.

Do not implement until both are approved.
