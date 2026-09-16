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
fn native_workspace_team_member_removal_preserves_workspace_access() {
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
        "member@example.com will keep their access to Warp and their other team memberships."
    );
    assert_eq!(
        dialog.confirm_button_text(),
        REMOVE_NATIVE_WORKSPACE_TEAM_MEMBER_CONFIRM_TEXT
    );
}

#[test]
fn workspace_member_removal_names_the_workspace_and_all_its_teams() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(
        CloudActionConfirmationDialogVariant::RemoveMemberFromWorkspace {
            member_email: "member@example.com".to_string(),
            workspace_name: "Warp".to_string(),
        },
    );

    assert_eq!(dialog.title_text(), "Remove from workspace?");
    assert_eq!(
        dialog.body_text(),
        "member@example.com will be removed from all teams in Warp and from the workspace itself."
    );
    assert_eq!(
        dialog.confirm_button_text(),
        REMOVE_MEMBER_FROM_WORKSPACE_CONFIRM_TEXT
    );
}
