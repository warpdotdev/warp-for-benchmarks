# REMOTE-2616: Keep run_agents action alive during remote child GitHub auth recovery

## Summary
Remote child agents dispatched via `run_agents` frequently encounter GitHub authentication requirements when targeting private repositories or restricted cloud environments. The client currently classifies these startup authentication blockers as unrecoverable errors, immediately marks child slots as permanently failed, and terminates the parent action, forcing the user to re-run orchestration from scratch. This specification defines client-side architecture to keep accepted `run_agents` actions alive in `running_actions` by introducing a distinct nonterminal `AIActionStatus::BlockedOnUserAction` status, establishing a typed `start_agent` update stream that carries structured blocker details across live child requests, coordinating process-wide GitHub OAuth completions across partial batches via generation counters, and presenting a unified post-confirmation remediation card while pausing per-attempt spawn timeouts and preserving running siblings.

## Product behavior
1. **Accepted action continuity during authentication blocker**: When a user accepts a `run_agents` action and one or more remote children require GitHub authentication, the action remains active in `running_actions`. It does not fail, cancel, or re-request pre-run tool approval.
2. **Post-confirmation card presentation**: When the action enters `BlockedOnUserAction`, the card immediately drops all pre-confirmation configuration controls (model, harness, execution mode, runner, environment, and auth secret dropdowns, as well as the Accept/Reject split button). The card mirrors the existing post-confirmation cloud-agent spawning interface.
3. **Grouped counters and single call-to-action (CTA)**: The remediation card displays aggregate counts of launched agents and auth-blocked agents (for example, "2 of 4 agents waiting for GitHub authentication", "2 agents launched"). It provides a single "Authenticate with GitHub" button and lists individual rows only for distinct permanent failures with their respective error messages.
4. **Process-wide OAuth completion**: Clicking "Authenticate with GitHub" opens the normalized GitHub authentication URL in the system browser. Completing OAuth once in the browser triggers a process-wide notification that automatically unblocks and retries all currently auth-blocked initial children across the batch simultaneously.
5. **Partial batch semantics**: In a mixed batch containing launched, launching, failed, and auth-blocked siblings, launched children continue running uninterrupted, permanently failed children show their terminal error messages, and auth-blocked children await authentication. The action projects `BlockedOnUserAction` as long as at least one recoverably blocked child remains.
6. **Automatic transition back to active execution**: As soon as OAuth completes and blocked children begin retrying, if no blocked children remain and automated work continues, the action automatically transitions back to active running (`RunningAsync`) and updates both GUI and TUI representations without user intervention.
7. **Single bounded retry per child**: Each auth-blocked child retries its launch exactly once after OAuth completion. If a retried child encounters another blocker or error, it transitions to a permanent failure rather than entering an infinite retry loop.
8. **Per-attempt timeout suspension**: The automated 30-second spawn timeout pauses while awaiting human authentication and re-arms with a fresh 30-second budget when the child transitions to its retry attempt.
9. **Parent cancellation isolation**: Cancelling the parent `run_agents` action cancels all unresolved/blocked child requests, cleans up hidden child panes that never launched, and sends cancellation requests for in-flight server tasks, while already launched sibling agents continue running.
10. **Frontend parity without false permission prompts**: Both GUI (`run_agents_card_view`) and TUI (`orchestration_block`) present runtime remediation without displaying pre-approval acceptance prompts, without stealing input focus for tool approval, and without emitting false CLI OSC permission request events.

## Technical design

### Current architecture and failure analysis
1. **Status model limitations (`app/src/ai/blocklist/action_model.rs:74-93`)**:
   `AIActionStatus` contains five variants: `Preprocessing`, `Queued`, `Blocked`, `RunningAsync`, and `Finished(Arc<AIAgentActionResult>)`.
   `get_action_status` (`action_model.rs:616-648`) computes `Blocked` exclusively for the front item of `pending_actions` when no action is currently running in the conversation. Any action present in `running_actions` is mapped unconditionally to `RunningAsync`.
2. **Premature child failure in `StartAgentExecutor` (`app/src/ai/blocklist/action_model/execute/start_agent.rs:311-318`)**:
   `StartAgentExecutor::dispatch` returns a one-shot channel `async_channel::Receiver<StartAgentOutcome>`. When a child conversation receives `ConversationStatus::Blocked { blocked_action }`, `start_agent_error_message_for_status` converts it into an error string (`"Child agent startup was blocked before initialization"` or the message). It then calls `complete_pending_as_error`, which removes the pending request from `StartAgentExecutor::pending` and sends `StartAgentOutcome::Error(...)`.
