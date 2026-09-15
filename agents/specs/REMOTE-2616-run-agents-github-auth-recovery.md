# Recover `run_agents` after GitHub authentication (REMOTE-2616)

## Summary
Keep an accepted `run_agents` action alive when one or more remote children need GitHub authentication. Today the parent finalizes those children as failed, the card says `Failed to spawn agent`, and the hidden child may later start after OAuth with no parent repair. After this change the accepted action stays in `running_actions`, projects a distinct nonterminal `AIActionStatus::BlockedOnUserAction` while any child is recoverably auth-blocked, shows one grouped Authenticate-with-GitHub CTA on the parent card, auto-retries every eligible blocked initial child from one OAuth completion, and only then returns a terminal `RunAgentsResult`. The change is client-only.

This supersedes the original REMOTE-2616 spawn-parameter display request for this PR. Post-accept run-wide parameter rendering is out of scope.

## Product behavior
1. After the user accepts `run_agents`, GitHub authentication is runtime remediation inside that accepted action. It is not a second confirmation and it is not a terminal spawn failure.
2. The parent conversation stays `ConversationStatus::InProgress`. The parent action stays in `running_actions`. The action does not return to pending-approval `AIActionStatus::Blocked`, does not re-enter `pending_actions`, and does not re-drain the queue.
3. Whenever any child in the batch is recoverably GitHub-auth-blocked, the parent action status is `AIActionStatus::BlockedOnUserAction`. This includes mixed batches that also have launched, failed, launching, or retrying siblings.
4. The parent card is the required remediation surface. The user does not need to open a hidden child pane. The card shows one grouped Authenticate-with-GitHub CTA, launched and blocked counts, and individual rows only for distinct permanent failures.
5. CTA copy matches the existing cloud-agent GitHub screen: title `GitHub Authentication Required`, detail `Please authenticate with GitHub to continue`, button `Authenticate with GitHub`. The button opens the server-provided URL after the existing `cloud_setup_auth_url_with_next` normalization.
6. One process-wide GitHub OAuth completion retries every eligible blocked *initial* child. Follow-up executions do not auto-retry (existing `handle_needs_github_auth` clears the retained request for non-`InitialRun`).
7. After authentication, the same accepted `run_agents` call continues. The user does not re-run the orchestration tool call. The card returns to `RunningAsync` only when no recoverable blocker remains and automated work continues.
8. If every child launches, the existing all-success copy remains (`Spawned 1 agent` / `Spawned N agents`). If at least one child launched and the rest failed permanently, the existing mixed `RunAgentsResult::Launched` contract remains (`Spawned K of N agents`). If every child failed permanently, the existing all-failure copy remains. Recoverable auth wait is never those terminal strings.
9. Parent cancellation of the still-running action cancels unresolved launches, ignores late updates, and cancels a raced create-run request. Already launched siblings keep running. The action result is `RunAgentsResult::Cancelled`.
10. Completing GitHub auth outside Warp's return URI leaves the action blocked with a visible cancel path. The card does not silently fail or retry.
11. Restored-from-history `run_agents` cards keep today's cancelled rendering. This change does not persist an in-flight auth wait across app restart.
12. Generic terminal failures continue to hide internal/sensitive error details. Auth remediation is the only new public blocker surface.

## Technical design
*Context:* warp commit `aa9f3a4364e66cc49561c69751367240a1035207` (detached `HEAD` on this checkout). Client-only: `warp-server` and `warp-proto-apis` are unchanged.

