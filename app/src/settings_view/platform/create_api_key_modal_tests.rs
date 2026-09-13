use warp_core::ui::appearance::Appearance;
use warp_server_client::auth::AgentIdentity;
use warpui::platform::WindowStyle;
use warpui::{App, TypedActionView, ViewHandle};

use super::{ApiKeyType, CreateApiKeyModal, CreateApiKeyModalAction};
use crate::auth::AuthStateProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn agent(uid: &str, name: &str, available: bool) -> AgentIdentity {
    AgentIdentity {
        uid: uid.to_string(),
        name: name.to_string(),
        available,
    }
}

fn create_modal(app: &mut App) -> ViewHandle<CreateApiKeyModal> {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(UserWorkspaces::default_mock);

    let (_, view) = app.add_window(WindowStyle::NotStealFocus, CreateApiKeyModal::new);
    view
}

/// Regression test for the searchable Agent picker in the New API key modal:
/// the agent dropdown is a `FilterableDropdown`, lists only available agents,
/// and filtering by a query narrows the visible list case-insensitively.
#[test]
fn test_agent_dropdown_is_searchable() {
    App::test((), |mut app| async move {
        let view = create_modal(&mut app);

        // Populate the picker with several agents; the unavailable one should be
        // excluded from the list entirely.
        view.update(&mut app, |modal, ctx| {
            modal.set_agents_for_test(
                vec![
                    agent("1", "Default Service Account", true),
                    agent("2", "Ben's Agent", true),
                    agent("3", "Server Migration Agent", true),
                    agent("4", "Unavailable Agent", false),
                ],
                ctx,
            );
        });

        // Only the 3 available agents are listed, and all are visible with no filter.
        let total = view.read(&app, |modal, ctx| modal.agent_dropdown.as_ref(ctx).len());
        assert_eq!(total, 3, "only available agents should be listed");
        let all_visible = view.read(&app, |modal, ctx| {
            modal
                .agent_dropdown
                .as_ref(ctx)
                .visible_items_len_for_test(ctx)
        });
        assert_eq!(all_visible, 3);

        // Typing a query filters the list case-insensitively.
        view.update(&mut app, |modal, ctx| {
            modal.agent_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_filter_query_for_test("BEN", ctx)
            });
        });
        let filtered = view.read(&app, |modal, ctx| {
            modal
                .agent_dropdown
                .as_ref(ctx)
                .visible_items_len_for_test(ctx)
        });
        assert_eq!(filtered, 1, "query should match only \"Ben's Agent\"");

        // A non-matching query yields no matches.
        view.update(&mut app, |modal, ctx| {
            modal.agent_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_filter_query_for_test("zzz", ctx)
            });
        });
        let none = view.read(&app, |modal, ctx| {
            modal
                .agent_dropdown
                .as_ref(ctx)
                .visible_items_len_for_test(ctx)
        });
        assert_eq!(none, 0);

        // Clearing the query restores the full list.
        view.update(&mut app, |modal, ctx| {
            modal.agent_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_filter_query_for_test("", ctx)
            });
        });
        let restored = view.read(&app, |modal, ctx| {
            modal
                .agent_dropdown
                .as_ref(ctx)
                .visible_items_len_for_test(ctx)
        });
        assert_eq!(restored, 3);
    })
}

#[test]
fn default_agent_is_selected_in_dropdown_and_creation_state() {
    App::test((), |mut app| async move {
        let view = create_modal(&mut app);

        view.update(&mut app, |modal, ctx| {
            modal.set_agents_for_test(
                vec![
                    agent("unavailable", "Unavailable Agent", false),
                    agent("default", "Default Agent", true),
                    agent("other", "Other Agent", true),
                ],
                ctx,
            );
        });

        view.read(&app, |modal, ctx| {
            assert_eq!(modal.selected_agent_uid.as_deref(), Some("default"));
            assert_eq!(
                modal.agent_dropdown.as_ref(ctx).selected_item_label(),
                Some("Default Agent".to_string())
            );
            assert!(!modal.is_create_disabled(ApiKeyType::Agent));
        });
    })
}

