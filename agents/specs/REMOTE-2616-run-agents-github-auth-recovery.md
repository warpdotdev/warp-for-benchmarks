# REMOTE-2616: Keep `run_agents` alive during GitHub authentication

## Summary
Keep an accepted `run_agents` action alive when one or more remote children cannot start until the user authenticates with GitHub. The parent action remains an executing action. The client presents one grouped authentication action, retries every eligible blocked initial child after one process-wide OAuth completion, and returns the final `RunAgentsResult` only after every child reaches a terminal launch outcome.

This change is client-only. It does not change the server, public API, protocol buffers, or parent `ConversationStatus`.

## Product behavior
1. After the user accepts a `run_agents` request, a recoverable GitHub authentication response does not finish or fail the parent action.
2. The parent action reports `AIActionStatus::BlockedOnUserAction` while at least one child has a current recoverable GitHub authentication blocker.
3. The parent action remains in `running_actions` while it reports `BlockedOnUserAction`.
4. The parent action never returns to `pending_actions` after acceptance.
5. The parent action never reports the pending-approval `AIActionStatus::Blocked` for runtime authentication.
6. The parent conversation keeps its existing `ConversationStatus`. Runtime authentication does not change the parent conversation to `ConversationStatus::Blocked`.
7. A partial batch reports `BlockedOnUserAction` when any child is recoverably authentication-blocked. This rule applies when sibling children are launched, launching, retrying, or permanently failed.
8. When no recoverable blocker remains and at least one child is launching or retrying, the parent action reports `RunningAsync`.
9. The parent action finishes only when every child is launched, permanently failed, timed out on its current automated attempt, or cancelled with the parent action.
10. A launched sibling stays launched while other children wait for authentication or retry. The client does not restart or cancel that sibling.
11. One GitHub OAuth completion retries every eligible blocked initial child in the process. The user does not authenticate once per child or once per batch.
12. The runtime card contains one `Authenticate with GitHub` action. It uses the normalized cloud-setup URL from the first blocked child in input order.
13. The runtime card groups nonterminal and successful child counts. It omits zero-count groups and uses correct singular or plural copy:
    - `<n> started`
    - `<n> starting`
    - `<n> waiting for GitHub authentication`
14. The runtime card uses this copy:
    - Title: `GitHub Authentication Required`
    - Detail: `Authenticate with GitHub to continue starting the blocked agents.`
    - Action: `Authenticate with GitHub`
15. The runtime card does not show individual rows for launched, launching, retrying, or authentication-blocked children.
16. The runtime card shows one individual row for each permanently failed child. Each row contains the child name and its public failure message.
17. The runtime card does not render resolved model, harness, location, host, environment, runner, API-key, or computer-use parameters. The existing pre-acceptance configuration UI is unchanged.
18. While all blockers are retrying and no child remains blocked, the card returns to the existing post-confirmation spawning presentation.
19. A repeated blocker after a retry requires a later OAuth completion. The same completion does not retry the same child more than once.
20. Each automated launch attempt has its own 30-second timeout.
21. The 30-second timeout runs only while that attempt is performing automated startup work. It pauses while the child waits for human authentication.
22. A retry starts a new 30-second timeout. Time spent waiting for authentication does not reduce the retry timeout.
23. Parent cancellation immediately finishes the parent action as cancelled, clears every unresolved retained child request, and ignores later updates for those requests.
24. If cancellation races with successful server task creation, the client cancels the newly created server task when its task identifier arrives.
25. Parent cancellation does not cancel siblings that were already reported as launched before cancellation.
26. A permanent child failure remains visible and does not prevent recoverably blocked siblings from retrying.
27. Final `RunAgentsResult::Launched.agents` preserves `agent_run_configs` input order regardless of child update order.
28. `RunAgentsResult::Failure` remains reserved for whole-request validation or failure to begin orchestration. Per-child permanent failures and per-attempt timeouts remain `RunAgentsAgentOutcomeKind::Failed` entries in `RunAgentsResult::Launched`.
29. Runtime authentication does not emit an approval permission request or approval response event.
30. Runtime authentication does not capture confirmation focus, hide normal input through the approval path, or bind Enter and Ctrl+C to Accept and Reject. The authentication action and normal parent cancellation remain available through their own runtime controls.
31. The GUI and TUI expose the same state, grouped counts, failure rows, retry behavior, and cancellation behavior.

