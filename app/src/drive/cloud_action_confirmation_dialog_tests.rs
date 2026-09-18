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
fn native_workspace_remove_from_workspace_copy() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(CloudActionConfirmationDialogVariant::RemoveFromWorkspace {
        member_email: "member@example.com".to_string(),
        workspace_name: "Acme".to_string(),
    });

    assert_eq!(dialog.title_text(), "Remove from workspace?");
    assert_eq!(
        dialog.body_text(),
        "This will remove member@example.com from all teams in Acme and from the workspace itself."
    );
    assert_eq!(dialog.confirm_button_text(), "Remove from workspace");
}

#[test]
fn native_team_remove_omits_reload_credits() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(CloudActionConfirmationDialogVariant::RemoveFromNativeTeam {
        member_email: "member@example.com".to_string(),
        workspace_name: "Acme".to_string(),
    });

    assert_eq!(dialog.title_text(), "Remove from team?");
    assert_eq!(
        dialog.body_text(),
        "member@example.com will remain in Acme and any other teams they belong to."
    );
    assert_eq!(dialog.confirm_button_text(), "Remove from team");
}
