//! Shell integration scripts for bash, zsh, and fish.
//!
//! Each public function emits a shell script intended to be eval'd (bash/zsh)
//! or sourced (fish) from the user's shell init file. The script installs a
//! key binding that runs the peanutbutter TUI and injects the selected command
//! into the shell's readline buffer, plus tab-completion for `peanutbutter edit`.

use crate::{BASH_ALIAS_NAME, BINARY_NAME};
use std::env;
use std::io;
use std::path::Path;

/// Emit the bash integration script using the path of the currently running
/// executable. Intended for `peanutbutter bash`; the caller should `eval` the
/// output in their shell init file.
pub fn bash_integration_for_current_exe(binding: &str) -> io::Result<String> {
    let exe = env::current_exe()?;
    bash_integration_script(binding, &exe)
}

/// Build the bash integration script for a given `executable` path and
/// readline `binding` (e.g. `"C+b"`). Separated from
/// [`bash_integration_for_current_exe`] so tests can supply a controlled path.
pub fn bash_integration_script(binding: &str, executable: &Path) -> io::Result<String> {
    let binding = readline_binding(binding)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let executable = shell_quote(&executable.to_string_lossy());
    Ok(format!(
        r#"\builtin unalias {BASH_ALIAS_NAME} &>/dev/null || \builtin true
\builtin alias {BASH_ALIAS_NAME}='{BINARY_NAME}'
__pb_insert_command() {{
  local __pb_cmd
  __pb_cmd=$({executable} execute)
  local __pb_status=$?
  if [[ $__pb_status -ne 0 ]]; then
    return $__pb_status
  fi
  if [[ -z $__pb_cmd ]]; then
    READLINE_LINE="${{READLINE_LINE}}"
    READLINE_POINT=${{READLINE_POINT}}
    return 0
  fi
  READLINE_LINE="${{READLINE_LINE:0:$READLINE_POINT}}${{__pb_cmd}}${{READLINE_LINE:$READLINE_POINT}}"
  READLINE_POINT=$(( READLINE_POINT + ${{#__pb_cmd}} ))
}}
bind -x '"{binding}":__pb_insert_command'
__pb_complete() {{
  local cur subcommand
  cur="${{COMP_WORDS[COMP_CWORD]}}"
  subcommand="${{COMP_WORDS[1]}}"
  if [[ "$subcommand" == "edit" ]]; then
    COMPREPLY=()
    local candidate
    while IFS= read -r candidate; do
      COMPREPLY+=("$candidate")
    done < <({executable} complete-edit "$cur")
    return 0
  fi
  COMPREPLY=( $(compgen -W "bash edit execute zsh fish" -- "$cur") )
}}
complete -o nospace -F __pb_complete {BINARY_NAME} {BASH_ALIAS_NAME}
"#
    ))
}

/// Emit the zsh integration script using the path of the currently running
/// executable. The caller should `eval` the output in their `.zshrc`.
pub fn zsh_integration_for_current_exe(binding: &str) -> io::Result<String> {
    let exe = env::current_exe()?;
    zsh_integration_script(binding, &exe)
}

/// Build the zsh integration script for a given `executable` path and ZLE
/// `binding` (e.g. `"C+b"`). Separated from [`zsh_integration_for_current_exe`]
/// so tests can supply a controlled path.
pub fn zsh_integration_script(binding: &str, executable: &Path) -> io::Result<String> {
    let binding = zsh_bindkey_binding(binding)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let executable = shell_quote(&executable.to_string_lossy());
    Ok(format!(
        r#"\builtin unalias {BASH_ALIAS_NAME} 2>/dev/null; \builtin true
\builtin alias {BASH_ALIAS_NAME}='{BINARY_NAME}'
__pb_insert_command() {{
  local __pb_cmd
  __pb_cmd=$({executable} execute)
  local __pb_status=$?
  if (( __pb_status != 0 )); then
    zle reset-prompt
    return $__pb_status
  fi
  if [[ -z $__pb_cmd ]]; then
    zle reset-prompt
    return 0
  fi
  BUFFER="${{BUFFER:0:$CURSOR}}${{__pb_cmd}}${{BUFFER:$CURSOR}}"
  CURSOR=$(( CURSOR + ${{#__pb_cmd}} ))
  zle reset-prompt
}}
zle -N __pb_insert_command
bindkey "{binding}" __pb_insert_command
_pb_complete() {{
  if [[ "${{words[2]}}" == "edit" ]]; then
    local -a candidates
    candidates=( ${{(f)"$({executable} complete-edit "${{words[CURRENT]}}")"}} )
    compadd -S '' -- "${{candidates[@]}}"
  else
    compadd -- bash edit execute zsh fish
  fi
}}
compdef _pb_complete {BINARY_NAME} {BASH_ALIAS_NAME}
"#
    ))
}

/// Emit the fish integration script using the path of the currently running
/// executable. The caller should `source` the output in their `config.fish`.
pub fn fish_integration_for_current_exe(binding: &str) -> io::Result<String> {
    let exe = env::current_exe()?;
    fish_integration_script(binding, &exe)
}

/// Build the fish integration script for a given `executable` path and key
/// `binding` (e.g. `"C+b"`). Separated from [`fish_integration_for_current_exe`]
/// so tests can supply a controlled path.
pub fn fish_integration_script(binding: &str, executable: &Path) -> io::Result<String> {
    let binding =
        fish_bind_key(binding).map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let executable = shell_quote(&executable.to_string_lossy());
    // The complete-edit helper is extracted into a named function so the
    // single-quoted executable path doesn't conflict with fish's -a '...' quoting.
    Ok(format!(
        r#"function __pb_insert_command
  set -l __pb_cmd ({executable} execute)
  if test -n "$__pb_cmd"
    commandline -i -- $__pb_cmd
  end
  commandline -f repaint
end
function __pb_complete_edit
  {executable} complete-edit (commandline -ct)
end
bind {binding} __pb_insert_command
alias {BASH_ALIAS_NAME}='{BINARY_NAME}'
complete -c {BINARY_NAME} -f -n 'not __fish_seen_subcommand_from bash edit execute zsh fish' -a 'bash edit execute zsh fish'
complete -c {BINARY_NAME} -f -n '__fish_seen_subcommand_from edit' -a '(__pb_complete_edit)'
complete -c {BASH_ALIAS_NAME} -w {BINARY_NAME}
"#
    ))
}

/// Parse a control binding like `C+b` and return the raw key character.
fn parse_ctrl_key(binding: &str) -> Result<char, String> {
    let binding = binding.trim();
    for prefix in ["C+", "C-", "Ctrl+", "Ctrl-", "ctrl+", "ctrl-"] {
        if let Some(rest) = binding.strip_prefix(prefix) {
            let mut chars = rest.chars();
            let ch = chars
                .next()
                .ok_or_else(|| "binding is missing a key after the control prefix".to_string())?;
            if chars.next().is_some() {
                return Err("only single-key control bindings are supported in v1".to_string());
            }
            return Ok(ch);
        }
    }
    Err("expected a control binding like C+b".to_string())
}

fn readline_binding(binding: &str) -> Result<String, String> {
    parse_ctrl_key(binding).map(|ch| format!("\\C-{}", ch.to_ascii_lowercase()))
}

fn zsh_bindkey_binding(binding: &str) -> Result<String, String> {
    parse_ctrl_key(binding).map(|ch| format!("^{}", ch.to_ascii_uppercase()))
}

fn fish_bind_key(binding: &str) -> Result<String, String> {
    parse_ctrl_key(binding).map(|ch| format!("\\c{}", ch.to_ascii_lowercase()))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn bash_script_uses_readline_bind_and_executable_path() {
        let script = bash_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("\\builtin unalias pb &>/dev/null || \\builtin true"));
        assert!(script.contains("\\builtin alias pb='peanutbutter'"));
        assert!(script.contains("bind -x '\"\\C-b\":__pb_insert_command'"));
        assert!(script.contains("'/tmp/peanutbutter' execute"));
        assert!(script.contains("READLINE_LINE=\"${READLINE_LINE}\""));
        assert!(script.contains("READLINE_POINT=${READLINE_POINT}"));
    }

    #[test]
    fn bash_script_registers_edit_completion_for_binary_and_alias() {
        let script = bash_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("__pb_complete()"));
        assert!(script.contains("'/tmp/peanutbutter' complete-edit \"$cur\""));
        assert!(script.contains("complete -o nospace -F __pb_complete peanutbutter pb"));
    }

    #[test]
    fn bash_script_completion_word_list_includes_all_shells() {
        let script = bash_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("bash edit execute zsh fish"));
    }

    #[test]
    fn bash_script_uses_requested_binding() {
        let script = bash_integration_script("C+f", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("bind -x '\"\\C-f\":__pb_insert_command'"));
    }

    #[test]
    fn bash_script_rejects_invalid_binding() {
        assert!(bash_integration_script("notabinding", Path::new("/tmp/pb")).is_err());
    }

    #[test]
    fn zsh_script_registers_zle_widget_and_bindkey() {
        let script = zsh_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("zle -N __pb_insert_command"));
        assert!(script.contains("bindkey \"^B\" __pb_insert_command"));
        assert!(script.contains("'/tmp/peanutbutter' execute"));
        assert!(script.contains("BUFFER="));
        assert!(script.contains("CURSOR="));
    }

    #[test]
    fn zsh_script_emits_pb_alias_and_compdef() {
        let script = zsh_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("\\builtin alias pb='peanutbutter'"));
        assert!(script.contains("compdef _pb_complete peanutbutter pb"));
        assert!(script.contains("'/tmp/peanutbutter' complete-edit"));
    }

    #[test]
    fn zsh_script_uses_requested_binding() {
        let script = zsh_integration_script("C+f", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("bindkey \"^F\" __pb_insert_command"));
    }

    #[test]
    fn zsh_script_rejects_invalid_binding() {
        assert!(zsh_integration_script("notabinding", Path::new("/tmp/pb")).is_err());
    }

    #[test]
    fn fish_script_registers_bind_and_commandline_insert() {
        let script = fish_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("bind \\cb __pb_insert_command"));
        assert!(script.contains("commandline -i -- $__pb_cmd"));
        assert!(script.contains("commandline -f repaint"));
        assert!(script.contains("'/tmp/peanutbutter' execute"));
    }

    #[test]
    fn fish_script_emits_pb_alias_and_completions() {
        let script = fish_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("alias pb='peanutbutter'"));
        assert!(script.contains("complete -c peanutbutter"));
        assert!(script.contains("complete -c pb -w peanutbutter"));
        // complete-edit is called from a helper function to avoid single-quote nesting
        assert!(script.contains("__pb_complete_edit"));
        assert!(script.contains("'/tmp/peanutbutter' complete-edit"));
    }

    #[test]
    fn fish_script_uses_requested_binding() {
        let script = fish_integration_script("C+f", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("bind \\cf __pb_insert_command"));
    }

    #[test]
    fn fish_script_rejects_invalid_binding() {
        assert!(fish_integration_script("notabinding", Path::new("/tmp/pb")).is_err());
    }
}