### How the area works today
- `AIActionStatus` has `Preprocessing | Queued | Blocked | RunningAsync | Finished`. `Blocked` means the action is the front of `pending_actions` and nothing is in `running_actions` (`action_model.rs:74-93, 615-647`). `handle_not_executed_action` emits `ActionBlockedOnUserConfirmation` and sets parent `ConversationStatus::Blocked` (`action_model.rs:809-831`).
- Accepting `run_agents` removes it from `pending_actions` and puts it in `running_actions` as `RunningAsync` (`start_pending_action_by_id`, `action_model.rs:859-897`). `get_action_status` then always returns `RunningAsync` for that id.
- `RunAgentsExecutor` fans out through `StartAgentExecutor::dispatch` and waits **sequentially** on each slot with a 30s `SPAWN_TIMEOUT` (`run_agents.rs:44-47, 293-334`). The one-shot `StartAgentOutcome` is `Started { agent_id } | Error(String)` (`start_agent.rs:13-19`).
- `start_agent_error_message_for_status` treats child `ConversationStatus::Blocked` as a terminal start error (`start_agent.rs:311-317`). `complete_pending_as_error` then removes the pending request (`start_agent.rs:130-140`). `should_cleanup_failed_child_launch` already tries to keep recoverable `Blocked` children (`start_agent.rs:286-294`), but the pending request is still closed.
- The child ambient model keeps the GitHub URL (`Status::NeedsGithubAuth`, `model.rs:111-115, 1394-1435`) and auto-retries an *initial* retained `SpawnAgentRequest` on `GitHubAuthEvent::AuthCompleted` (`model.rs:1438-1448`; covered by `github_auth_completed_retries_stored_initial_run_request`). The parent never sees that retry: `StartAgentExecutor::handle_history_event` has no pending request, so `complete_pending_as_started` does not run.
- `GitHubAuthNotifier` emits a payload-less `AuthCompleted` (`github_auth_notifier.rs:11-15, 31-32`). There is no completion generation, so a callback that races the blocker is lost (`handle_github_auth_completed` returns unless status is already `NeedsGithubAuth`).
- The GUI card treats anything other than `Finished` / spawning / `RunningAsync` / restored as confirmation UI when status is `Blocked` (`run_agents_card_view.rs:1224-1285`). After finish it collapses to `format_terminal_state` and drops per-child errors (`run_agents_card_view.rs:1595-1656`).
- TUI `TuiOrchestrationBlock::is_awaiting_confirmation` is `AIActionStatus::Blocked` only (`orchestration_block.rs:466-475`). Generic TUI and CLI OSC treat `Blocked` as a permission prompt (`tui_generic_tool_call_view.rs:105-111, 358`; `cli_agent_osc_event_publisher.rs:100-136`).
- `RunAgentsExecutor::cancel_execution` only clears `PendingRunAgents::Publishing` (`run_agents.rs:108-122`). A cancel during `Spawning` does not unregister start-agent requests or retained child spawns.

### Proposed changes

#### 1. Distinct nonterminal action status
Add `AIActionStatus::BlockedOnUserAction` next to `RunningAsync` in `action_model.rs`.

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

Projection rules for `get_action_status`:
- Pending front of queue and no `running_actions` entry: `Blocked` (unchanged).
- Other pending: `Queued` (unchanged).
- Id is in `running_actions`:
  - If `RunAgentsExecutor` reports any recoverably auth-blocked child for that action: `BlockedOnUserAction`.
  - Else: `RunningAsync`.
- Else finished / preprocessing: unchanged.

Helper contract:
- `is_blocked()` matches **only** `Blocked`.
- `is_running()` matches **only** `RunningAsync`.
- New `is_blocked_on_user_action()` matches `BlockedOnUserAction`.
- Do not fold `BlockedOnUserAction` into either existing helper.

The action remains in `running_actions` for the whole auth wait. Do not move it to `pending_actions`. Do not emit `ActionBlockedOnUserConfirmation`. Do not call `handle_not_executed_action`. Do not change parent `ConversationStatus`.

#### 2. Typed start-agent update stream
Replace the one-shot `async_channel::bounded(1)` `StartAgentOutcome` with a typed stream. Pending requests stay in `StartAgentExecutor::pending` across recoverable blockers.

```rust
pub enum StartAgentUpdate {
    BlockedOnUserAction {
        message: String,
        action_url: String,
    },
    Started { agent_id: String },
    Failed { error: String },
}
```

Close the stream (remove from `pending`) only on `Started`, permanent `Failed`, spawn timeout, or cancellation. Do not close on `BlockedOnUserAction`.

`start_agent_error_message_for_status` must stop treating `ConversationStatus::Blocked` as `Failed`. Child `ConversationStatus::Blocked` remains valid for the hidden child conversation. The structured blocker (`message`, normalized `auth_url`) travels on `StartAgentUpdate::BlockedOnUserAction`, not as `StartAgentOutcome::Error(String)`.

