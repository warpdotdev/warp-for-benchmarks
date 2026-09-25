#[cfg(unix)]
use std::path::PathBuf;

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
fn test_fish_ctrl_r_handoff_invokes_fzf_history_widget() {
    for widget in ["fzf-history-widget", "_fzf_search_history"] {
        assert_eq!(
            run_fish_ctrl_r_handoff(widget, true, true),
            format!("{widget}|{widget}")
        );
    }
}

#[cfg(unix)]
#[test]
fn test_fish_ctrl_r_handoff_requires_supported_function() {
    for widget in ["fzf-history-widget", "_fzf_search_history", "_atuin_search"] {
        assert_eq!(run_fish_ctrl_r_handoff(widget, false, false), "|");
    }
}

#[cfg(unix)]
#[test]
fn test_fish_ctrl_r_handoff_rejects_arbitrary_function() {
    assert_eq!(
        run_fish_ctrl_r_handoff("custom-history-widget", true, false),
        "|"
    );
}

#[cfg(unix)]
fn run_fish_ctrl_r_handoff(widget: &str, define_widget: bool, invoke_widget: bool) -> String {
    let bootstrap = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../app/assets/bundled/bootstrap/fish.sh");
    let widget_definition = if define_widget {
        format!("function {widget}; set -g invoked_widget {widget}; end")
    } else {
        String::new()
    };
    let invocation = if invoke_widget {
        "warp_run_external_ctrl_r_widget >/dev/null 2>/dev/null"
    } else {
        ""
    };
    let script = format!(
        r#"
{widget_definition}
bind \cr {widget}
set -gx WARP_SESSION_ID 1
source "{bootstrap}" >/dev/null 2>/dev/null
{invocation}
printf '%s|%s' "$_WARP_EXTERNAL_CTRL_R_WIDGET" "$invoked_widget"
"#,
        bootstrap = bootstrap.display()
    );
    let output = Command::new("fish")
        .args(["--no-config", "-c", &script])
        .output()
        .expect("fish should be installed");
    assert!(
        output.status.success(),
        "fish exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("fish output should be valid UTF-8")
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
