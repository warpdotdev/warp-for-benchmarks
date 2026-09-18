use super::*;

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
    assert_eq!(dialog.confirm_button_text(), "Remove from team");
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
    assert_eq!(dialog.confirm_button_text(), "Remove from workspace");
}

#[test]
fn legacy_team_member_removal_keeps_reload_credit_warning() {
    let mut dialog = CloudActionConfirmationDialog::new();
    dialog.set_variant(CloudActionConfirmationDialogVariant::RemoveTeamMemberReloadCredits);

    assert_eq!(
        dialog.title_text(),
        "Are you sure you want to remove this member?"
    );
    assert_eq!(
        dialog.body_text(),
        "This member will lose access to any remaining reload credits tied to this team. If they rejoin later, they’ll regain access to any unused, non-expired credits."
    );
    assert_eq!(dialog.confirm_button_text(), "Remove Member");
}