3. **Sequential slot blocking in `RunAgentsExecutor` (`app/src/ai/blocklist/action_model/execute/run_agents.rs:294-336`)**:
   `RunAgentsExecutor::dispatch_children_for_prepared_request` iterates through `slots` sequentially in a blocking loop:
   ```rust path=/workspace/warp/app/src/ai/blocklist/action_model/execute/run_agents.rs start=296
   for slot in slots {
       let kind = match slot {
           ChildSlot::Failed(error) => RunAgentsAgentOutcomeKind::Failed { error },
           ChildSlot::Pending(recv) => {
               let timeout = warpui::r#async::Timer::after(SPAWN_TIMEOUT);
               match futures::future::select(Box::pin(recv.recv()), Box::pin(timeout)).await {
                   // ...
               }
           }
       };
       outcomes.push(kind);
   }
   ```
   When `StartAgentOutcome::Error` arrives, it immediately turns into `RunAgentsAgentOutcomeKind::Failed`. The entire batch finishes with terminal outcomes, removing the action from `pending` and leaving no mechanism for in-flight auth recovery.

---

### Core architectural changes

```
+-----------------------------------------------------------------------------------------+
|                                  BlocklistAIActionModel                                 |
|                                                                                         |
|   pending_actions: [ ... ]                         running_actions: [ run_agents ]      |
|                                                           |                             |
|                                            +--------------v---------------+             |
|                                            | get_action_status(action_id) |             |
|                                            +--------------+---------------+             |
|                                                           |                             |
|                                       Any child in AuthBlocked?                         |
|                                        /                     \                          |
|                                      Yes                      No                        |
|                                       v                        v                        |
|                         BlockedOnUserAction              RunningAsync                   |
+---------------------------------------+------------------------+------------------------+
                                        |                        |
                   +--------------------v------------------------v----+
                   |                 RunAgentsExecutor                |
                   |                                                  |
                   |  ChildSlot 0: Launched { agent_id }              |
                   |  ChildSlot 1: AuthBlocked { blocker, attempt: 1 }|
                   |  ChildSlot 2: Failed { error }                   |
                   +--------------------+-----------------------------+
                                        ^
                                        | (StartAgentUpdate::Blocked)
                   +--------------------+-----------------------------+
                   |                 StartAgentExecutor               |
                   |  pending: { request_id => PendingStartAgent }    |
                   |  (survives Blocked; closes on start/fail/cancel) |
                   +--------------------+-----------------------------+
                                        ^
                                        | (AuthCompleted event)
                   +--------------------+-----------------------------+
                   |                GitHubAuthNotifier                |
                   |  generation: AtomicU64                           |
                   +--------------------------------------------------+
```

#### 1. Status model expansion (`app/src/ai/blocklist/action_model.rs`)
Add a distinct nonterminal variant `BlockedOnUserAction` to `AIActionStatus`:

```rust path=/workspace/warp/app/src/ai/blocklist/action_model.rs start=73
#[derive(Clone, Debug)]
pub enum AIActionStatus {
    Preprocessing,
    Queued,
    /// Front of pending queue awaiting user approval/confirmation to run.
    Blocked,
    /// Executing asynchronously without blockers.
    RunningAsync,
    /// Running asynchronously but paused awaiting runtime remediation from the user.
    BlockedOnUserAction,
    /// Terminal outcome (completed, failed, or cancelled).
    Finished(Arc<AIAgentActionResult>),
}
```

Add status helper methods:
- `pub fn is_blocked(&self) -> bool`: returns `true` **only** for `AIActionStatus::Blocked` (preserves existing approval-gate semantics).
- `pub fn is_blocked_on_user_action(&self) -> bool`: returns `true` for `AIActionStatus::BlockedOnUserAction`.
- `pub fn is_running(&self) -> bool`: returns `true` for `AIActionStatus::RunningAsync | AIActionStatus::BlockedOnUserAction`.
- `pub fn is_actionable_by_user(&self) -> bool`: returns `true` for `Blocked` or `BlockedOnUserAction`.