## Technical design

### Current state
All references are pinned to commit `aa9f3a4364e66cc49561c69751367240a1035207`.

- `app/src/ai/blocklist/action_model.rs:79-132` defines `AIActionStatus`. `Blocked` means a queued action awaits confirmation. `RunningAsync` means an accepted asynchronous action is executing.
- `app/src/ai/blocklist/action_model.rs:601-648` derives `Blocked` from the front of `pending_actions` and `RunningAsync` from `running_actions`.
- `app/src/ai/blocklist/action_model.rs:826-907` removes an accepted asynchronous action from `pending_actions`, adds it to `running_actions`, and changes the parent conversation to `InProgress`.
- `app/src/ai/blocklist/action_model/execute/run_agents.rs:225-374` dispatches child requests, waits on each child receiver serially, applies one 30-second timeout per receiver, and emits a terminal aggregate only after the loop completes.
- `app/src/ai/blocklist/action_model/execute/start_agent.rs:12-238` returns one terminal `StartAgentOutcome`. It removes a pending child request when the child conversation becomes `Blocked`, so a recoverable authentication blocker currently closes the request as an error.
- `app/src/ai/orchestration/remote_child.rs:122-355` classifies a GitHub authentication response as `CloudAgentStartupIssue::Blocked`. `cloud_setup_auth_url_with_next` supplies the normalized callback URL.
- `app/src/terminal/view/ambient_agent/model.rs:1391-1449` retains an initial cloud request and retries it when `GitHubAuthNotifier` emits `AuthCompleted`.
- `app/src/ai/ambient_agents/github_auth_notifier.rs:8-39` emits an unnumbered process-wide completion event. The event has no generation for race detection or duplicate suppression.
- `app/src/terminal/view/ambient_agent/view_impl.rs:295-322` maps a remote child authentication blocker to the child conversation's `ConversationStatus::Blocked`. This child-surface state is valid and remains in use.
- `app/src/pane_group/pane/terminal_pane.rs:1790-1967` owns GUI remote-child surface creation and delegates startup to `AmbientAgentViewModel`.
- `crates/warp_tui/src/orchestration_model.rs:426-566` owns TUI remote-child surface creation and calls the server directly. It records a blocker in `TuiCloudRunState` and the child conversation.
- `crates/warp_tui/src/cloud_run.rs:11-113` retains the TUI child's structured startup blocker.
- `crates/warp_tui/src/cloud_run_view.rs:226-285` currently tells the user to authenticate and rerun orchestration. The retained-request design replaces that copy with automatic retry semantics.
- `app/src/ai/blocklist/inline_action/run_agents_card_view.rs:1203-1296` and `crates/warp_tui/src/orchestration_block/render.rs:229-267` render the post-acceptance state from action status and renderer-local spawning state.
- `app/src/ai/blocklist/block/view_impl/output.rs:3831-3866` maps every action status to a generic GUI action icon.
- `app/src/ai/blocklist/block.rs:5608-5627` makes the floating AI control panel visible only when the latest action reports `is_running()`.
- `crates/warp_tui/src/tool_call_labels.rs:51-181` collapses every `AIActionStatus` into a generic display state. It currently has no runtime-remediation state.
- `crates/warp_tui/src/agent_block.rs:505-644`, `crates/warp_tui/src/tui_generic_tool_call_view.rs:34-126`, and `app/src/ai/blocklist/block.rs:4558-4596` use `Blocked` or `ActionBlockedOnUserConfirmation` to create approval UI and focus behavior.
- `crates/warp_tui/src/cli_agent_osc_event_publisher.rs:91-159` emits CLI `permission_request` events only for `ActionBlockedOnUserConfirmation`. Runtime remediation must not enter this path.
- `app/src/ai/blocklist/action_model/execute.rs:830-914` routes cancellation of a running `RunAgents` action to `RunAgentsExecutor::cancel_execution`.

