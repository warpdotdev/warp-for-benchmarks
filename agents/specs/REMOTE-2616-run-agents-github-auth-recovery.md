# Keep `run_agents` Alive During GitHub Authentication

## Summary
Keep an accepted `run_agents` action nonterminal when one or more remote initial children cannot start until the user authenticates with GitHub. The client must retain each blocked child request, show one grouped authentication action, retry every eligible child once for each relevant OAuth completion, and finish the original action only after every child has launched or reached a permanent terminal outcome. This change is client-only.

## Baseline
This specification is based on client commit `aa9f3a4364e66cc49561c69751367240a1035207`.

- `app/src/ai/blocklist/action_model.rs:84` defines `AIActionStatus`. It currently distinguishes queued approval (`Blocked`) from asynchronous execution (`RunningAsync`), but it has no runtime user-remediation state.
- `app/src/ai/blocklist/action_model.rs:615` derives action status from `pending_actions`, `running_actions`, and finished results.
- `app/src/ai/blocklist/action_model.rs:817` projects a pending action that needs confirmation to `ConversationStatus::Blocked`.
- `app/src/ai/blocklist/action_model.rs:879` removes an accepted action from `pending_actions` before execution and adds asynchronous execution to `running_actions`.
- `app/src/ai/blocklist/action_model/execute/run_agents.rs:233` dispatches every child through `StartAgentExecutor`.
- `app/src/ai/blocklist/action_model/execute/run_agents.rs:271` consumes child receivers serially and applies a 30-second timeout while awaiting each final `StartAgentOutcome`.
- `app/src/ai/blocklist/action_model/execute/start_agent.rs:12` exposes only terminal `Started` and `Error` outcomes.
- `app/src/ai/blocklist/action_model/execute/start_agent.rs:289` maps a child `ConversationStatus::Blocked` to an error, and the completion path removes the pending child request.
- `app/src/ai/orchestration/remote_child.rs:123` already classifies GitHub authentication as a structured `CloudAgentStartupBlocker` with a public message and normalized auth URL.
- `app/src/ai/orchestration/remote_child.rs:320` normalizes the server-provided URL through `cloud_setup_auth_url_with_next`.
- `app/src/ai/ambient_agents/github_auth_notifier.rs:12` broadcasts OAuth completion without a generation.
- `app/src/ai/ambient_agents/github_auth_url.rs:47` builds the Cloud Setup callback URL used by native and web clients.
- `app/src/ai/ambient_agents/model.rs:1435` retains and retries an initial Cloud Mode request after OAuth completion. It does not coordinate a `run_agents` batch.
- `crates/warp_tui/src/orchestration_model.rs:538` records a recoverable remote-child blocker, but the retained request is not retried.
- `app/src/ai/blocklist/inline_action/run_agents_card_view.rs:1220` and `crates/warp_tui/src/orchestration_block/render.rs:232` own the GUI and TUI `run_agents` cards.

## Product behavior
1. Accepting `run_agents` remains a single user decision.
2. A GitHub authentication failure after acceptance is runtime remediation. It is not another permission request.
3. The accepted action remains in `running_actions` while any initial child is waiting for GitHub authentication.
4. Runtime remediation does not put the action back in `pending_actions`.
5. Runtime remediation does not re-run preprocessing, approval, request normalization, plan publication, or child fan-out.
6. Runtime remediation does not change the parent `ConversationStatus`.
7. If one or more children are recoverably auth-blocked, the action projects `AIActionStatus::BlockedOnUserAction`.
8. Rule 7 applies to partial batches. Launched, permanently failed, launching, retrying, and auth-blocked children may coexist.
9. The action returns to `AIActionStatus::RunningAsync` only when no recoverable blocker remains and at least one child is still launching or retrying.
10. The action becomes `Finished` only when every child slot is `Launched`, permanently `Failed`, or cancelled by the parent action.
11. A single successful process-wide GitHub OAuth completion retries every eligible auth-blocked initial child in the batch.
12. Each eligible child retries once for that OAuth completion. Duplicate events and stale callbacks do not start an additional attempt.
13. A child that blocks again after a retry waits for a later OAuth completion. The client does not loop automatically on the same completion.
14. Launched siblings continue running when another sibling is auth-blocked, permanently fails, times out, or is cancelled before launch.
15. The card shows one grouped **Authenticate with GitHub** action regardless of the number of blocked children.
16. The grouped card shows aggregate launched and auth-blocked counts. It may also show a single aggregate count for children still starting.
17. The grouped card shows one named row for each permanently failed child. It does not show individual rows for launched, blocked, launching, or retrying children.
18. The grouped card does not render resolved model, harness, execution location, environment, runner, worker host, computer-use, or credential parameters after confirmation.
19. The post-confirmation card follows the existing Cloud Agent status-card visual language: one status header, concise detail, one primary link action, and optional failure rows.
20. The GUI and TUI expose the same status, counts, CTA label, public blocker detail, and permanent-failure rows.
21. A runtime-remediation card does not take confirmation focus. Enter and Ctrl-C must not act as Accept and Reject for the already accepted action.
22. Parent cancellation clears every unresolved retained child request and ends the `run_agents` action as cancelled.
23. Parent cancellation does not cancel children that already launched.
24. A late response after cancellation cannot reopen, retry, or mutate the cancelled action.
25. If a server task is created concurrently with cancellation, the client immediately sends a best-effort cancellation for that raced task.

