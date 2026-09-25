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
fn fish_ctrl_r_handoff_requires_allowlisted_existing_function() {
    let output = run_fish_bootstrap_test(
        r#"
function report_ctrl_r_widget
    warp_bootstrapped >/dev/null
    echo "$_WARP_EXTERNAL_CTRL_R_WIDGET"
end

function _fzf_search_history; end
bind \cr _fzf_search_history
report_ctrl_r_widget
functions -e _fzf_search_history
report_ctrl_r_widget

function fzf-history-widget; end
bind \cr fzf-history-widget
report_ctrl_r_widget
functions -e fzf-history-widget
report_ctrl_r_widget

function _atuin_search; end
bind \cr _atuin_search
report_ctrl_r_widget
functions -e _atuin_search
report_ctrl_r_widget

function custom-history-widget; end
bind \cr custom-history-widget
report_ctrl_r_widget
"#,
    );

    assert_eq!(
        output,
        "_fzf_search_history\n\nfzf-history-widget\n\n_atuin_search\n\n\n"
    );
}

#[cfg(unix)]
#[test]
fn fish_ctrl_r_handoff_invokes_tagged_fzf_widget() {
    let output = run_fish_bootstrap_test(
        r#"
function _fzf_search_history
    set -g invoked_widget _fzf_search_history
end
function commandline
    if test (count $argv) -eq 0
        echo selected
    end
end
function warp_send_json_message; end

set -g _WARP_EXTERNAL_CTRL_R_WIDGET _fzf_search_history
warp_run_external_ctrl_r_widget
echo "$invoked_widget"
"#,
    );

    assert_eq!(output, "_fzf_search_history\n");
}

#[cfg(unix)]
fn run_fish_bootstrap_test(assertions: &str) -> String {
    let bootstrap_path = format!(
        "{}/../../app/assets/bundled/bootstrap/fish.sh",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = Command::new("fish")
        .args([
            "-c",
            r#"
set -g WARP_SESSION_ID 1
set -g WARP_IS_LOCAL_SHELL_SESSION 0
set -g WARP_USING_WINDOWS_CON_PTY false
source $argv[1] >/dev/null
eval $argv[2]
"#,
            &bootstrap_path,
            assertions,
        ])
        .output()
        .expect("fish should be installed for Unix tests");

    assert!(
        output.status.success(),
        "fish exited with status {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("fish output should be UTF-8")
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