The root cause is a lifecycle contract mismatch. The child surface can retain and retry an initial request, but `StartAgentExecutor` treats the same blocker as terminal. `RunAgentsExecutor` therefore loses the request that must survive OAuth and cannot represent a live parent action that needs runtime user remediation.

### Action status shape
Add this unit variant to `AIActionStatus`:

```rust
BlockedOnUserAction
```

The variant is a transient client status. Do not serialize or persist it. The authoritative detail, URL, counts, and per-child state live in the `RunAgentsExecutor` progress snapshot.

Keep helper meanings narrow:

- `is_blocked()` continues to mean pending approval only.
- `is_running()` continues to mean `RunningAsync` only.
- Add an explicit runtime-remediation predicate for `BlockedOnUserAction`.
- Add or use an explicit nonterminal-execution predicate where a consumer must include both `RunningAsync` and `BlockedOnUserAction`.
- Do not silently make approval consumers include the new variant through `is_blocked()`.

For an action present in `running_actions`, derive status in this order:

1. Return `BlockedOnUserAction` when the `RunAgentsExecutor` snapshot has one or more current recoverable blockers.
2. Return `RunningAsync` when no blocker remains and automated work is active.
3. Return `Finished` only after the normal terminal result path removes the action from `running_actions`.
Change `BlocklistAIActionModel::get_action_status` to accept `AppContext`. When the requested identifier belongs to a running `RunAgents` action, read `RunAgentsExecutor::snapshot(action_id)` through `BlocklistAIActionExecutor` before selecting the status. Update every call site to pass its existing app or model context. Do not cache a second renderer-owned or action-model-owned copy of the snapshot.

The executor snapshot is authoritative. Progress events contain the action identifier only and invalidate consumers. A renderer or action-status consumer reads a fresh snapshot after each event. It does not treat an event payload as state.

Add `RunAgentsExecutorEvent::ProgressChanged { action_id }` and forward it as a distinct non-approval `BlocklistAIActionEvent::ActionProgressChanged(action_id)`. Emit it after each accepted slot transition and when publication becomes spawning. GUI and TUI block subscriptions use this event only to notify and invalidate layout. They do not emit `ActionBlockedOnUserConfirmation` or update the parent conversation.

### Per-child state machine
`RunAgentsExecutor` stores one indexed slot per input child. Each slot retains the original request and its input index.

- `Preparing`
  - The client is validating and preparing the child.
  - Next states: `Launching`, `Failed`, or `Cancelled`.
- `Launching { attempt, auth_generation_at_start }`
  - The initial automated attempt is in progress.
  - Attempt numbering starts at 1.
  - A 30-second timer is armed for this attempt.
  - Next states: `BlockedOnGitHubAuth`, `Launched`, `Failed`, `TimedOut`, or `Cancelled`.
- `BlockedOnGitHubAuth { attempt, blocker, auth_generation_at_start }`
  - The timer for the attempt is stopped.
  - `blocker` retains the public message and normalized URL.
  - The pending `StartAgentRequest` remains open.
  - Next states: `Retrying` or `Cancelled`.
- `Retrying { attempt, auth_generation_at_start }`
  - The client reuses the retained child request and child surface.
  - `attempt` is the previous attempt plus 1.
  - A new 30-second timer is armed.
  - Next states: `BlockedOnGitHubAuth`, `Launched`, `Failed`, `TimedOut`, or `Cancelled`.
- `Launched { agent_id }`
  - Terminal for launch aggregation.
  - The child continues running independently.