## Technical design

### 1. Add a distinct action status
Add a payload-free variant:

```rust
pub enum AIActionStatus {
    Preprocessing,
    Queued,
    Blocked,
    RunningAsync,
    BlockedOnUserAction,
    Finished(Arc<AIAgentActionResult>),
}
```

The blocker payload stays in `RunAgentsExecutor`. `AIActionStatus` remains a coarse presentation state.

Update the status helpers with explicit semantics:

- `is_blocked()` remains true only for pending approval.
- Add `is_blocked_on_user_action()`.
- `is_running()` remains true only for `RunningAsync`.
- Add or use an `is_in_flight()` helper when a consumer means “accepted and nonterminal”; it returns true for `RunningAsync` and `BlockedOnUserAction`.
- `is_done()`, `is_success()`, `is_failed()`, and `is_cancelled()` remain terminal-only.

Change `BlocklistAIActionModel::get_action_status` to accept `&AppContext`. When an action ID is in `running_actions`, query the corresponding `RunAgentsExecutor` progress snapshot:

- Return `BlockedOnUserAction` if the snapshot contains at least one recoverable blocker.
- Otherwise return `RunningAsync`.
- Preserve existing behavior for preprocessing, pending, and finished actions.

The executor snapshot is authoritative. Executor events only tell views and the action model to invalidate and re-read the snapshot. Do not copy child progress into a second action-model state map.

### 2. Keep accepted actions in the running phase
Do not call `handle_not_executed_action` for runtime remediation. That path is reserved for an action that has not started because it needs approval.

Do not:

- insert the accepted `AIAgentAction` back into `pending_actions`;
- emit `ActionBlockedOnUserConfirmation`;
- call `try_to_execute_available_actions` for the same action;
- drain or requeue sibling actions;
- update the parent conversation to `ConversationStatus::Blocked`.

The original async execution future remains unresolved until the batch reaches a terminal aggregate result. This keeps phase ordering intact and prevents the controller from sending a premature tool result to the model.

### 3. Make child startup a typed update stream
Replace the terminal-only child receiver contract with a typed stream. The exact type names may follow module conventions, but the variants and data are required:

```rust
pub enum StartAgentUpdate {
    AttemptStarted {
        attempt: u64,
    },
    BlockedOnUserAction {
        attempt: u64,
        blocker: CloudAgentStartupBlocker,
    },
    Started {
        attempt: u64,
        agent_id: String,
    },
    Failed {
        attempt: u64,
        error: String,
    },
}
```

Requirements:

- Attempt numbers start at 1 and increase monotonically per child request.
- The blocker variant carries `CloudAgentStartupBlocker` directly. Do not round-trip the blocker through `ConversationStatus::Blocked` or an untyped error string.
- The blocker message is safe for public rendering.
- The blocker URL is the normalized URL produced by `cloud_setup_auth_url_with_next`.
- `StartAgentExecutor::pending` survives `BlockedOnUserAction`.
- A pending entry closes only on `Started`, permanent `Failed`, per-attempt timeout, or cancellation.
- The initial hidden GUI pane or TUI session remains attached to the same request across retries.
- A retry reuses the original normalized `SpawnAgentRequest`. It does not create a second child conversation or a second child slot.
- Terminal cleanup runs only for a permanent pre-launch failure or cancellation. A blocker never triggers `CleanupFailedChildLaunch`.

The GUI remote-child materializer and `TuiOrchestrationModel` publish these updates through `StartAgentExecutor` with the original `StartAgentRequestId`. History status remains available for child chips and child surfaces, but it is not the batch-control channel.