Normalize the URL with `github_auth_url::cloud_setup_auth_url_with_next` at the same point `classify_cloud_agent_startup_error` already does (`remote_child.rs:325-332`). Public `message` is the existing user-facing GitHub-auth copy, not a raw internal error.

When the child later receives `ConversationServerTokenAssigned` / an agent id, call `complete_pending_as_started` as today. That path still registers the orchestration event streamer.

#### 3. `RunAgentsExecutor` snapshot is authoritative
Expand `RunAgentsSpawningSnapshot` from `{ agent_count }` to per-child launch state. Keep `agent_count` as `children.len()` or retain the field as a derived convenience; views must read children from the snapshot, not from the original request after accept.

Per-child state machine:

`Launching(attempt) -> BlockedOnGithubAuth { message, action_url, attempt } -> Launching(attempt+1) -> Launched | Failed`

Allowed edges:
- `Launching` -> `BlockedOnGithubAuth` on `StartAgentUpdate::BlockedOnUserAction`.
- `Launching` -> `Launched` on `Started`.
- `Launching` -> `Failed` on permanent `Failed`, timeout, or cancellation of that slot.
- `BlockedOnGithubAuth` -> `Launching(attempt+1)` on an eligible OAuth retry.
- `BlockedOnGithubAuth` -> `Failed` only on parent cancellation of that slot, or if the child becomes a non-recoverable failure *after* a retry (capacity, credits, other). A repeated GitHub-auth blocker stays `BlockedOnGithubAuth` and refreshes `message` / `action_url`. It does not fail the slot.
- No edge from `Launched` back to blocked.

Aggregate parent status from the snapshot:
- Any child `BlockedOnGithubAuth` => parent `BlockedOnUserAction`.
- Else any child `Launching` => parent `RunningAsync`.
- Else all children terminal => send `RunAgentsResult::Launched { agents }` in **input order** and emit `SpawningFinished`.

Process child updates concurrently. Do not wait slot 0 before reading slot 1. Preserve `agent_run_configs` order only when building the final `agents` vec.

`RunAgentsExecutorEvent` keeps `SpawningStarted` / `SpawningFinished` and adds a progress event whose payload is only `action_id`. Views re-read `RunAgentsExecutor::snapshot(action_id)`. Events do not carry the snapshot as source of truth after the first start.

`RunAgentsExecutor` exposes `fn snapshot(&self, action_id: &AIAgentActionId) -> Option<RunAgentsSpawningSnapshot>` for `get_action_status` and for GUI/TUI cards.

#### 4. Timeout
`SPAWN_TIMEOUT` stays 30 seconds and applies **per automated attempt**.
- While the child is `Launching`, the timer runs.
- On `BlockedOnGithubAuth`, pause/cancel that attempt's timer. Human OAuth wait is not a harness timeout.
- On retry `Launching(attempt+1)`, start a fresh 30-second timer.
- Timeout fails only that child slot with the existing timeout copy (`run_agents.rs:322-327`). Other children continue.

#### 5. OAuth completion generations and attempt numbers
Extend `GitHubAuthNotifier` so each `AuthCompleted` carries a monotonically increasing `completion_generation: u64`. Keep a process-wide latest generation on the notifier.

Each blocked initial child stores:
- `attempt: u32` (starts at 1 for the first launch)
- `last_handled_generation: u64` (starts at 0)
- `eligible: bool` (true only for `SessionStartupKind::InitialRun` with a retained `SpawnAgentRequest`)

Retry rules:
- On `AuthCompleted(generation)`, every eligible `BlockedOnGithubAuth` child with `last_handled_generation < generation` retries **once**, then sets `last_handled_generation = generation`.
- If `AuthCompleted` arrives before the blocker is published, latch the generation on the child request (or on the executor slot). When the blocker arrives, if latched generation > `last_handled_generation`, retry immediately without waiting for another callback.
- A duplicate event with the same generation does not retry again.
- A new blocker after a retry waits for a *later* generation. Same-generation duplicates must not double-spawn.
- A callback after cancellation is ignored (`pending` already removed; ambient retained request already cleared).
- Follow-up children remain ineligible (retained request already cleared). They do not join the grouped auto-retry.