- `Failed { error }`
  - Terminal for launch aggregation.
  - This includes validation failures, permanent startup failures, and a timeout of the current automated attempt.
- `Cancelled`
  - Terminal only for parent-action cancellation.
  - It does not replace a slot that reached `Launched` before cancellation.

`TimedOut` can be represented directly or normalized immediately to `Failed`. Its final public error remains:

`Agent failed to start within 30 seconds. The harness binary may not be installed.`

The state machine ignores an update whose attempt number does not match the slot's current attempt. It also ignores every update after the slot is terminal or the parent action is cancelled.

### Typed start-agent update stream
Replace the one-result `StartAgentOutcome` receiver with a typed update stream. The exact internal names may follow repository conventions, but the stream must represent:

```rust
enum StartAgentUpdate {
    BlockedOnUserAction {
        attempt: u64,
        blocker: CloudAgentStartupBlocker,
    },
    Retrying {
        attempt: u64,
    },
    Started {
        attempt: u64,
        agent_id: String,
    },
    Failed {
        attempt: u64,
        error: String,
    },
    Cancelled {
        attempt: u64,
    },
}
```

Requirements:

- The blocker update carries `CloudAgentStartupBlocker::GitHubAuthRequired` without flattening it to `ConversationStatus::Blocked.blocked_action`.
- The blocker contains the public server message and the URL after `cloud_setup_auth_url_with_next` normalization.
- A blocked update is nonterminal. The sender and pending request remain registered.
- The pending request closes only on `Started`, permanent `Failed`, current-attempt timeout, or cancellation.
- The existing child `ConversationStatus::Blocked` remains a presentation and navigation state for the child pane. It is not the orchestration result channel.
- GUI and TUI launch owners publish the same typed updates to `StartAgentExecutor`.
- Every update associated with an automated attempt, including cancellation, carries that attempt number.
- The stream is internal. It does not add a result variant to public `RunAgentsResult`.

`StartAgentExecutor` continues to correlate requests with child conversations by `StartAgentRequestId`. Extend its retained entry with the original request, current attempt, attempt-start OAuth generation, last consumed completion generation, child conversation identifier, and cancellation state.

### OAuth generation and retry rules
Change `GitHubAuthNotifier` from a stateless event source to a process-wide monotonically increasing completion generation:

- The notifier stores `completion_generation: u64`.
- Every successful GitHub OAuth callback increments the generation once and emits `AuthCompleted { generation }`.
- Duplicate delivery of the same callback must not increment the generation more than once. Use the callback identity already available at the URI completion boundary, or make that boundary invoke the notifier exactly once.
- Every automated child attempt records the current generation as `auth_generation_at_start`.

Retry eligibility is deterministic:

1. When a current-attempt blocker arrives, compare the notifier's current generation with `auth_generation_at_start`.
2. If the current generation is greater, OAuth completed before the blocker was observed. Consume that generation and retry the child immediately once.
3. Otherwise, retain the blocker and wait.
4. When `AuthCompleted { generation }` arrives, retry each retained blocked initial child for which `generation > auth_generation_at_start` and the generation has not already been consumed by that child.
5. Mark the generation consumed before emitting the retry request.
6. Start the retry as the next attempt and set its `auth_generation_at_start` to the consumed generation.
7. A duplicate blocker for the same attempt is idempotent.
8. A duplicate completion event for the same generation is idempotent.
9. If the retry blocks again, it waits for a later generation.
10. A completion after timeout, permanent failure, or cancellation does nothing.

These rules cover callback-before-blocker, blocker-before-callback, duplicate updates, repeated blockers, timeout races, and cancellation races without double-spawning a child.

### Concurrent aggregation and progress snapshot
Replace the serial receiver loop in `RunAgentsExecutor` with concurrent child-update processing.