### 4. Track authoritative per-child progress in `RunAgentsExecutor`
Replace `PendingRunAgents::{Publishing, Spawning}` plus the count-only snapshot with per-action progress that preserves input indices.

Each child slot has exactly one of these states:

- `Preparing`: local validation or request construction has not completed.
- `Launching { attempt }`: automated attempt `attempt` is in flight and its 30-second timer is armed.
- `BlockedOnUserAction { attempt, blocker }`: the timer is paused and the retained request awaits OAuth.
- `Retrying { attempt }`: OAuth was consumed and the next automated attempt is being dispatched.
- `Launched { agent_id }`: terminal success for the slot.
- `Failed { error }`: terminal permanent failure for the slot.
- `Cancelled`: terminal cancellation for an unresolved slot.

Expose a read-only `RunAgentsProgressSnapshot` by action ID. It contains:

- total child count;
- launched count;
- auth-blocked count;
- automated-work count for `Preparing`, `Launching`, and `Retrying`;
- permanent failures in original input order, with child name and public error;
- a single grouped blocker message and primary URL;
- whether the action is terminal.

Select the grouped blocker from the lowest original input index that is currently blocked. Retain every child's blocker internally. If blocked children provide different normalized URLs, keep the selected URL stable until that child leaves the blocked state, log a safe mismatch, and still render only one CTA.

Emit `RunAgentsExecutorEvent::ProgressChanged { action_id }` after each real state transition. The event carries no state snapshot. Keep start and finish events only if existing animation or telemetry needs them.

### 5. Consume children concurrently and order only the result
Replace the serial `for slot in slots` receive loop with concurrent per-slot processing.

- Start every valid child without awaiting a previous child's result.
- Process `StartAgentUpdate` values as soon as they arrive.
- Store terminal outcomes by original input index.
- Build `RunAgentsResult::Launched.agents` by iterating the original `agent_run_configs` and looking up the indexed outcome.
- A slow or blocked lower-index child must not delay progress from a later child.
- Existing request-construction failures become indexed terminal `Failed` slots immediately.

The 30-second timeout is per automated attempt:

- Arm it when `AttemptStarted { attempt }` is accepted for the current attempt.
- Cancel it when that attempt emits `BlockedOnUserAction`, `Started`, or `Failed`.
- Do not count time spent in `BlockedOnUserAction`.
- Re-arm a fresh 30-second timer for the next `AttemptStarted`.
- Ignore a timeout whose action, child, or attempt no longer matches current state.
- A current timeout permanently fails that child, clears its retained request, and does not affect launched siblings.

### 6. Coordinate OAuth with generations and attempts
Change `GitHubAuthNotifier` from an event-only singleton to a monotonic completion source:

- Store `completion_generation: u64`.
- Increment it once when the client handles a GitHub OAuth completion intent.
- Emit `AuthCompleted { generation }`.
- Expose the current generation for callback-before-blocker checks.

Each retained remote initial-child launch stores:

- current attempt number;
- notifier generation captured when the current attempt started;
- latest completion generation consumed for a retry;
- whether the request is launching, blocked, terminal, or cancelled;
- the current server-call task handle;
- the original `SpawnAgentRequest`.

Apply these race rules:

1. **Normal callback:** If generation `G` arrives while the child is blocked, and `G` is newer than both the attempt-start generation and the last consumed generation, consume `G` and start exactly one next attempt.
2. **Callback before blocker:** If the attempt later returns a blocker and the notifier's current generation is newer than the attempt-start generation, consume the current generation and retry once immediately.
3. **Duplicate blocker:** Ignore a second blocker update for the same attempt after the first state transition.
4. **Stale attempt update:** Ignore any blocker, success, failure, or timeout whose attempt does not equal the current attempt.
5. **Repeated blocker:** When retry attempt `N+1` starts, capture the current notifier generation. If it blocks, it waits for a strictly newer generation.
6. **Multiple children:** Every blocked child evaluates the same process-wide generation independently. One generation can therefore retry all eligible children once.
7. **Duplicate callback delivery:** A child consumes a generation at most once. A callback received while its retry is already in flight does not start a parallel attempt.
8. **Cancellation:** Mark the request cancelled and clear the retained request before aborting local work. Subsequent callbacks and updates are ignored.
9. **Cancellation versus server success:** If a cancelled attempt returns a task or run identifier, do not report `Started`; call the existing ambient-task cancellation API best-effort.

Use wrapping-safe comparison or a wider generation type with checked increment. A generation overflow must not make an old completion eligible.