`handle_github_auth_completed` in the ambient model stays the mechanism that resubmits the retained `SpawnAgentRequest`. The new generation/attempt guards wrap that call so each eligible child retries at most once per generation.

#### 6. Cancellation
Extend `RunAgentsExecutor::cancel_execution` to cover `Publishing` **and** `Spawning` / auth-wait:
- Remove the pending run.
- For every unresolved start-agent request in the batch: unregister it, close its update stream, tell the child ambient model to clear the retained `SpawnAgentRequest` and to ignore later `AuthCompleted`.
- If a create-run HTTP task is in flight for an unresolved child, cancel that task.
- Ignore late `StartAgentUpdate`s for those request ids.
- Do not cancel conversations that already reached `Launched`.
- Emit `SpawningFinished` and let the existing running-async cancel path record `RunAgentsResult::Cancelled`.

`cancel_action_with_id` already routes running ids through `cancel_running_async_action` (`action_model.rs:1061-1075`). `BlockedOnUserAction` must use that path because the action is still in `running_actions`. Do not treat Ctrl-C / Reject during auth wait as pending-queue deny.

#### 7. GUI card (`run_agents_card_view.rs`)
After accept, never render the confirmation editor, never render run-wide model/harness/mode/env/runner controls, and never render `format_terminal_state` until the action is `Finished`.

While `RunningAsync` with no blockers: keep a compact spawning card. Prefer snapshot launched/launching counts over the old `Spawning N agents…` only count when the snapshot has per-child state.

While `BlockedOnUserAction`:
- Header: waiting for GitHub authentication (not `Failed to spawn`, not `Configuring agents…`, not the confirmation title).
- Grouped counts, for example `Spawned K · Waiting for GitHub authentication (M)`.
- One grouped CTA using `CloudAgentStartupPresentation::github_auth(url, RetryRetainedRequest)`. If several blocked children carry different URLs after normalization, still show one CTA; use the first blocked child's normalized URL. Internal per-child URLs stay on the snapshot.
- Do not list a row per blocked child when they share the same recoverable reason.
- List individual rows only for distinct permanent `Failed` children, using the existing public error string.
- Launched siblings stay visible as launched, not hidden by the blocker.

`is_blocked` on the confirmation card stays `AIActionStatus::Blocked` only. Enter/Ctrl-C confirmation bindings must not fire in `BlockedOnUserAction`. Cancel during auth wait uses the running-action cancel path.

Do not subscribe to `ActionBlockedOnUserConfirmation` for this state. Subscribe to `RunAgentsExecutorEvent` progress and re-read the snapshot.

#### 8. TUI (`orchestration_block.rs`, generic renderers, labels)
- `is_awaiting_confirmation` remains `AIActionStatus::Blocked` only. `BlockedOnUserAction` must return false so the card does not steal input as a permission prompt and does not open configuring pages.
- Render the same grouped counts + one CTA from the executor snapshot. Reuse `CloudAgentStartupPresentation` copy. Open the normalized URL from the TUI action binding.
- `TuiGenericToolCallView` must not call `ensure_permission_prompt` or `render_blocked` for `BlockedOnUserAction`. Exhaustive status matches must add an explicit arm.
- `tool_call_display_state` / `tool_call_label`: `BlockedOnUserAction` is not `Blocked` and must not append `(awaiting approval)`. Use waiting-for-auth / running-adjacent copy, not permission copy.
- CLI OSC (`cli_agent_osc_event_publisher.rs`): do not publish `permission_request` for runtime remediation. Do not emit `ActionBlockedOnUserConfirmation`. Parent `ConversationStatus` stays `InProgress`, so existing `Blocked { .. } => "Warp Agent is waiting for your input."` does not fire on the parent.