- Start all valid children without waiting for an earlier child to finish.
- Consume all child streams concurrently.
- Apply each update immediately to its indexed slot.
- Arm and cancel timeout futures per slot and per attempt.
- Recompute grouped counts and projected parent status after every accepted update.
- Emit an invalidation event after state changes.
- Read progress from `RunAgentsExecutor::snapshot(action_id)`.
- Preserve input order only when constructing final `RunAgentsAgentOutcome` values.

The snapshot contains enough state for both renderers:

- Total child count.
- Launched count.
- Launching and retrying count.
- Recoverably blocked count.
- Permanently failed entries in input order.
- The first blocked child's normalized URL in input order.
- Whether automated work remains.

Do not copy run-wide configuration into the progress snapshot.

### GUI ownership
The GUI ownership boundary remains:

- `RunAgentsExecutor` owns batch state, slot state, status projection, aggregation, timeout policy, and final input ordering.
- `StartAgentExecutor` owns retained child requests, attempt correlation, OAuth-generation eligibility, and the typed update stream.
- `PaneGroup` and `TerminalView` own child pane creation, lookup, and cancellation routing.
- `AmbientAgentViewModel` owns the actual remote spawn task and the child pane's cloud startup presentation.

Change the GUI flow as follows:

1. Initial dispatch creates one hidden child pane and links it to `StartAgentRequestId`.
2. `AmbientAgentViewModel` publishes a structured blocked update instead of relying on child `ConversationStatus::Blocked` as the result.
3. A retry reuses the same pane, conversation, `SpawnAgentRequest`, and retained request identifier.
4. The ambient model must not independently issue a second retry for a notifier event already consumed by `StartAgentExecutor`.
5. The child pane continues to show the existing `CloudAgentStartupPresentation::github_auth(..., RetryRetainedRequest)` UI.
6. The parent `RunAgentsCardView` reads the executor snapshot and renders the grouped runtime card.
7. Runtime remediation triggers a dedicated action-progress invalidation event. It does not emit `ActionBlockedOnUserConfirmation`.

### TUI ownership
The TUI ownership boundary remains:

- Shared `RunAgentsExecutor` and `StartAgentExecutor` own the same batch and retained-request semantics as the GUI.
- `TuiOrchestrationModel` owns remote child session creation, the actual server spawn call, reuse of that session for retries, and raced-task cancellation.
- `TuiCloudRunState` owns the individual child's structured startup presentation.
- `TuiOrchestrationBlock` owns the grouped parent card.

Change the TUI flow as follows:

1. Retain the initial prepared spawn request in the child session until launch is terminal or cancelled.
2. Publish `BlockedOnUserAction`, `Retrying`, `Started`, and `Failed` updates with attempt numbers.
3. Subscribe to the centralized retry request from `StartAgentExecutor`; do not implement separate unnumbered retry logic.
4. Reuse the existing TUI cloud child session on retry.
5. Render `RetryRetainedRequest` copy instead of `RerunOrchestrationRequest`.
6. Make `TuiOrchestrationBlock` read the authoritative snapshot and render the same grouped counts, CTA, and permanent-failure rows as the GUI.
7. Do not route the runtime card through `TuiPermissionPrompt`, the orchestration acceptance keymap context, or `active_blocking_input_source`.

### Grouped parent card
Add a shared renderer-neutral presentation derived from the executor snapshot. GUI and TUI may use platform-specific elements, but they must use the same copy and count rules.

- If any child is authentication-blocked, render the authentication title, detail, grouped counts, one CTA, and permanent-failure rows.
- Select the CTA URL from the lowest input-index blocked child.
- If blockers have different public messages or URLs, retain all values internally but still show one CTA and the shared detail. Log the mismatch without exposing duplicate actions.
- When the final blocker enters retry, render the existing spawning state.
- When all slots are terminal, render the existing terminal summary.
- Do not render resolved run-wide parameters after acceptance.
- Do not restore Accept, Reject, edit controls, or confirmation keybindings after acceptance.

### Cancellation
Extend `RunAgentsExecutor::cancel_execution` to cover publication, launching, authentication wait, and retry.