#[test]
fn explicit_agent_selection_survives_agent_list_refresh() {
    App::test((), |mut app| async move {
        let view = create_modal(&mut app);

        view.update(&mut app, |modal, ctx| {
            modal.set_agents_for_test(
                vec![
                    agent("default", "Default Agent", true),
                    agent("selected", "Selected Agent", true),
                ],
                ctx,
            );
            modal.handle_action(
                &CreateApiKeyModalAction::SelectAgent("selected".to_string()),
                ctx,
            );
            modal.set_agents_for_test(
                vec![
                    agent("default", "Default Agent", true),
                    agent("selected", "Selected Agent", true),
                ],
                ctx,
            );
        });

        view.read(&app, |modal, ctx| {
            assert_eq!(modal.selected_agent_uid.as_deref(), Some("selected"));
            assert_eq!(
                modal.agent_dropdown.as_ref(ctx).selected_item_label(),
                Some("Selected Agent".to_string())
            );
            assert!(!modal.is_create_disabled(ApiKeyType::Agent));
        });
    })
}

#[test]
fn empty_agent_list_clears_selection_and_disables_creation() {
    App::test((), |mut app| async move {
        let view = create_modal(&mut app);

        view.update(&mut app, |modal, ctx| {
            modal.set_agents_for_test(vec![agent("default", "Default Agent", true)], ctx);
            modal.set_agents_for_test(Vec::new(), ctx);
        });

        view.read(&app, |modal, ctx| {
            assert_eq!(modal.selected_agent_uid, None);
            assert_eq!(modal.agent_dropdown.as_ref(ctx).selected_item_label(), None);
            assert!(modal.is_create_disabled(ApiKeyType::Agent));
        });
    })
}

#[test]
fn loading_agents_disables_agent_key_creation() {
    App::test((), |mut app| async move {
        let view = create_modal(&mut app);

        view.update(&mut app, |modal, ctx| {
            modal.set_agents_for_test(vec![agent("default", "Default Agent", true)], ctx);
            modal.is_loading_agents = true;
        });

        view.read(&app, |modal, _| {
            assert!(modal.is_create_disabled(ApiKeyType::Agent));
        });
    })
}

#[test]
fn personal_key_creation_does_not_require_an_agent() {
    App::test((), |mut app| async move {
        let view = create_modal(&mut app);

        view.read(&app, |modal, _| {
            assert!(!modal.is_create_disabled(ApiKeyType::Personal));
        });
    })
}

#[test]
fn default_agent_selection_is_restored_after_modal_reset() {
    App::test((), |mut app| async move {
        let view = create_modal(&mut app);

        view.update(&mut app, |modal, ctx| {
            modal.set_agents_for_test(
                vec![
                    agent("1", "Default Service Account", true),
                    agent("2", "Ben's Agent", true),
                ],
                ctx,
            );
            modal.agent_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_filter_query_for_test("Ben", ctx);
            });
        });

        view.read(&app, |modal, ctx| {
            assert_eq!(
                modal.agent_dropdown.as_ref(ctx).selected_item_label(),
                Some("Ben's Agent".to_string())
            );
        });

        view.update(&mut app, |modal, ctx| {
            modal.on_close(ctx);
            modal.set_agents_for_test(
                vec![
                    agent("1", "Default Service Account", true),
                    agent("2", "Ben's Agent", true),
                ],
                ctx,
            );
        });

        view.read(&app, |modal, ctx| {
            assert_eq!(modal.selected_agent_uid.as_deref(), Some("1"));
            assert_eq!(
                modal.agent_dropdown.as_ref(ctx).selected_item_label(),
                Some("Default Service Account".to_string())
            );
            assert!(!modal.is_create_disabled(ApiKeyType::Agent));
        });
    })
}
