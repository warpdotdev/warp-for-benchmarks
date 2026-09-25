use command::blocking::Command;

use super::*;

const FISH_BOOTSTRAP: &str = include_str!("../../../app/assets/bundled/bootstrap/fish.sh");

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

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}

/// Returns the definition of a top-level function in the fish bootstrap script.
fn fish_bootstrap_function(name: &str) -> &'static str {
    let start = FISH_BOOTSTRAP
        .find(&format!("\nfunction {name}\n"))
        .unwrap_or_else(|| panic!("fish.sh should define {name}"))
        + 1;
    let len = FISH_BOOTSTRAP[start..]
        .find("\nend\n")
        .expect("top-level fish function should end with an unindented `end`")
        + "\nend".len();
    &FISH_BOOTSTRAP[start..start + len]
}

/// Runs `script` after defining the named bootstrap functions, returning its stdout if it exited
/// successfully. Returns `None` without running anything when fish isn't installed.
fn run_fish(functions: &[&str], script: &str) -> Option<Result<String, String>> {
    if Command::new("fish").arg("--version").output().is_err() {
        eprintln!("skipping: fish is not installed");
        return None;
    }
    let definitions = functions
        .iter()
        .map(|name| fish_bootstrap_function(name))
        .collect::<Vec<_>>()
        .join("\n");
    let output = Command::new("fish")
        .args(["--no-config", "-c", &format!("{definitions}\n{script}")])
        .output()
        .expect("fish should run");
    let stdout = String::from_utf8(output.stdout).expect("fish output should be UTF-8");
    Some(if output.status.success() {
        Ok(stdout)
    } else {
        Err(stdout)
    })
}

fn fish_detected_ctrl_r_widget(setup: &str) -> Option<Result<String, String>> {
    run_fish(
        &["warp_external_ctrl_r_widget"],
        &format!("{setup}\nwarp_external_ctrl_r_widget"),
    )
}

#[test]
fn test_fish_detects_defined_fzf_fish_ctrl_r_widget() {
    let Some(result) = fish_detected_ctrl_r_widget(
        "function _fzf_search_history; end\nbind \\cr _fzf_search_history",
    ) else {
        return;
    };
    assert_eq!(result, Ok("_fzf_search_history\n".to_owned()));
}

#[test]
fn test_fish_ignores_supported_ctrl_r_widget_whose_function_is_missing() {
    let Some(result) = fish_detected_ctrl_r_widget("bind \\cr fzf-history-widget") else {
        return;
    };
    assert_eq!(result, Err(String::new()));
}

#[test]
fn test_fish_ignores_unsupported_ctrl_r_widget() {
    let Some(result) =
        fish_detected_ctrl_r_widget("function my_history_search; end\nbind \\cr my_history_search")
    else {
        return;
    };
    assert_eq!(result, Err(String::new()));
}

#[test]
fn test_fish_ctrl_r_handoff_invokes_fzf_fish_widget() {
    let Some(result) = run_fish(
        &["warp_escape_json", "warp_run_external_ctrl_r_widget"],
        r#"
        set -g fish_private_mode 1
        set -g WARP_SESSION_ID 1
        set -g _WARP_EXTERNAL_CTRL_R_WIDGET _fzf_search_history
        set -g stub_commandline ''
        function commandline
            if test "$argv[1]" = -r
                set -g stub_commandline $argv[2]
            else
                echo $stub_commandline
            end
        end
        function _fzf_search_history
            set -g stub_commandline 'git status'
        end
        function warp_send_json_message
            echo $argv
        end
        warp_run_external_ctrl_r_widget
        "#,
    ) else {
        return;
    };
    let message = result.expect("handoff should succeed");
    assert!(
        message.contains(r#""buffer": "git status""#),
        "unexpected message: {message}"
    );
}
