# TermFlow shell integration for bash (3.2+). Emits OSC 133 / 6973 marks.
[ -n "${__tf_loaded-}" ] && return
__tf_loaded=1
__tf_cmd_ran=0
__tf_at_prompt=0

__tf_osc() { builtin printf '\033]%s\007' "$1"; }

# DEBUG trap: the first command after the prompt marks the start of a block.
__tf_preexec() {
  [ "$__tf_at_prompt" = 1 ] || return
  [ -n "${COMP_LINE-}" ] && return
  __tf_at_prompt=0
  local cmd
  cmd=$(HISTTIMEFORMAT='' builtin history 1 | command sed 's/^ *[0-9]* *//')
  __tf_osc "6973;cmd=$(builtin printf '%s' "$cmd" | command base64 | command tr -d '\n')"
  __tf_osc "133;C"
  __tf_cmd_ran=1
}

__tf_precmd_first() {
  local ec=$?
  if [ "$__tf_cmd_ran" = 1 ]; then
    __tf_osc "133;D;$ec"
    __tf_cmd_ran=0
  fi
  return $ec
}

__tf_precmd_last() {
  __tf_osc "6973;cwd=$PWD"
  __tf_osc "6973;git_branch=$(command git symbolic-ref --short HEAD 2>/dev/null)"
  __tf_osc "133;A"
  case "$PS1" in
    *$'\033]133;B\007'*) ;;
    *) PS1="$PS1"'\[\033]133;B\007\]' ;;
  esac
  __tf_at_prompt=1
}

PROMPT_COMMAND="__tf_precmd_first${PROMPT_COMMAND:+; $PROMPT_COMMAND}; __tf_precmd_last"
trap '__tf_preexec' DEBUG
