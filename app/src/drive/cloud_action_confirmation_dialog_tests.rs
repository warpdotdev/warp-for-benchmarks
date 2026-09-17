use super::*;

#[test]
fn native_workspace_leave_preserves_other_access() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(
        CloudActionConfirmationDialogVariant::LeaveNativeWorkspaceTeam {
            team_name: "Warp".to_string(),
        },
    );

    assert_eq!(dialog.title_text(), "Leave Warp?");
    assert_eq!(
        dialog.body_text(),
        "Your workspace access and other team memberships won’t change."
    );
    assert_eq!(dialog.confirm_button_text(), LEAVE_TEAM_CONFIRM_TEXT);
}

#[test]
fn native_workspace_team_removal_preserves_workspace_access() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(
        CloudActionConfirmationDialogVariant::RemoveNativeWorkspaceTeamMember {
            member_email: "member@example.com".to_string(),
            workspace_name: "Warp".to_string(),
        },
    );

    assert_eq!(dialog.title_text(), "Remove from team?");
    assert_eq!(
        dialog.body_text(),
        "member@example.com will retain access to the Warp workspace and any other teams they’re a member of."
    );
    assert_eq!(dialog.confirm_button_text(), "Remove from team");
}

#[test]
fn native_workspace_removal_describes_full_scope() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(
        CloudActionConfirmationDialogVariant::RemoveWorkspaceMember {
            member_email: "member@example.com".to_string(),
            workspace_name: "Warp".to_string(),
        },
    );

    assert_eq!(dialog.title_text(), "Remove from workspace?");
    assert_eq!(
        dialog.body_text(),
        "member@example.com will be removed from all teams in the Warp workspace and from the workspace itself."
    );
    assert_eq!(dialog.confirm_button_text(), "Remove from workspace");
}

#[test]
fn legacy_team_removal_keeps_reload_credit_warning() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(
        CloudActionConfirmationDialogVariant::RemoveTeamMemberReloadCredits {
            member_email: "member@example.com".to_string(),
        },
    );

    assert_eq!(
        dialog.body_text(),
        "member@example.com will lose access to any remaining reload credits tied to this team. If they rejoin later, they’ll regain access to any unused, non-expired credits."
    );
}