Cancellation order:

1. Mark the parent execution cancelled before notifying child owners.
2. Remove or tombstone every unresolved retained `StartAgentRequest`.
3. Cancel every active per-attempt timeout.
4. Tell GUI or TUI launch owners to cancel attempts that are launching or retrying.
5. Ignore all later typed updates for the cancelled action or stale attempt.
6. If a cancelled attempt returns a task identifier, issue one best-effort server cancellation for that task.
7. Do not cancel slots already in `Launched`.
8. Complete the existing action cancellation path exactly once.

Cancellation must be idempotent. A second cancellation request, late timeout, late blocker, late failure, or late start update has no effect.

### Consumer audit
Update every exhaustive `AIActionStatus` match. Classify each consumer by intent instead of adding the new variant to a wildcard branch.

Approval-only consumers:

- GUI blocked headers, action buttons, speedbumps, and focus.
- TUI permission prompts, AskUserQuestion input ownership, orchestration acceptance, and active blocker selection.
- `AIBlock::is_blocked_on_user_confirmation`.
- `ActionBlockedOnUserConfirmation`.
- CLI `permission_request` and `permission_replied` events.

These consumers continue to react only to `Blocked`.

Nonterminal-execution consumers:

- Parent cancellation routing.
- Unfinished-action checks.
- `AIBlock::should_show_ai_control_panel`.
- Control-panel and progress visibility.
- Card liveness and transcript footer suppression.

These consumers explicitly include `RunningAsync` and `BlockedOnUserAction` where the action is still executing.

Render consumers:

- `RunAgentsCardView` and `TuiOrchestrationBlock` render the dedicated runtime-remediation card for `BlockedOnUserAction`.
- `action_icon` in `app/src/ai/blocklist/block/view_impl/output.rs` maps `BlockedOnUserAction` to an in-progress attention icon. It does not use the approval stop icon or a blocked-action header.
- `tool_call_display_state` adds `ToolCallDisplayState::Remediation` for `BlockedOnUserAction`. The state uses the attention style and the label suffix `(action required)`. It never uses `(awaiting approval)`.
- Non-`RunAgents` actions cannot produce `BlockedOnUserAction`. If they do, log an invariant violation and render a noninteractive attention state.

Conversation consumers:

- Do not derive the parent `ConversationStatus` from `BlockedOnUserAction`.
- Continue to permit child conversations to use `ConversationStatus::Blocked` for their own cloud startup surface.
- Do not emit parent desktop or CLI approval notifications from the child blocker.

### Failure and compatibility behavior
- No server or proto changes.
- No new `RunAgentsResult` variant.
- No persisted representation for `BlockedOnUserAction` or in-flight snapshots.
- Local child launches keep their existing behavior.
- Remote permanent error classification in `CloudAgentStartupFailure` remains unchanged.
- Existing capacity, quota, overload, preparation, and unsupported-mode failures remain permanent per-child failures.
- The GitHub auth URL continues to use `cloud_setup_auth_url_with_next`.
- Existing direct cloud-mode initial-run authentication retry remains supported, but it must use the generation-aware notifier so it cannot double-retry.
- Cloud follow-up authentication remains outside this retained-initial-request contract. The existing follow-up behavior is unchanged.
- Restoring a transcript with no live executor snapshot uses the existing restored/cancelled presentation. The client does not reconstruct an OAuth wait from history.
- A missing, empty, or invalid authentication URL remains a structured blocker internally but disables the CTA and renders the public blocker message. It does not convert the blocker into approval.

## Decisions

### Status representation
- **Chosen: add `AIActionStatus::BlockedOnUserAction`.**
  - Advantages: represents a live accepted action; prevents approval focus, input, queue, conversation-status, and permission-event reuse; gives renderers an exhaustive state.
  - Disadvantages: requires an audit of every exhaustive status match.