#### 9. GUI confirmation focus / input / block helpers
Every consumer that uses `status.is_blocked()`, `matches!(status, AIActionStatus::Blocked)`, or `AIBlock::is_blocked_on_user_confirmation` is a pending-approval consumer. Leave those on `Blocked` only. Required audit sites include:
- `app/src/ai/blocklist/action_model.rs` (`is_blocked`, `get_action_status`)
- `app/src/ai/blocklist/block.rs` (`is_blocked_on_user_confirmation`)
- `app/src/ai/blocklist/block/view_impl/output.rs` (per-tool `AIActionStatus` matches)
- `app/src/ai/blocklist/inline_action/run_agents_card_view.rs`
- `app/src/ai/blocklist/inline_action/requested_command.rs`
- `app/src/ai/blocklist/inline_action/search_codebase.rs`
- `app/src/ai/blocklist/inline_action/code_diff_view.rs`
- `app/src/ai/blocklist/inline_action/ask_user_question_view.rs`
- `crates/warp_tui/src/tool_call_labels.rs`
- `crates/warp_tui/src/tui_generic_tool_call_view.rs`
- `crates/warp_tui/src/orchestration_block.rs`
- `crates/warp_tui/src/cli_agent_osc_event_publisher.rs`
- `crates/warp_tui/src/agent_block.rs`
- `crates/warp_tui/src/tui_shell_command_view.rs`, `tui_file_edits_view.rs`, `tui_ask_question_view.rs`, `tui_permission_prompt.rs`
- integration assertions under `app/src/integration_testing/agent_mode/assertions.rs`

Add an explicit `BlockedOnUserAction` arm at every exhaustive match. Do not use `_ =>` to treat it as confirmation or as a generic running spinner without a named decision.

#### 10. Failure and compatibility
- Terminal `RunAgentsResult` shape is unchanged. Mixed success remains `RunAgentsResult::Launched` when at least one child launched after all slots are terminal.
- `StartAgentUpdate` / snapshot types are client-only. No proto change.
- `CloudAgentStartupBlocker` and `CloudAgentStartupPresentation` stay the shared copy source. Update the outdated comment on `CloudAgentStartupBlocker` that says the original child launch still resolves as failed (`remote_child.rs:129-133`).
- Hidden child `NeedsGithubAuth` screen may still render if the user opens that pane. It is not the required path.
- WASM/TUI exports: export the new status variant through `tui_export.rs` the same way `AIActionStatus` is exported today. Keep wasm compiling; remote-child GitHub retry is a native cloud-spawn path.

## Decisions
### D1. Parent action status while waiting for GitHub auth
Options:
- **A. New `AIActionStatus::BlockedOnUserAction`, action stays in `running_actions` (CHOSEN).** Advantages: pending-approval `Blocked` keeps its meaning; confirmation focus, Enter/Ctrl-C, generic permission cards, and CLI `permission_request` stay correct; `is_blocked()` does not lie; compile-fail exhaustive matches force a consumer audit. Disadvantages: every `AIActionStatus` match site must be updated.
- **B. Project existing `Blocked` from a running action.** Advantages: no new variant. Disadvantages: `get_action_status` today cannot return `Blocked` for an id in `running_actions`; `is_blocked()` would then mean both "please confirm this tool" and "GitHub OAuth"; GUI/TUI confirmation widgets and CLI permission events would fire on an already-accepted action; `handle_not_executed_action` would fight `running_actions`. Rejected: the status-consumer audit found this conflation is the defect.
- **C. Move the action back onto `pending_actions`.** Advantages: reuses the blocked-front-of-queue path. Disadvantages: re-queues and re-drains an accepted action; can re-open confirmation; can change parent `ConversationStatus` to `Blocked`; can auto-execute or deny again; races `try_to_execute_available_actions`. Rejected: authentication is not a new permission gate.

Winner: A. Reviewers must approve this status shape before implementation.

### D2. Partial-batch projection
Options:
- **A. Project `BlockedOnUserAction` when any child is recoverably auth-blocked, including mixed launched/failed/launching/retrying siblings. Return to `RunningAsync` only when no blocker remains and automated work continues (CHOSEN).** Advantages: one parent CTA; launched siblings stay visible; the tool call stays open; matches "keep the accepted action alive". Disadvantages: parent status is blocked even if some children already run.
- **B. Stay `RunningAsync` until every child is blocked or terminal, and only then flip.** Disadvantages: a mixed batch would look like ordinary spawning while a child needs auth; the CTA would appear late or never for partial batches. Rejected by the audit.
- **C. Finalize blocked children as failed and keep the action running for the rest.** Disadvantages: this is today's bug; parent says failed; retry cannot repair the slot. Rejected.

