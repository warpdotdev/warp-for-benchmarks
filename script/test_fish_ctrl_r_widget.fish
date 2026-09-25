set -g WARP_SESSION_ID 1
source (status dirname)/../app/assets/bundled/bootstrap/fish.sh >/dev/null

function warp_send_json_message
    set -g test_payload $argv[1]
end

function test_history_widget
    set -g test_widget_calls (math $test_widget_calls + 1)
end

function commandline
    if test "$argv[1]" = -r
        set -g test_commandline_cleared 1
    else
        printf 'selected-history\n'
    end
end

function assert_handoff
    set -l expected "$argv[1]"
    set -l tagged no
    if string match -q '*external_ctrl_r_history*' -- "$test_payload"
        set tagged yes
    end

    set -l expected_tag no
    if test -n "$expected"
        set expected_tag yes
    end

    if test "$_WARP_EXTERNAL_CTRL_R_WIDGET" != "$expected" -o "$tagged" != "$expected_tag"
        printf 'Unexpected Ctrl-R handoff for %s: widget=%s (expected %s), tagged=%s (expected %s)\n' \
            (bind \cr) "$_WARP_EXTERNAL_CTRL_R_WIDGET" "$expected" "$tagged" "$expected_tag" >&2
        return 1
    end
end

for widget in _fzf_search_history fzf-history-widget _atuin_search
    bind \cr $widget
    functions -c test_history_widget $widget
    warp_bootstrapped
    assert_handoff $widget; or exit 1

    if test "$widget" != _atuin_search
        set -g fish_private_mode 1
        set -g test_widget_calls 0
        set -g test_commandline_cleared 0
        warp_run_external_ctrl_r_widget
        if test "$test_widget_calls" -ne 1 -o "$test_commandline_cleared" -ne 1
            printf 'Ctrl-R did not invoke and clear %s\n' "$widget" >&2
            exit 1
        end
        if not string match -q '*"buffer": "selected-history"*' -- "$test_payload"
            printf 'Ctrl-R did not return the selection from %s\n' "$widget" >&2
            exit 1
        end
    end

    functions -e $widget
    warp_bootstrapped
    assert_handoff ''; or exit 1
end

bind \cr unrelated-history-widget
functions -c test_history_widget unrelated-history-widget
warp_bootstrapped
assert_handoff ''; or exit 1
