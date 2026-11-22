# TermFlow shell integration for fish. Emits OSC 133 / 6973 marks.
# fish 4+ may emit OSC 133 itself; TermFlow ignores repeated marks.
if set -q __tf_loaded
    exit
end
set -g __tf_loaded 1

function __tf_osc
    printf '\e]%s\a' $argv[1]
end

function __tf_preexec --on-event fish_preexec
    __tf_osc "6973;cmd="(printf '%s' $argv[1] | command base64 | command tr -d '\n')
    __tf_osc "133;C"
end

function __tf_postexec --on-event fish_postexec
    __tf_osc "133;D;$status"
end

function __tf_prompt --on-event fish_prompt
    __tf_osc "6973;cwd=$PWD"
    __tf_osc "6973;git_branch="(command git symbolic-ref --short HEAD 2>/dev/null)
    __tf_osc "133;A"
end

functions -c fish_prompt __tf_user_prompt
function fish_prompt
    __tf_user_prompt
    printf '\e]133;B\a'
end