Winner: A. Reviewers must approve these partial-batch semantics before implementation.

### D3. One grouped CTA versus per-child CTAs
One grouped CTA (CHOSEN) versus a row-level button per blocked child. One OAuth completion already retries every eligible child, so extra buttons are duplicate actions. Per-child URLs remain on the snapshot. Distinct permanent failures keep individual rows.

### D4. Start-agent stream versus richer one-shot error
Typed nonterminal updates (CHOSEN) versus encoding the URL in `StartAgentOutcome::Error(String)` and re-dispatching. The one-shot channel is why `complete_pending_as_error` drops the request today. A stream lets `ConversationServerTokenAssigned` complete the same pending request after retry.

### D5. Concurrent child updates versus sequential `recv`
Concurrent (CHOSEN). Sequential wait (`run_agents.rs:296-334`) would pause later children's blocker publication behind earlier timeouts. Input order is restored only in the final `agents` vec.

### D6. Post-accept parameter display
Drop it (CHOSEN). The original REMOTE-2616 request asked to keep run-wide parameters inspectable after spawn. The auth-recovery audit requires the post-confirmation card to match existing cloud-agent UI, not the confirmation editor. Parameter display remains a later REMOTE-2616 follow-up if product still wants it.

## Assumptions
- **Assumption:** Reviewers approve D1 and D2 as written. Implementation must not start from B or C.
- **Assumption:** The only recoverable start-agent blocker in this change is `CloudAgentStartupBlocker::GitHubAuthRequired`. Other `CloudAgentStartupFailure` variants stay terminal.
- **Assumption:** "Eligible" means an *initial* orchestrated child that still holds a retained `SpawnAgentRequest`. Follow-ups stay on today's no-auto-retry path.
- **Assumption:** If normalized auth URLs differ across blocked children, showing the first blocked child's URL is enough because one OAuth completion unblocks every eligible child.
- **Assumption:** Parent cancel during a mixed launched+blocked batch yields `RunAgentsResult::Cancelled`, not mixed `Launched`. Launched child conversations keep running.
- **Assumption:** App restart persistence of an in-flight auth wait is out of scope. Restored cards stay cancelled.
- **Assumption:** No visual computer-use proof is required unless a reviewer opts in. Validation is unit/integration tests plus compile of exhaustive matches.
- **Assumption:** No server or proto change is required; `POST /api/v1/agent/run` already returns `auth_url`.

## Out of scope
- Showing run-wide spawn parameters (model, harness, environment, runner, prompts, skills) on the finished card. Deliberately deferred; conflicts with the post-confirmation cloud-agent UI requirement.
- REMOTE-2409 activity spinner / open-child navigation.
- Changing follow-up GitHub-auth auto-retry.
- Persisting blocked `run_agents` across app restart.
- New recoverable blockers other than GitHub auth.
- Cancelling already-launched siblings when the parent spawn action is cancelled.
- `warp-server` / `warp-proto-apis` changes.
- Teaching the parent conversation a new `ConversationStatus` variant.

## Validation criteria
Each criterion names how it is checked. New tests go in existing `*_tests.rs` files (`run_agents_tests.rs`, `run_agents_card_view_tests.rs`, `model_tests.rs`, TUI orchestration/generic/label tests). No inline `#[cfg(test)]` modules.