### 7. GUI ownership
`app/src/ai/blocklist/inline_action/run_agents_card_view.rs` owns the parent grouped GUI presentation.

- On `BlockedOnUserAction`, render the post-confirmation remediation card, not the confirmation editor.
- Read the latest executor snapshot during render.
- Show launched and blocked counts in one summary line.
- Show an in-progress count only when nonblocked automated work remains.
- Render one **Authenticate with GitHub** link from the grouped primary URL.
- Render one row per permanent failure with the child name and error.
- Do not render run-wide resolved parameters.
- Do not expose Accept, Reject, configuration pickers, or the accept split menu.
- Do not auto-open auth-secret creation. GitHub OAuth recovery is separate from harness API-key selection.
- A blocked initial child pane may show that startup waits on the parent batch, but it must not add another authentication CTA.

`BlocklistAIActionEvent` must distinguish approval from runtime remediation. Add invalidation events for entering and leaving runtime remediation if the existing executor subscription cannot cover every consumer. Never route them through `ActionBlockedOnUserConfirmation`.

### 8. TUI ownership
`crates/warp_tui/src/orchestration_block.rs` owns the parent grouped TUI presentation.

- Render the same counts, grouped blocker detail, one selectable auth URL, and permanent-failure rows as the GUI.
- Do not call `render_acceptance` or show configuration metadata after the action is accepted.
- Do not advertise Enter-to-accept, Ctrl-E-to-edit, or Ctrl-C-to-reject in remediation state.
- The CTA can receive link focus. The orchestration card itself is not an approval blocker.
- `TuiAIBlock::active_blocking_input_source` continues to recognize only `AIActionStatus::Blocked`.
- A blocked child cloud-run session may show a passive waiting state, but the parent orchestration block owns the only auth CTA.
- TUI cancellation uses the same retained-request cancellation path and race guard as GUI cancellation.

### 9. Consumer audit
Update every exhaustive `AIActionStatus` consumer. Required behavior by category:

- **Run-agents GUI/TUI cards:** render grouped remediation from the executor snapshot.
- **Generic GUI action icons:** use an attention/user-action-required treatment that differs from the approval stop icon and the running spinner.
- **TUI `ToolCallDisplayState`:** add a distinct user-action-required state. Its label must not say “awaiting approval.”
- **Generic tool renderers:** show noninteractive runtime-remediation copy. Never attach permission buttons.
- **Confirmation views for commands, file edits, code search, questions, and review comments:** treat the new variant as non-confirmable and unreachable for their action types. Handle it explicitly without falling through to `Blocked`.
- **Focus and input routing:** only pending-approval `Blocked` replaces input or claims confirmation focus.
- **Cancellation routing:** both `RunningAsync` and `BlockedOnUserAction` cancel through the running executor path; only `Blocked` cancels through the pending-action path.
- **CLI-agent OSC permission events:** `permission_request` and `permission_replied` remain exclusive to pending approval. Runtime remediation must not emit either event. Handle the new internal events/status explicitly so a future wildcard cannot conflate them.
- **TUI exports and test fixtures:** export the new status and typed update types needed by `warp_tui`.

The audit includes the production matches in:

- `app/src/ai/blocklist/action_model.rs`
- `app/src/ai/blocklist/block.rs`
- `app/src/ai/blocklist/block/view_impl/output.rs`
- `app/src/ai/blocklist/block/view_impl/orchestration.rs`
- `app/src/ai/blocklist/inline_action/ask_user_question_view.rs`
- `app/src/ai/blocklist/inline_action/code_diff_view.rs`
- `app/src/ai/blocklist/inline_action/requested_command.rs`
- `app/src/ai/blocklist/inline_action/run_agents_card_view.rs`
- `app/src/ai/blocklist/inline_action/search_codebase.rs`
- `crates/warp_tui/src/agent_block.rs`
- `crates/warp_tui/src/cli_agent_osc_event_publisher.rs`
- `crates/warp_tui/src/orchestration_block.rs`
- `crates/warp_tui/src/orchestration_block/render.rs`
- `crates/warp_tui/src/tool_call_labels.rs`
- `crates/warp_tui/src/tui_ask_question_view.rs`
- `crates/warp_tui/src/tui_file_edits_view.rs`
- `crates/warp_tui/src/tui_generic_tool_call_view.rs`
- `crates/warp_tui/src/tui_review_comments.rs`
- `crates/warp_tui/src/tui_shell_command_view.rs`