Update `get_action_status` (`app/src/ai/blocklist/action_model.rs:616-648`):
```rust path=/workspace/warp/app/src/ai/blocklist/action_model.rs start=634
self.running_actions
    .values()
    .find(|running| running.contains(id))
    .map(|_| {
        if self.executor.as_ref(ctx).is_action_blocked_on_user_action(id) {
            AIActionStatus::BlockedOnUserAction
        } else {
            AIActionStatus::RunningAsync
        }
    })
```

#### 2. Typed `start_agent` update stream (`app/src/ai/blocklist/action_model/execute/start_agent.rs`)
Replace the one-shot `async_channel::Sender<StartAgentOutcome>` with an update channel emitting `StartAgentUpdate`:

```rust path=/workspace/warp/app/src/ai/blocklist/action_model/execute/start_agent.rs start=11
#[derive(Debug, Clone)]
pub enum StartAgentUpdate {
    /// Nonterminal blocker requiring user action.
    Blocked(CloudAgentStartupBlocker),
    /// Terminal success with assigned agent id.
    Started { agent_id: String },
    /// Terminal failure.
    Failed { error: String },
}
```

In `StartAgentExecutor`:
- `PendingStartAgent` retains its `sender: async_channel::Sender<StartAgentUpdate>`.
- When `classify_cloud_agent_startup_error` yields `CloudAgentStartupIssue::Blocked(blocker)`:
  1. The pending request sends `StartAgentUpdate::Blocked(blocker)`.
  2. The entry in `self.pending` is **retained**, not removed.
  3. The child conversation remains in `Status::NeedsGithubAuth` and is not cleaned up.
- The entry is removed from `self.pending` only when `StartAgentUpdate::Started` or `StartAgentUpdate::Failed` is sent, or upon explicit cancellation.

#### 3. Concurrent execution and per-child lifecycle in `RunAgentsExecutor` (`app/src/ai/blocklist/action_model/execute/run_agents.rs`)
Replace sequential slot iteration with concurrent update handling:

```rust path=/workspace/warp/app/src/ai/blocklist/action_model/execute/run_agents.rs start=56
#[derive(Debug)]
enum ChildSlotState {
    Preparing,
    Launching {
        attempt: usize,
        timeout_handle: Option<SpawnedFutureHandle>,
    },
    AuthBlocked {
        blocker: CloudAgentStartupBlocker,
        attempt: usize,
        observed_generation: u64,
    },
    Retrying {
        attempt: usize,
        timeout_handle: Option<SpawnedFutureHandle>,
    },
    Launched {
        agent_id: String,
    },
    Failed {
        error: String,
    },
    Cancelled,
}
```

Snapshot structure exposed for UI invalidation and status queries:
```rust path=/workspace/warp/app/src/ai/blocklist/action_model/execute/run_agents.rs start=68
#[derive(Debug, Clone)]
pub struct RunAgentsProgressSnapshot {
    pub total_agents: usize,
    pub launched_count: usize,
    pub auth_blocked_count: usize,
    pub primary_auth_url: Option<String>,
    pub auth_message: Option<String>,
    pub distinct_failures: Vec<(String, String)>, // (child_name, error_message)
    pub is_blocked_on_user_action: bool,
}
```

Event additions on `RunAgentsExecutorEvent`:
```rust path=/workspace/warp/app/src/ai/blocklist/action_model/execute/run_agents.rs start=76
pub enum RunAgentsExecutorEvent {
    SpawningStarted {
        action_id: AIAgentActionId,
        snapshot: RunAgentsSpawningSnapshot,
    },
    ProgressUpdated {
        action_id: AIAgentActionId,
        snapshot: RunAgentsProgressSnapshot,
    },
    SpawningFinished {
        action_id: AIAgentActionId,
    },
}
```

Authoritative snapshot rules:
- `RunAgentsExecutor` owns the authoritative progress state for each in-flight action.
- Child updates update the internal `ChildSlotState` table immediately and emit `ProgressUpdated`.
- Final result ordering is guaranteed: outcomes are mapped back to their original `agent_run_configs` indices when building `RunAgentsResult::Launched`.

#### 4. OAuth generations and race prevention (`app/src/ai/ambient_agents/github_auth_notifier.rs`)
Track an incremental completion generation on `GitHubAuthNotifier`:

```rust path=/workspace/warp/app/src/ai/ambient_agents/github_auth_notifier.rs start=10
#[derive(Debug, Clone)]
pub enum GitHubAuthEvent {
    AuthCompleted { generation: u64 },
}

pub struct GitHubAuthNotifier {
    generation: std::sync::atomic::AtomicU64,
}

impl GitHubAuthNotifier {
    pub fn current_generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn notify_auth_completed(&self, ctx: &mut ModelContext<Self>) {
        let generation = self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        ctx.emit(GitHubAuthEvent::AuthCompleted { generation });
    }
}
```

