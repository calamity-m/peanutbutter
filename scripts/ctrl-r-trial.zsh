# EXPERIMENTAL: ctrl-r history-search trial for peanutbutter (zsh).
#
# Rebinds Ctrl+R to `pb history`: fuzzy search over recent shell history first,
# Ctrl+T inside the picker falls through to the normal snippet TUI. A history
# pick (or a snippet that consumed the buffer) replaces the whole line;
# a plain snippet pick inserts at the cursor like the usual pb hotkey.
#
# Try it:   source scripts/ctrl-r-trial.zsh
# Custom binary: PB_CTRL_R_TRIAL_BIN=target/debug/peanutbutter source scripts/ctrl-r-trial.zsh
# Undo:     bindkey '^R' history-incremental-search-backward   (or start a new shell)

# Snapshot the binary at source time so the widget keeps working even though
# PB_CTRL_R_TRIAL_BIN was only set for the `source` invocation itself.
__PB_CTRL_R_TRIAL_BIN="${PB_CTRL_R_TRIAL_BIN:-peanutbutter}"

__pb_ctrl_r_trial() {
  local __pb_cmd __pb_status
  # Pipe the full in-memory history (newest first, pb runs filtered out) on
  # stdin — an env var caps out at ~128KiB on Linux, full history does not
  # fit. The TUI reads keys from /dev/tty instead, like fzf.
  __pb_cmd=$(fc -lnr 1 2>/dev/null | sed 's/^\t//' | grep -Ev '^[[:space:]]*(pb|peanutbutter)([[:space:]]|$)' | PEANUTBUTTER_BUFFER="$BUFFER" "$__PB_CTRL_R_TRIAL_BIN" history)
  __pb_status=$?
  if (( __pb_status == 10 )); then
    BUFFER="$__pb_cmd"
    CURSOR=${#__pb_cmd}
    zle reset-prompt
    return 0
  fi
  if (( __pb_status != 0 )); then
    zle reset-prompt
    return $__pb_status
  fi
  if [[ -z $__pb_cmd ]]; then
    zle reset-prompt
    return 0
  fi
  BUFFER="${BUFFER:0:$CURSOR}${__pb_cmd}${BUFFER:$CURSOR}"
  CURSOR=$(( CURSOR + ${#__pb_cmd} ))
  zle reset-prompt
}
zle -N __pb_ctrl_r_trial
bindkey '^R' __pb_ctrl_r_trial