Run a fresh repository search for `AIActionStatus::` during implementation. The list above is a baseline, not permission to leave a new exhaustive match unhandled.

## Per-child transitions

- `Preparing -> Launching(1)` after request construction succeeds.
- `Preparing -> Failed` on local validation or request-construction failure.
- `Launching(N) -> BlockedOnUserAction(N)` on the first current recoverable auth blocker.
- `Launching(N) -> Launched` on current server success.
- `Launching(N) -> Failed` on current permanent error or current 30-second timeout.
- `BlockedOnUserAction(N) -> Retrying(N+1)` on one eligible OAuth completion generation.
- `Retrying(N+1) -> Launching(N+1)` immediately before the server call.
- Any unresolved state `-> Cancelled` when the parent action is cancelled.
- Terminal states do not transition.

The aggregate action status is derived after every transition:

- Any blocked slot: `BlockedOnUserAction`.
- No blocked slots and at least one nonterminal automated slot: `RunningAsync`.
- All slots terminal: `Finished`.

## Decisions and alternatives

### Distinct `BlockedOnUserAction` status — selected
- Advantages: keeps approval and runtime remediation separate; preserves running-action identity; gives renderers and input routing an exhaustive compiler-checked branch.
- Disadvantages: requires a broad consumer audit and changes the `get_action_status` call shape.

### Project existing `Blocked` from a running action — rejected
- Advantages: fewer enum changes and existing attention styling.
- Disadvantages: `Blocked` currently means “front pending action awaiting approval.” It drives confirmation focus, permission copy, pending-action cancellation, and `ConversationStatus::Blocked`. Reusing it would create a second meaning and make partial batches look unaccepted.

### Move the action back to the pending queue — rejected
- Advantages: reuses pending-action UI and execution entry points.
- Disadvantages: can request approval twice, re-run plan publication and fan-out, duplicate already launched children, reorder sibling actions, send duplicate telemetry, and change the parent conversation status. It cannot represent a partially launched batch safely.

### Keep `RunningAsync` and put the blocker only in card state — rejected
- Advantages: smallest status-model change.
- Disadvantages: generic renderers, cancellation, focus, accessibility, and CLI consumers cannot distinguish automated progress from required user remediation.

### One CTA per blocked child — rejected
- Advantages: preserves the URL closest to each child.
- Disadvantages: all initial children use the same process-wide GitHub completion signal. Repeated CTAs imply separate authentication work and scale poorly.

### One grouped CTA with per-child internal state — selected
- Advantages: matches the OAuth scope, supports partial batches, and keeps retry bookkeeping independent.
- Disadvantages: the parent card must select one stable URL and aggregate child state.

### Serial child update consumption — rejected
- Advantages: naturally preserves result order.
- Disadvantages: hides later child progress and can multiply the 30-second timeout by the number of earlier children.

### Concurrent consumption with indexed final assembly — selected
- Advantages: immediate progress, per-attempt timeouts, and deterministic output order.
- Disadvantages: requires explicit slot indices and stale-update guards.

## Failure and compatibility behavior
- Permanent capacity, credits, overload, transport, deserialization, request-construction, and unsupported-mode failures keep their current public messages and become terminal child failures.
- Only a classified `CloudAgentStartupBlocker::GitHubAuthRequired` enters remediation.
- An absent or invalid auth URL remains a permanent failure. Do not render a disabled or malformed CTA.
- Existing local children do not subscribe to GitHub OAuth and do not gain retry behavior.
- Existing standalone Cloud Mode initial-run OAuth retry remains functional. It may reuse the notifier generation API, but it must not be coupled to a `run_agents` action.
- Follow-up Cloud Mode requests remain outside this change.
- No GraphQL schema, protobuf, server endpoint, persisted conversation format, or action-result format changes.
- `RunAgentsResult` remains terminal and wire-compatible. Blocked and retrying states are client-local and are never serialized as final child outcomes.
- Restored transcripts cannot restore an in-memory retained startup request. If the client exits during authentication, the restored action follows existing incomplete-action cancellation behavior instead of silently relaunching children.
- Telemetry must not include auth URLs, OAuth query parameters, credential names, or tokens. Counts, attempt number, and coarse transition reason are permitted.

## Assumptions
- GitHub OAuth completion is process-wide for the signed-in Warp account.
- All GitHub auth blockers in one batch are eligible for the same completion event even when their normalized URLs differ.
- The server accepts replaying the same initial `SpawnAgentRequest` after successful GitHub authentication.
- A successful spawn response includes the task identifier needed for best-effort cancellation if cancellation wins the client-side race.
- Visual computer-use proof is not required for this spec approval. GUI and TUI rendering tests are required.