1. **Status projection, mixed batch.** A running `run_agents` action with one launched child, one launching child, and one `BlockedOnGithubAuth` child returns `AIActionStatus::BlockedOnUserAction` from `get_action_status`. Checked by a new unit test on `BlocklistAIActionModel` / executor snapshot projection. Verifies behaviors 2–3 and D2.
2. **Not pending-approval `Blocked`.** The same action is not in `pending_actions`, `is_blocked()` is false, `is_running()` is false, `is_blocked_on_user_action()` is true, and parent `ConversationStatus` is `InProgress`. Checked by the same test. Verifies behaviors 1–2 and D1.
3. **Return to `RunningAsync`.** When the last blocker clears and at least one child is still `Launching`, status becomes `RunningAsync`. When all children are terminal, status becomes `Finished`. Checked by advancing the snapshot in that test. Verifies behavior 7.
4. **Pending request survives blocker.** `StartAgentExecutor` keeps the request in `pending` on `BlockedOnUserAction` and later `complete_pending_as_started` on `ConversationServerTokenAssigned`. Checked by a new start-agent unit test. Verifies the Linear "parent card is not repaired" defect.
5. **One OAuth completion retries every eligible initial child once.** Two blocked initial children both resubmit on `AuthCompleted(generation=1)` and do not resubmit on a duplicate generation=1. Checked by notifier + executor tests. Verifies behavior 6 and D3.
6. **Callback-before-blocker.** Fire `AuthCompleted` then publish the blocker; the child retries once without a second callback. Checked by a generation-latch test. Verifies the race rule in §5.
7. **Repeated blocker after retry.** After retry, a second GitHub-auth blocker stays nonterminal and waits for generation=2. Checked by the same suite. Verifies §3 repeated-blocker edge.
8. **Timeout pause and re-arm.** A child that stays in `BlockedOnGithubAuth` for longer than 30s does not fail. A retry that then stays in `Launching` for 30s fails that slot only. Checked by a timeout test with a fake clock or injectable timer. Verifies §4.
9. **Concurrent updates, ordered result.** Child 1 fails permanently while child 0 is still blocked; the snapshot shows both; the final `agents` vec is still request order. Checked by a `run_agents_tests.rs` case. Verifies D5 and behavior 8.
10. **Cancellation.** Cancel the parent action while one child is launched, one is blocked, and one create-run is in flight: retained request cleared, late `Started` ignored, in-flight task cancelled, launched sibling still running, result `Cancelled`. Checked by executor cancel tests. Verifies behavior 9 and §6.
11. **Follow-up ineligible.** A follow-up `NeedsGithubAuth` child does not retry on `AuthCompleted`. Existing `followup_github_auth_does_not_reuse_stored_initial_request` still passes. Verifies behavior 6.
12. **GUI card.** Given `BlockedOnUserAction` plus mixed snapshot, the card is not the confirmation editor, not `Configuring agents…`, not `Failed to spawn agent`, shows grouped launched/blocked counts, shows one Authenticate-with-GitHub CTA, and lists only distinct permanent failures. Checked by `run_agents_card_view_tests.rs`. Verifies behaviors 4–5, 8, 12 and D6.
13. **TUI / generic / labels / CLI.** `is_awaiting_confirmation` is false; generic view does not render a permission prompt; label does not include `(awaiting approval)`; OSC does not emit `permission_request`. Checked by TUI unit tests in `orchestration_block_tests.rs`, `tui_generic_tool_call_view_tests.rs`, `tool_call_labels_tests.rs`, `cli_agent_osc_event_publisher_tests.rs`. Verifies behavior 1 and §8–9.
14. **Consumer compile audit.** `cargo clippy -p warp --all-targets --tests -- -D warnings` and `cargo clippy -p warp_tui --all-targets --tests -- -D warnings` pass. Every `AIActionStatus` match includes `BlockedOnUserAction` with a named decision. Verifies §9.
15. **Compatibility.** Existing mixed/all-success/all-failure `format_terminal_state` tests and `run_agents` launch tests still pass. Restored cards still render cancelled. Verifies behaviors 8, 11.
16. **Repo checks.** From the `warp` repo root: `./script/format --check`, `./script/check_no_inline_test_modules`, and the affected tests (`cargo nextest run -p warp --no-fail-fast` covering `action_model`, `run_agents`, `run_agents_card_view`, `ambient_agent`, plus `cargo nextest run -p warp_tui` for orchestration/generic/label/OSC tests). No `warp-server` diff.

## Approval gate
Do not implement Rust changes from this spec until a reviewer approves:
1. Status shape: new `AIActionStatus::BlockedOnUserAction`, action stays in `running_actions` (Decision D1).
2. Partial-batch semantics: project that status whenever any child is recoverably auth-blocked; return to `RunningAsync` only when no blocker remains and automated work continues (Decision D2).

A comment on this PR, or an explicit approval of those two decisions, is the gate. If either decision is rejected, revise this file on the same branch; do not open a second spec PR.
