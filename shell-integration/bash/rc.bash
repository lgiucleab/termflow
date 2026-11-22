# TermFlow bash startup shim: load the user's usual startup files, then shell integration.
if [ "${TERMFLOW_BASH_LOGIN-}" = 1 ]; then
  [ -r /etc/profile ] && . /etc/profile
  if [ -r ~/.bash_profile ]; then . ~/.bash_profile
  elif [ -r ~/.bash_login ]; then . ~/.bash_login
  elif [ -r ~/.profile ]; then . ~/.profile
  fi
else
  [ -r ~/.bashrc ] && . ~/.bashrc
fi
unset TERMFLOW_BASH_LOGIN
[ -n "${TERMFLOW_SHELL_INTEGRATION-}" ] && . "$TERMFLOW_SHELL_INTEGRATION"