Race resolution rules:
1. **Callback before blocker**: When a child receives `StartAgentUpdate::Blocked(blocker)`:
   The child slot inspects `GitHubAuthNotifier::current_generation()`. If `current_generation > launch_generation`, auth has already completed while the spawn was in flight. The child immediately transitions to `Retrying` and initiates retry without waiting for a new event.
2. **Duplicate notifications**: When `GitHubAuthEvent::AuthCompleted { generation }` fires:
   Any child in `ChildSlotState::AuthBlocked` checks `generation > observed_generation`. If valid, it transitions to `Retrying`. Repeated notifications with the same or older generation are discarded.
3. **Repeated blockers / single retry bound**:
   Each child slot tracks `attempt: usize`.
   - Initial attempt: `attempt = 1`.
   - Upon auth completion: `attempt` increments to `2`.
   - If a child on `attempt >= 2` receives `StartAgentUpdate::Blocked`, it transitions permanently to `ChildSlotState::Failed { error: "GitHub authentication required; retry attempt failed." }`.
4. **Timeout management**:
   `SPAWN_TIMEOUT = Duration::from_secs(30)`.
   - When a child transitions to `Launching`, start a 30s timer.
   - When transitioning to `AuthBlocked`, abort/drop `timeout_handle`. Human auth wait is unbounded and does not deplete the 30s timeout.
   - When transitioning to `Retrying`, start a fresh 30s timer. If the timer expires before `Started`, transition to `ChildSlotState::Failed`.
5. **Parent cancellation**:
   When `cancel_execution` is invoked for `action_id`:
   - All unresolved child slots (`Launching`, `AuthBlocked`, `Retrying`) are transitioned to `Cancelled`.
   - In-flight server tasks are cancelled via `cancel_spawned_task(task_id)`.
   - Child hidden panes that never received an `agent_id` are cleaned up.
   - Any late updates arriving on child channels for this `action_id` are ignored.
   - Already launched sibling agents (`ChildSlotState::Launched`) remain running.

---

### GUI and TUI frontend implementations

#### GUI (`app/src/ai/blocklist/inline_action/run_agents_card_view.rs`)
1. **Drop pre-confirmation controls**:
   When `status == Some(AIActionStatus::BlockedOnUserAction)`:
   Do not render `render_confirmation_card` or any pickers (model, harness, runner, environment, auth secret, host).
2. **Render remediation card**:
   ```rust
   fn render_blocked_on_user_action_card(
       snapshot: &RunAgentsProgressSnapshot,
       appearance: &Appearance,
       app: &AppContext,
   ) -> Box<dyn Element>
   ```
   - **Header**: Attention icon + "GitHub authentication required".
   - **Summary text**: "Authentication is required to start child agents for this task."
   - **Status counts**:
     - `• {launched_count} of {total} agents launched` (success glyph)
     - `• {auth_blocked_count} of {total} agents waiting for GitHub authentication` (attention glyph)
   - **Failure rows**:
     If `snapshot.distinct_failures` is non-empty, render each failed agent with its error message:
     `• {name}: {error}` (error glyph).
   - **Action row**:
     - Primary button: "Authenticate with GitHub" (renders GitHub icon; clicking opens `snapshot.primary_auth_url` using `app.open_url(...)`).
     - Secondary button: "Cancel" (dispatches `RunAgentsCardViewAction::Reject`).
3. **Subscriptions**:
   Subscribe to `RunAgentsExecutorEvent::ProgressUpdated` to update the retained snapshot and trigger `ctx.notify()`.

#### TUI (`crates/warp_tui/src/orchestration_block/render.rs` & `orchestration_block.rs`)
1. **Keep `interactive` false for remediation**:
   In `crates/warp_tui/src/orchestration_block/render.rs:234-241`:
   ```rust path=/workspace/warp/crates/warp_tui/src/orchestration_block/render.rs start=234
   let status = block.controller.action_status(&block.action_id, app);
   let interactive = !block.is_restored
       && block.spawning.is_none()
       && matches!(status, Some(AIActionStatus::Blocked));
   ```
   `interactive` evaluates to `false` when status is `BlockedOnUserAction`. This ensures the TUI does not render acceptance or configuring mode.