## Out of scope
- Server-side resumable launch tokens.
- Retrying non-auth startup failures.
- Retrying already launched children.
- Persisting blocked startup requests across client restart.
- Changing parent `ConversationStatus` or child run lifecycle semantics after launch.
- Changing the run-wide confirmation editor.
- Rendering resolved run-wide parameters after confirmation.
- Adding OAuth support for providers other than GitHub.

## Testing and validation

### Action model and executor tests
Add or extend tests under `app/src/ai/blocklist/action_model/execute/` and `app/src/ai/blocklist/action_model_tests.rs`:

- an accepted blocked batch stays in `running_actions`, is absent from `pending_actions`, and projects `BlockedOnUserAction`;
- the parent `ConversationStatus` is unchanged on blocker and retry;
- partial launched + blocked, failed + blocked, launching + blocked, and retrying + blocked batches all project `BlockedOnUserAction`;
- removing the last blocker with automated work remaining projects `RunningAsync`;
- final result order matches input order even when child updates arrive out of order;
- a later child can launch while an earlier child is blocked;
- each automated attempt gets an independent 30-second timeout;
- blocked wait time does not consume timeout budget;
- timeout from an old attempt is ignored;
- request-construction failures coexist with launched and blocked slots;
- terminal result contains launched and permanent failures only after every slot is terminal.

### Start-agent and OAuth race tests
Add deterministic tests around `StartAgentExecutor`, `GitHubAuthNotifier`, GUI remote launch ownership, and `TuiOrchestrationModel`:

- blocker emits the typed message and normalized URL and keeps the pending request open;
- one completion retries all eligible blocked children once;
- callback-before-blocker retries once;
- duplicate completion for the same generation does not retry twice;
- duplicate blocker for the same attempt does not retry twice;
- a repeated blocker waits for a newer generation;
- success, failure, and timeout from a stale attempt are ignored;
- cancellation before callback clears the retained request;
- callback after cancellation does nothing;
- cancellation racing a successful spawn sends best-effort task cancellation and does not report `Started`;
- launched siblings are not cancelled with unresolved siblings;
- permanent failure closes and cleans up the retained pre-launch child surface;
- local and standalone Cloud Mode behavior remains unchanged.

### GUI rendering tests
Extend `app/src/ai/blocklist/inline_action/run_agents_card_view_tests.rs`:

- one and many blocked children render one CTA;
- the card renders launched and blocked counts for a partial batch;
- only permanent failures render named rows;
- run-wide model, harness, location, environment, runner, host, and computer-use values are absent after confirmation;
- runtime remediation has no Accept, Reject, edit, or permission controls;
- runtime remediation does not claim approval focus;
- the generic action icon and fallback renderer distinguish user action from approval and running.

### TUI rendering and routing tests
Extend:

- `crates/warp_tui/src/orchestration_block_tests.rs`
- `crates/warp_tui/src/orchestration_model_tests.rs`
- `crates/warp_tui/src/tool_call_labels_tests.rs`
- `crates/warp_tui/src/agent_block_tests.rs`
- `crates/warp_tui/src/cli_agent_osc_event_publisher_tests.rs`

Verify:

- GUI-equivalent grouped counts, CTA, and failure rows;
- no resolved run-wide metadata after confirmation;
- no approval key hints or blocking input source;
- distinct generic label and glyph without “awaiting approval”;
- Ctrl-C uses running cancellation, not pending rejection;
- no `permission_request` or `permission_replied` OSC event for runtime remediation;
- TUI OAuth retry and cancellation races match GUI behavior.

### Commands
Run focused tests first:

```bash
cargo test -p warp run_agents
cargo test -p warp github_auth
cargo test -p warp_tui orchestration
cargo test -p warp_tui tool_call
cargo test -p warp_tui cli_agent_osc
```

Then run repository-required formatting and presubmit checks for all touched Rust files.

## Approval gate
Implementation must not begin until reviewers approve both of these decisions:

1. `AIActionStatus::BlockedOnUserAction` is a distinct nonterminal status for an accepted action. It does not reuse pending-approval `Blocked`, move the action to `pending_actions`, or change parent `ConversationStatus`.
2. Any recoverably auth-blocked child makes the whole accepted batch project `BlockedOnUserAction`, including partial batches with launched, failed, launching, or retrying siblings. One process-wide OAuth completion retries every eligible blocked initial child once while launched siblings continue running.