- **Rejected: project existing `AIActionStatus::Blocked` from a running action.**
  - Advantage: no enum variant.
  - Disadvantages: `Blocked` currently means front-of-pending-queue approval. Reuse would create approval cards, confirmation focus, permission events, and incorrect parent conversation behavior.
- **Rejected: move the action back to `pending_actions`.**
  - Advantage: reuses existing blocked rendering.
  - Disadvantages: permits re-acceptance, requeue, and re-drain; loses accepted execution identity; can duplicate launched siblings; changes queue ordering and parent status.

### Retry ownership
- **Chosen: generation and attempt logic in `StartAgentExecutor`; network spawn remains in GUI and TUI launch owners.**
  - Advantages: one exactly-once policy; one process-wide notifier reaches all eligible requests; platform owners retain their existing surface and server-call responsibilities.
  - Disadvantage: adds typed coordination events between the shared executor and both frontends.
- **Rejected: independent GUI and TUI OAuth subscriptions.**
  - Advantage: follows the current GUI pattern.
  - Disadvantages: duplicates race logic and allows behavior drift.

### Aggregation
- **Chosen: concurrent child streams with indexed final assembly.**
  - Advantages: one slow or blocked child does not delay sibling progress; timeout budgets overlap; final ordering stays stable.
  - Disadvantage: requires explicit slot state and stale-attempt rejection.
- **Rejected: retain the serial receiver loop.**
  - Advantage: simple aggregation.
  - Disadvantages: later children are not observed promptly and timeout duration scales with child order.

### Rendering source
- **Chosen: executor snapshot is authoritative; events only invalidate.**
  - Advantages: prevents missed-event and event-order bugs; GUI and TUI read identical state.
  - Disadvantage: renderers need snapshot access.
- **Rejected: renderer-local accumulation from event payloads.**
  - Advantage: fewer executor reads.
  - Disadvantages: duplicate state and stale UI after dropped or reordered events.

### Runtime card detail
- **Chosen: grouped counts, one CTA, and rows only for permanent failures.**
  - Advantages: one OAuth completion applies process-wide; avoids one action per child; keeps partial-batch failures actionable.
  - Disadvantage: blocked child names are not individually listed.
- **Rejected: one row and CTA per blocked child.**
  - Advantage: exposes every child state.
  - Disadvantages: implies separate authentication work and scales poorly.
- **Rejected: render resolved run-wide parameters after acceptance.**
  - Advantage: repeats launch configuration.
  - Disadvantages: does not help remediation and diverges from the existing post-confirmation cloud-agent UI.

## Assumptions
- **Assumption:** GitHub authentication is the only recoverable startup blocker in this change. The typed blocker and update stream remain extensible.
- **Assumption:** Child names are unique under existing `run_agents` validation. Permanent failure rows use those names as stable labels.
- **Assumption:** If blocked children provide different normalized URLs, any one URL completes the same process-wide GitHub integration. The first blocked child in input order supplies the CTA URL.
- **Assumption:** Computer-use verification was not requested. Automated model and renderer tests are the required UI proof for this specification.

## Out of scope
- Server, worker, API, GraphQL, or protocol-buffer changes.
- New persisted orchestration-progress state.
- Retrying permanent failures.
- Restarting or cancelling already launched siblings when another child blocks.
- Changing pre-acceptance orchestration configuration, permissions, or auto-execution policy.
- Changing general cloud follow-up OAuth behavior.
- Adding a new CLI-agent protocol event for runtime remediation.
- Redesigning individual child cloud-run panes beyond changing rerun copy to retained retry copy.

## Validation criteria
All criteria must pass before implementation is marked ready.

1. Add action-model tests that place an accepted `RunAgents` action in `running_actions` and assert:
   - no blocker produces `RunningAsync`;
   - any current recoverable child blocker produces `BlockedOnUserAction`;
   - clearing the final blocker while work continues returns to `RunningAsync`;
   - the action never appears in `pending_actions`;
   - the parent `ConversationStatus` does not change.