2. **Dedicated remediation section**:
   When `status == Some(AIActionStatus::BlockedOnUserAction)`:
   Render a dedicated remediation card within `render_fallback_tool_call_section`:
   - Title: `■ Run agents (authentication required)`
   - Body:
     - Grouped summary: `{auth_blocked_count} agents awaiting GitHub authentication, {launched_count} launched`
     - Failure details for any terminal failures.
   - Footer hints:
     `Enter to open GitHub login  •  Ctrl + C to cancel`
   - Keybindings: Enter executes `TuiCloudRunAction::OpenUrl(auth_url)`.

---

### Status-consumer audit across the codebase

| Component | File Path | Existing Handling | Required Revision for `BlockedOnUserAction` |
|---|---|---|---|
| `BlocklistAIActionModel` | `app/src/ai/blocklist/action_model.rs` | `running_actions` unconditionally mapped to `RunningAsync` | Check executor; project `BlockedOnUserAction` when auth blockers exist. Add helper methods. |
| `AIBlock` Action Loop | `app/src/ai/blocklist/block.rs:4800` | `action_statuses.iter().any(AIActionStatus::is_blocked)` steals focus for approval | `is_blocked()` remains false for `BlockedOnUserAction`. No focus theft for approval. |
| Code Diff View | `app/src/ai/blocklist/block.rs:3460` | Checks `Finished(result)` | Wildcard arm preserves unchanged behavior. |
| Search Codebase View | `app/src/ai/blocklist/block.rs:4075` | Checks `Finished(result)` | Wildcard arm preserves unchanged behavior. |
| CLI OSC Event Publisher | `crates/warp_tui/src/cli_agent_osc_event_publisher.rs:100` | Listens to `ActionBlockedOnUserConfirmation` | Do not emit `permission_request` OSC event for runtime auth recovery. |
| TUI Tool Call Labels | `crates/warp_tui/src/tool_call_labels.rs:145` | Maps `Blocked` to `State::Blocked` (`"(awaiting approval)"`) | Map `BlockedOnUserAction` to `ToolCallDisplayState::Running` or attention state; format label as `Run agents (authentication required)`. |
| TUI Active Blocker Source | `crates/warp_tui/src/agent_block.rs:977` | Checks `matches!(status, Some(AIActionStatus::Blocked))` to steal input focus | Ignores `BlockedOnUserAction`; does not treat runtime remediation as an unconfirmed tool call. |
| TUI Generic Tool View | `crates/warp_tui/src/tui_generic_tool_call_view.rs:358` | Panics if `Blocked` without `permission_prompt` | `BlockedOnUserAction` falls through to fallback presentation, preventing panics. |
| Permissions Module | `app/src/ai/blocklist/permissions.rs` | Inspects profiles for tool execution permissions | Completely unaffected; runtime remediation is orthogonal to autonomy permissions. |
| Cancellation Routing | `app/src/ai/blocklist/action_model.rs:1068` | If in `running_actions`, calls `cancel_running_async_action` | Action is in `running_actions`, so cancellation routes directly to `cancel_running_async_action` correctly. |

---

### Files and functions to change

1. `app/src/ai/blocklist/action_model.rs`:
   - `AIActionStatus`: add variant `BlockedOnUserAction`.
   - `AIActionStatus::is_blocked_on_user_action(&self) -> bool`.
   - `AIActionStatus::is_running(&self) -> bool`: include `BlockedOnUserAction`.
   - `get_action_status(&self, id: &AIAgentActionId)`: query `RunAgentsExecutor::is_auth_blocked(id)`.
2. `app/src/ai/blocklist/action_model/execute/start_agent.rs`:
   - Define `enum StartAgentUpdate { Blocked(CloudAgentStartupBlocker), Started { agent_id: String }, Failed { error: String } }`.
   - Update `StartAgentExecutor::dispatch` return type to `async_channel::Receiver<StartAgentUpdate>`.
   - Update `start_agent_error_message_for_status` / `handle_history_event` to send `StartAgentUpdate::Blocked` and retain `PendingStartAgent`.
3. `app/src/ai/blocklist/action_model/execute/run_agents.rs`:
   - Replace sequential slot loop in `dispatch_children_for_prepared_request` with concurrent event collection.
   - Implement `ChildSlotState` state machine, 30s timer pause and re-arm, and generation check.
   - Implement `RunAgentsProgressSnapshot` and `is_auth_blocked(&self, action_id) -> bool`.
   - Update `cancel_execution` to cancel server tasks and mark unresolved slots as cancelled.
