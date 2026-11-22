# TermFlow zsh shim: restore the user's ZDOTDIR, run their .zshenv, then load shell integration.
if [[ -n "${TERMFLOW_USER_ZDOTDIR+x}" ]]; then
  export ZDOTDIR="$TERMFLOW_USER_ZDOTDIR"
else
  unset ZDOTDIR
fi
unset TERMFLOW_USER_ZDOTDIR
[[ -f "${ZDOTDIR:-$HOME}/.zshenv" ]] && builtin source "${ZDOTDIR:-$HOME}/.zshenv"
if [[ -o interactive && -n "${TERMFLOW_SHELL_INTEGRATION-}" ]]; then
  builtin source "$TERMFLOW_SHELL_INTEGRATION"
fi