2. Add `StartAgentExecutor` stream tests for:
   - a blocker is nonterminal and retains the request;
   - start, permanent failure, timeout, and cancellation close the request;
   - child `ConversationStatus::Blocked` does not become a terminal error;
   - stale-attempt updates are ignored.
3. Add notifier-generation tests for:
   - blocker then callback retries once;
   - callback then blocker retries once;
   - duplicate callback delivery retries once;
   - duplicate blocker delivery retries once;
   - two blocked initial children both retry from one completion;
   - a repeated blocker waits for a later generation;
   - a completion after terminal failure or cancellation does nothing.
4. Add timeout tests with paused or injected time:
   - the initial automated attempt times out at 30 seconds;
   - human authentication wait does not time out;
   - retry arms a fresh 30-second timeout;
   - an old attempt's timeout cannot fail a newer attempt;
   - timeout and success races produce one terminal slot result.
5. Add `RunAgentsExecutor` concurrency tests:
   - a later child launches while an earlier child is blocked;
   - sibling updates are processed without input-order blocking;
   - partial batches with launched, failed, launching, retrying, and blocked siblings project `BlockedOnUserAction`;
   - the last blocker entering retry projects `RunningAsync`;
   - final outcomes preserve input order.
6. Add cancellation-race tests:
   - cancellation clears blocked and retrying retained requests;
   - late blocker, failure, timeout, and start updates are ignored;
   - a server task created after cancellation receives one cancellation request;
   - already launched siblings receive no cancellation request;
   - the parent action finishes as cancelled exactly once.
7. Extend GUI `run_agents` card tests to assert:
   - one CTA for multiple blocked children;
   - grouped launched, starting, and blocked counts;
   - permanent failure rows only;
   - first-blocked-input-order URL selection;
   - exact title, detail, and action copy;
   - zero-count groups are omitted;
   - no resolved run-wide parameters appear;
   - final and spawning presentations remain unchanged.
8. Extend GUI ambient-agent tests to assert the same child pane and request are reused on retry, the normalized URL survives the typed update, and the ambient model cannot independently double-retry.
9. Extend TUI orchestration and cloud-run tests to assert:
   - the same retained child session is reused;
   - the child view uses `RetryRetainedRequest` copy;
   - the parent card matches GUI grouped semantics;
   - the runtime card does not enter acceptance or permission-prompt contexts;
   - normal input and explicit cancellation routing remain available.
10. Extend generic renderer and label tests so `BlockedOnUserAction` has a non-approval attention presentation and never includes `awaiting approval`.
11. Extend focus and input tests so only `Blocked` creates approval focus or suppresses input through `active_blocking_input_source`. `BlockedOnUserAction` must not activate Accept, Reject, Enter, Ctrl+C, `TuiPermissionPrompt`, or AskUserQuestion approval behavior. `should_show_ai_control_panel` must remain true so the normal in-flight cancellation control stays available.
12. Extend CLI notification tests so runtime remediation emits neither `permission_request` nor `permission_replied`, while existing pending approvals still emit both events correctly.
13. Compile-time exhaustiveness must force review of all `AIActionStatus` consumers. Do not use a wildcard solely to silence the new variant.
14. Run focused tests from the repository root:
    - `cargo nextest run -p warp --no-fail-fast`
    - `cargo nextest run -p warp_tui --no-fail-fast`
15. Run repository checks:
    - `./script/format --check`
    - `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`
    - `./script/presubmit`

## Approval gate
Do not implement Rust changes until a reviewer explicitly approves both decisions:

1. `AIActionStatus::BlockedOnUserAction` is a distinct nonterminal unit variant. It does not reuse approval `Blocked`, move the action to `pending_actions`, or change parent `ConversationStatus`.
2. In a partial batch, any recoverably authentication-blocked child makes the accepted parent action `BlockedOnUserAction`; launched siblings remain running, one OAuth completion retries every eligible blocked initial child, and final results preserve input order.