4. `app/src/ai/ambient_agents/github_auth_notifier.rs`:
   - Add `generation: AtomicU64` and `current_generation(&self) -> u64`.
   - Update `GitHubAuthEvent::AuthCompleted { generation: u64 }`.
5. `app/src/ai/blocklist/inline_action/run_agents_card_view.rs`:
   - In `render(&self)`: handle `AIActionStatus::BlockedOnUserAction` by calling `render_blocked_on_user_action_card`.
   - Render grouped counts, single "Authenticate with GitHub" CTA, failure rows, and Cancel button.
6. `crates/warp_tui/src/orchestration_block/render.rs`:
   - In `render(&TuiOrchestrationBlock)`: ensure `BlockedOnUserAction` keeps `interactive = false`.
   - Render runtime remediation section with grouped counts, auth instructions, and URL opening handler.
7. `crates/warp_tui/src/agent_block.rs`:
   - In `active_blocking_input_source`: verify it checks only `AIActionStatus::Blocked`.
8. `crates/warp_tui/src/tool_call_labels.rs`:
   - In `tool_call_display_state` and `tool_call_label`: handle `AIActionStatus::BlockedOnUserAction` with distinct label and attention glyph.

## Decisions

### 1. Status modeling: `AIActionStatus::BlockedOnUserAction` vs. alternatives
- **Option A: Add a distinct nonterminal `AIActionStatus::BlockedOnUserAction` (Chosen)**
  - *Advantages*: Clear semantic distinction between tool approval gating (pre-execution) and runtime remediation (in-execution). Action remains in `running_actions`. Parent `ConversationStatus` remains `InProgress`. No requeuing, re-draining, or duplicate execution risks. All UI consumers and OSC event publishers can explicitly distinguish remediation from approval.
  - *Disadvantages*: Adds an enum variant to `AIActionStatus`, requiring audit across status consumers.
  - *Why it won*: Eliminates all focus theft, false OSC permission events, and UI regression bugs inherent in overloading `Blocked`.
- **Option B: Project existing `AIActionStatus::Blocked` while action is in `running_actions` (Rejected)**
  - *Advantages*: No new enum variant.
  - *Disadvantages*: Catastrophic UI regressions. `run_agents_card_view.rs` and `orchestration_block/render.rs` branch on `Blocked` to display pre-confirmation editing controls (model, harness, environment pickers) and Accept/Reject buttons for an action that was already accepted. `agent_block.rs` steals keyboard focus for tool approval. `tool_call_labels.rs` appends `(awaiting approval)`. `TuiGenericToolCallView` panics due to missing permission prompts.
- **Option C: Move the action back to `pending_actions` queue (Rejected)**
  - *Advantages*: Reuses queue-level blocking logic.
  - *Disadvantages*: Disrupts queue ordering. Requeuing triggers `try_to_execute_available_actions`, which would re-drain and re-execute `RunAgentsExecutor::execute`. This would re-publish plans and re-spawn already-launched siblings, creating massive server-side duplication and resource leaks. Flipped parent `ConversationStatus` back and forth, breaking streaming and notifications.

### 2. Grouped call-to-action vs. per-child auth buttons
- **Option A: Single grouped CTA button with aggregated counts (Chosen)**
  - *Advantages*: GitHub OAuth authorization is workspace/account-wide. Once the user authenticates, credentials apply to all child environments. A single button avoids UI clutter and repetitive clicking. Individual rows are reserved for distinct permanent failures where error details matter.
  - *Disadvantages*: Does not show per-child auth URLs (unnecessary because all point to the same authorization endpoint).
  - *Why it won*: Matches user mental model and cloud-agent UI patterns.

### 3. Bounded retry policy: single retry vs. unbounded retries
- **Option A: Exactly one retry attempt per auth-blocked child (Chosen)**
  - *Advantages*: Prevents infinite retry loops when user authentication succeeds but credentials lack access to the specific organization or repository.
  - *Disadvantages*: If the user completes auth for the wrong account, a second retry requires re-running the orchestration action.
  - *Why it won*: Protects client stability and prevents automated request storms against server APIs.

