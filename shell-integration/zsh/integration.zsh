# TermFlow shell integration for zsh. Emits OSC 133 / 6973 marks.
[[ -n "${__tf_loaded-}" ]] && return
typeset -g __tf_loaded=1 __tf_cmd_ran=0
autoload -Uz add-zsh-hook

__tf_osc() { builtin printf '\e]%s\a' "$1"; }

# Runs first so $? is still the command's exit status.
__tf_precmd_first() {
  local ec=$?
  if (( __tf_cmd_ran )); then
    __tf_osc "133;D;$ec"
    __tf_cmd_ran=0
  fi
}

# Runs last: metadata, prompt start, and make sure the prompt ends with mark B.
__tf_precmd_last() {
  __tf_osc "6973;cwd=$PWD"
  __tf_osc "6973;git_branch=$(command git symbolic-ref --short HEAD 2>/dev/null)"
  __tf_osc "133;A"
  [[ "$PS1" == *$'\e]133;B\a'* ]] || PS1="$PS1%{"$'\e]133;B\a'"%}"
}

__tf_preexec() {
  __tf_osc "6973;cmd=$(builtin printf '%s' "$1" | command base64 | command tr -d '\n')"
  __tf_osc "133;C"
  __tf_cmd_ran=1
}

precmd_functions=(__tf_precmd_first $precmd_functions)
add-zsh-hook precmd __tf_precmd_last
add-zsh-hook preexec __tf_preexec
