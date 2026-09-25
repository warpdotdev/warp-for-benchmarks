#[cfg(unix)]
#[allow(clippy::disallowed_types)]
use std::process::Output;

#[cfg(unix)]
use command::blocking::Command;

use super::*;

struct TestAssetProvider;

impl AssetProvider for TestAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/bash.sh" => "#include hello_world",
            "bundled/bootstrap/fish.sh" => "# this is a comment\nthis_is_a_command",
            "bundled/bootstrap/zsh.sh" => {
                "asdf\n#include whitespace\n    prepended whitespace\n\n\n"
            }
            "bundled/bootstrap/pwsh.ps1" => {
                r#"# This is a comment
                Write-Output 'Testing some output'
                function test1 {
                    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingInvokeExpression', '', Justification = 'We actually need it')]
                    param([string]$command)
                    Invoke-Expression $command
                }"#
            }
            "hello_world" => "hello world!",
            "whitespace" => "no whitespace\n\n\n yes whitespace!",
            _ => anyhow::bail!("path not found in assets"),
        };
        Ok(Cow::Borrowed(content.as_bytes()))
    }
}

#[test]
fn test_include_directive() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &TestAssetProvider)),
        "hello world!\n"
    );
}

#[test]
fn test_trims_comments() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Fish, &TestAssetProvider)),
        "this_is_a_command\n"
    );
}

#[test]
fn test_trims_whitespace() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Zsh, &TestAssetProvider)),
        "asdf\nno whitespace\n yes whitespace!\n prepended whitespace\n"
    );
}

#[test]
fn test_trims_powershell_specifics() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::PowerShell, &TestAssetProvider)),
        " Write-Output 'Testing some output'\n function test1 {\n param([string]$command)\n Invoke-Expression $command\n }\n"
    );
}

#[cfg(unix)]
#[test]
fn test_fish_ctrl_r_handoff_requires_supported_function() {
    let cases = [
        ("fzf-history-widget", true, "fzf-history-widget"),
        ("_fzf_search_history", true, "_fzf_search_history"),
        ("_atuin_search", true, "_atuin_search"),
        ("fzf-history-widget", false, ""),
        ("_fzf_search_history", false, ""),
        ("_atuin_search", false, ""),
        ("custom-history-widget", true, ""),
    ];

    for (widget, define_function, expected) in cases {
        let function = if define_function {
            format!("function {widget}; end")
        } else {
            String::new()
        };
        let script = format!(
            r#"
                {function}
                bind \cr {widget}
                set -gx WARP_SESSION_ID 1
                source "$WARP_FISH_BOOTSTRAP" >/dev/null
                printf '%s' "$_WARP_EXTERNAL_CTRL_R_WIDGET"
            "#
        );
        let Some(output) = run_fish(&script) else {
            return;
        };

        assert!(
            output.status.success(),
            "fish failed for {widget}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    }
}

#[cfg(unix)]
#[test]
fn test_fish_ctrl_r_handoff_invokes_tagged_widget() {
    let script = r#"
        function _fzf_search_history
            set -g invoked_widget _fzf_search_history
        end
        bind \cr _fzf_search_history
        set -gx WARP_SESSION_ID 1
        source "$WARP_FISH_BOOTSTRAP" >/dev/null
        function commandline
            if test (count $argv) -eq 0
                printf selected
            end
        end
        function warp_send_json_message; end
        warp_run_external_ctrl_r_widget
        printf '%s' "$invoked_widget"
    "#;
    let Some(output) = run_fish(script) else {
        return;
    };

    assert!(
        output.status.success(),
        "fish failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "_fzf_search_history"
    );
}

#[cfg(unix)]
fn run_fish(script: &str) -> Option<Output> {
    Command::new("fish").arg("--version").output().ok()?;

    Some(
        Command::new("fish")
            .args(["--no-config", "--command", script])
            .env(
                "WARP_FISH_BOOTSTRAP",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../app/assets/bundled/bootstrap/fish.sh"
                ),
            )
            .output()
            .expect("fish was available for the version check"),
    )
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