### 4. Per-attempt timeout suspension
- **Option A: 30-second timeout per automated attempt, paused during human auth wait (Chosen)**
  - *Advantages*: The user may take several minutes to complete GitHub authentication in their browser. A continuous timer would expire while the user is actively authenticating. Pausing during `AuthBlocked` and re-arming for 30s during `Retrying` ensures automated infrastructure failures are caught while human interaction is respected.
  - *Disadvantages*: Requires tracking timer handles per child slot.
  - *Why it won*: Essential for reasonable user experience during interactive OAuth.

## Assumptions
- The primary GitHub OAuth URL is obtained from the first auth-blocked child and normalized via `github_auth_url::cloud_setup_auth_url_with_next`.
- Because GitHub OAuth grants user/workspace credentials across all cloud runners, completing authentication for one child unblocks all eligible initial children in the batch.
- Computer use verification was not opted in for this backend/client infrastructure task; standard unit, integration, and presubmit tests are sufficient.
- The change is strictly client-only: server APIs already provide structured error objects and task cancellation endpoints.

## Out of scope
- Non-GitHub authentication blockers (e.g. GitLab/Bitbucket credentials or missing third-party harness API keys at runtime, which are validated pre-launch).
- Modifying the server-side multi-agent protobuf definitions or public API endpoints.
- Re-running or restarting sibling child agents that have already launched successfully.
- Auto-retrying terminal errors (such as runner capacity exhaustion or invalid environment configurations).

## Validation criteria
1. **Status projection and helper unit tests** (`app/src/ai/blocklist/action_model.rs`):
   - `test_action_status_blocked_on_user_action_predicates`: verify `is_blocked()` returns `false`, `is_blocked_on_user_action()` returns `true`, and `is_running()` returns `true`.
   - `test_get_action_status_projects_blocked_on_user_action`: verify an action in `running_actions` projects `BlockedOnUserAction` when `is_auth_blocked` is true and returns to `RunningAsync` when resolved.
2. **`RunAgentsExecutor` unit tests** (`app/src/ai/blocklist/action_model/execute/run_agents_tests.rs`):
   - `test_run_agents_single_child_auth_recovery`: mock child emitting `Blocked`, verify action projects `BlockedOnUserAction`, notify `AuthCompleted`, verify child transitions to `Retrying` and launches successfully.
   - `test_run_agents_partial_batch_auth_recovery`: batch of 3 children (1 launched, 1 auth-blocked, 1 permanently failed). Verify action projects `BlockedOnUserAction`, launched sibling continues running, failed sibling records failure, and auth-blocked child succeeds after OAuth.
   - `test_run_agents_repeated_blocker_fails_permanently`: auth-blocked child retries once; if a second blocker is emitted, verify it transitions permanently to `Failed` without looping.
   - `test_run_agents_timeout_pauses_during_auth`: verify 30s timer does not expire while in `AuthBlocked` for >30s; verify fresh 30s timer applies upon `Retrying`.
   - `test_run_agents_callback_before_blocker_generation_race`: trigger `notify_auth_completed` before child blocker arrives; verify child immediately retries without hanging.
   - `test_run_agents_cancellation_cleans_up_and_preserves_launched`: cancelling parent drops unresolved child requests, calls `cancel_spawned_task`, and leaves launched siblings running.
3. **GUI card render tests** (`app/src/ai/blocklist/inline_action/run_agents_card_view_tests.rs`):
   - `test_run_agents_card_renders_remediation_mode`: verify that when status is `BlockedOnUserAction`, no parameter dropdowns or Accept/Reject split buttons are rendered; verify presence of grouped counts and single "Authenticate with GitHub" CTA.
4. **TUI block render tests** (`crates/warp_tui/src/orchestration_block_tests.rs`):
   - `test_tui_orchestration_block_blocked_on_user_action_non_interactive`: verify `interactive` is false and acceptance mode keybindings are not registered; verify remediation text is displayed.
5. **No false CLI permission OSC events** (`crates/warp_tui/src/cli_agent_osc_event_publisher_tests.rs`):
   - `test_osc_publisher_ignores_blocked_on_user_action`: verify no `permission_request` OSC event is published during runtime auth remediation.
6. **Presubmit checks**:
   - `cargo check --workspace --all-targets` passes.
   - `cargo clippy --workspace --all-targets -- -D warnings` passes.
   - `cargo test` passes.
7. **Approval gate**:
   - Formal reviewer sign-off on the `AIActionStatus::BlockedOnUserAction` status shape and partial-batch projection semantics is required prior to code implementation.
