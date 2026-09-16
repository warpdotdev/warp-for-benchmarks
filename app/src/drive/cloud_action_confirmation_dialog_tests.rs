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
fn workspace_removal_names_the_member_and_the_workspace() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(
        CloudActionConfirmationDialogVariant::RemoveUserFromWorkspace {
            email: "member@example.com".to_string(),
            workspace_name: "Warp".to_string(),
        },
    );

    assert_eq!(dialog.title_text(), "Remove from workspace?");
    assert_eq!(
        dialog.body_text(),
        "Are you sure you want to remove member@example.com from the workspace? This will remove member@example.com from all teams in Warp and from the workspace itself."
    );
    assert_eq!(
        dialog.confirm_button_text(),
        REMOVE_FROM_WORKSPACE_CONFIRM_TEXT
    );
}

#[test]
fn native_workspace_team_removal_says_workspace_membership_is_kept() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(
        CloudActionConfirmationDialogVariant::RemoveUserFromNativeWorkspaceTeam {
            email: "member@example.com".to_string(),
            workspace_name: "Warp".to_string(),
        },
    );

    assert_eq!(dialog.title_text(), "Remove from team?");
    assert_eq!(
        dialog.body_text(),
        "Are you sure you want to remove member@example.com from this team? member@example.com will still keep their Warp workspace membership and their other team memberships."
    );
    assert_eq!(dialog.confirm_button_text(), REMOVE_FROM_TEAM_CONFIRM_TEXT);
}
