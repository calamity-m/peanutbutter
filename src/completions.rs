//! Shell integration scripts for bash, zsh, fish, and PowerShell.
//!
//! Each public function emits a shell script intended to be eval'd (bash/zsh),
//! sourced (fish), or added to the user's PowerShell profile. The script installs a
//! key binding that runs the peanutbutter TUI and injects the selected command
//! into the shell's readline buffer, plus tab-completion for `peanutbutter edit`.

use crate::{BASH_ALIAS_NAME, BINARY_NAME, REPLACE_BUFFER_EXIT_CODE};
use std::env;
use std::io;
use std::path::Path;

const TOP_LEVEL_COMMANDS: &str = "execute init edit new completions lint gc stats settings lsp";
const SHELL_TARGETS: &str = "bash zsh fish powershell";
const POWERSHELL_TOP_LEVEL_COMMANDS: &str =
    "'execute','init','edit','new','completions','lint','gc','stats','settings','lsp','--theme'";
const POWERSHELL_SHELL_TARGETS: &str = "'bash','zsh','fish','powershell'";

/// Shell targeted by `peanutbutter completions <shell>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Shell {
    /// Emit bash integration code.
    Bash,
    /// Emit zsh integration code.
    Zsh,
    /// Emit fish integration code.
    Fish,
    /// Emit PowerShell integration code.
    Powershell,
}

/// Emit the bash integration script using the path of the currently running
/// executable. Intended for `peanutbutter completions bash`; the caller should `eval` the
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
    let replace_code = REPLACE_BUFFER_EXIT_CODE;
    Ok(format!(
        r#"\builtin unalias {BASH_ALIAS_NAME} &>/dev/null || \builtin true
__pb_dispatch() {{
  if [[ "$1" == "new" ]]; then
    local __pb_hist
    __pb_hist=$(fc -lnr 1 50 2>/dev/null | sed 's/^\t//' | grep -Ev '^[[:space:]]*(pb|peanutbutter)([[:space:]]|$)' | awk 'BEGIN{{total=0; max=65536}} {{len=length($0)+1; if (total+len>max) exit; total+=len; print}}' | tr '\n' '\037')
    PEANUTBUTTER_HISTORY="$__pb_hist" command {executable} "$@"
  else
    command {executable} "$@"
  fi
}}
{BASH_ALIAS_NAME}() {{ __pb_dispatch "$@"; }}
{BINARY_NAME}() {{ __pb_dispatch "$@"; }}
__pb_insert_command() {{
  local __pb_cmd
  __pb_cmd=$(PEANUTBUTTER_BUFFER="$READLINE_LINE" {executable} execute)
  local __pb_status=$?
  if [[ $__pb_status -eq {replace_code} ]]; then
    READLINE_LINE="$__pb_cmd"
    READLINE_POINT=${{#__pb_cmd}}
    return 0
  fi
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
  local cur prev subcommand
  cur="${{COMP_WORDS[COMP_CWORD]}}"
  prev="${{COMP_WORDS[COMP_CWORD-1]}}"
  subcommand="${{COMP_WORDS[1]}}"
  if [[ "$prev" == "--theme" ]]; then
    COMPREPLY=( $(compgen -W "$({executable} complete-theme "$cur")" -- "$cur") )
    return 0
  fi
  if [[ "$subcommand" == "edit" ]]; then
    COMPREPLY=()
    local candidate
    while IFS= read -r candidate; do
      COMPREPLY+=("$candidate")
    done < <({executable} complete-edit "$cur")
    return 0
  fi
  if [[ "$subcommand" == "completions" ]]; then
    COMPREPLY=( $(compgen -W "{SHELL_TARGETS}" -- "$cur") )
    return 0
  fi
  COMPREPLY=( $(compgen -W "--theme {TOP_LEVEL_COMMANDS}" -- "$cur") )
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
    let replace_code = REPLACE_BUFFER_EXIT_CODE;
    Ok(format!(
        r#"\builtin unalias {BASH_ALIAS_NAME} 2>/dev/null; \builtin true
__pb_dispatch() {{
  if [[ "$1" == "new" ]]; then
    local __pb_hist
    __pb_hist=$(fc -lnr -50 2>/dev/null | sed 's/^\t//' | grep -Ev '^[[:space:]]*(pb|peanutbutter)([[:space:]]|$)' | awk 'BEGIN{{total=0; max=65536}} {{len=length($0)+1; if (total+len>max) exit; total+=len; print}}' | tr '\n' '\037')
    PEANUTBUTTER_HISTORY="$__pb_hist" command {executable} "$@"
  else
    command {executable} "$@"
  fi
}}
{BASH_ALIAS_NAME}() {{ __pb_dispatch "$@"; }}
{BINARY_NAME}() {{ __pb_dispatch "$@"; }}
__pb_insert_command() {{
  local __pb_cmd
  __pb_cmd=$(PEANUTBUTTER_BUFFER="$BUFFER" {executable} execute)
  local __pb_status=$?
  if (( __pb_status == {replace_code} )); then
    BUFFER="$__pb_cmd"
    CURSOR=${{#__pb_cmd}}
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
  BUFFER="${{BUFFER:0:$CURSOR}}${{__pb_cmd}}${{BUFFER:$CURSOR}}"
  CURSOR=$(( CURSOR + ${{#__pb_cmd}} ))
  zle reset-prompt
}}
zle -N __pb_insert_command
bindkey "{binding}" __pb_insert_command
_pb_complete() {{
  if [[ "${{words[CURRENT-1]}}" == "--theme" ]]; then
    local -a candidates
    candidates=( ${{(f)"$({executable} complete-theme "${{words[CURRENT]}}")"}} )
    compadd -- "${{candidates[@]}}"
  elif [[ "${{words[2]}}" == "edit" ]]; then
    local -a candidates
    candidates=( ${{(f)"$({executable} complete-edit "${{words[CURRENT]}}")"}} )
    compadd -S '' -- "${{candidates[@]}}"
  elif [[ "${{words[2]}}" == "completions" ]]; then
    compadd -- {SHELL_TARGETS}
  else
    compadd -- --theme {TOP_LEVEL_COMMANDS}
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
    let replace_code = REPLACE_BUFFER_EXIT_CODE;
    // The complete-edit helper is extracted into a named function so the
    // single-quoted executable path doesn't conflict with fish's -a '...' quoting.
    Ok(format!(
        r#"function __pb_insert_command
  set -lx PEANUTBUTTER_BUFFER (commandline)
  set -l __pb_cmd ({executable} execute)
  set -l __pb_status $status
  if test $__pb_status -eq {replace_code}
    commandline -r -- $__pb_cmd
  else if test $__pb_status -eq 0 -a -n "$__pb_cmd"
    commandline -i -- $__pb_cmd
  end
  commandline -f repaint
end
function __pb_complete_edit
  {executable} complete-edit (commandline -ct)
end
function __pb_complete_theme
  {executable} complete-theme (commandline -ct)
end
bind {binding} __pb_insert_command
function __pb_dispatch
  if test (count $argv) -gt 0; and test $argv[1] = "new"
    set -l __pb_hist (history --max=50 | string match -vr '^\s*(pb|peanutbutter)(\s|$)' | string join \x1f)
    set -lx PEANUTBUTTER_HISTORY $__pb_hist
    command {executable} $argv
  else
    command {executable} $argv
  end
end
function {BASH_ALIAS_NAME}
  __pb_dispatch $argv
end
function {BINARY_NAME}
  __pb_dispatch $argv
end
complete -c {BINARY_NAME} -f -l theme -a '(__pb_complete_theme)'
complete -c {BINARY_NAME} -f -n 'not __fish_seen_subcommand_from {TOP_LEVEL_COMMANDS}' -a '{TOP_LEVEL_COMMANDS}'
complete -c {BINARY_NAME} -f -n '__fish_seen_subcommand_from edit' -a '(__pb_complete_edit)'
complete -c {BINARY_NAME} -f -n '__fish_seen_subcommand_from completions' -a '{SHELL_TARGETS}'
complete -c {BASH_ALIAS_NAME} -w {BINARY_NAME}
"#
    ))
}

/// Emit the PowerShell integration script using the path of the currently running
/// executable. Intended for `peanutbutter completions powershell`; the caller should add the
/// output to their PowerShell profile.
pub fn powershell_integration_for_current_exe(binding: &str) -> io::Result<String> {
    let exe = env::current_exe()?;
    powershell_integration_script(binding, &exe)
}

/// Build the PowerShell integration script for a given `executable` path and
/// PSReadLine `binding` (e.g. `"C+b"`). Separated from
/// [`powershell_integration_for_current_exe`] so tests can supply a controlled path.
pub fn powershell_integration_script(binding: &str, executable: &Path) -> io::Result<String> {
    let binding = powershell_binding(binding)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let executable = powershell_quote(&executable.to_string_lossy());
    Ok(format!(
        r#"$script:__pb_exe = {executable}
function __pb_dispatch {{
  if ($args.Count -gt 0 -and $args[0] -eq 'new') {{
    $oldHistory = $env:PEANUTBUTTER_HISTORY
    $env:PEANUTBUTTER_HISTORY = (Get-History -Count 50 | Sort-Object Id -Descending | ForEach-Object CommandLine | Where-Object {{ $_ -notmatch '^\s*(pb|peanutbutter)(\s|$)' }} | Select-Object -First 50) -join [char]31
    try {{ & $script:__pb_exe @args }} finally {{
      if ($null -eq $oldHistory) {{ Remove-Item Env:\PEANUTBUTTER_HISTORY -ErrorAction SilentlyContinue }} else {{ $env:PEANUTBUTTER_HISTORY = $oldHistory }}
    }}
  }} else {{
    & $script:__pb_exe @args
  }}
}}
function {BASH_ALIAS_NAME} {{ __pb_dispatch @args }}
function {BINARY_NAME} {{ __pb_dispatch @args }}
Set-PSReadLineKeyHandler -Chord '{binding}' -ScriptBlock {{
  $cmd = (& $script:__pb_exe execute) -join [Environment]::NewLine
  if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrEmpty($cmd)) {{ return }}
  [Microsoft.PowerShell.PSConsoleReadLine]::Insert($cmd)
}}
Register-ArgumentCompleter -CommandName {BINARY_NAME},{BASH_ALIAS_NAME} -ScriptBlock {{
  param($wordToComplete, $commandAst, $cursorPosition)
  $words = @($commandAst.CommandElements | ForEach-Object {{ $_.Extent.Text }})
  if ($words.Count -ge 2 -and ($words[$words.Count - 1] -eq '--theme' -or $words[$words.Count - 2] -eq '--theme')) {{
    & $script:__pb_exe complete-theme $wordToComplete | ForEach-Object {{ [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }}
  }} elseif ($words.Count -ge 2 -and $words[1] -eq 'edit') {{
    & $script:__pb_exe complete-edit $wordToComplete | ForEach-Object {{ [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }}
  }} elseif ($words.Count -ge 2 -and $words[1] -eq 'completions') {{
    {POWERSHELL_SHELL_TARGETS} | Where-Object {{ $_ -like "$wordToComplete*" }} | ForEach-Object {{ [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }}
  }} else {{
    {POWERSHELL_TOP_LEVEL_COMMANDS} | Where-Object {{ $_ -like "$wordToComplete*" }} | ForEach-Object {{ [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }}
  }}
}}
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

fn powershell_binding(binding: &str) -> Result<String, String> {
    parse_ctrl_key(binding).map(|ch| format!("Ctrl+{}", ch.to_ascii_lowercase()))
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
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
        assert!(script.contains("pb() {"));
        assert!(script.contains("if [[ \"$1\" == \"new\" ]]"));
        assert!(script.contains("PEANUTBUTTER_HISTORY="));
        assert!(script.contains("command '/tmp/peanutbutter' \"$@\""));
        assert!(script.contains("bind -x '\"\\C-b\":__pb_insert_command'"));
        assert!(script.contains("'/tmp/peanutbutter' execute"));
        assert!(script.contains("READLINE_LINE=\"${READLINE_LINE}\""));
        assert!(script.contains("READLINE_POINT=${READLINE_POINT}"));
        // Buffer is passed as a one-shot command-prefix assignment (no bare
        // export that would leak into the interactive shell), and the replace
        // exit code drives a whole-line replacement.
        assert!(
            script.contains("PEANUTBUTTER_BUFFER=\"$READLINE_LINE\" '/tmp/peanutbutter' execute")
        );
        assert!(!script.contains("export PEANUTBUTTER_BUFFER"));
        assert!(script.contains(&format!(
            "if [[ $__pb_status -eq {REPLACE_BUFFER_EXIT_CODE} ]]"
        )));
        assert!(script.contains("READLINE_LINE=\"$__pb_cmd\""));
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
        assert!(
            script.contains("--theme execute init edit new completions lint gc stats settings lsp")
        );
        assert!(script.contains("compgen -W \"bash zsh fish powershell\""));
        assert!(script.contains("complete-theme \"$cur\""));
        assert!(script.contains("--theme"));
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
        assert!(script.contains("PEANUTBUTTER_BUFFER=\"$BUFFER\" '/tmp/peanutbutter' execute"));
        assert!(!script.contains("export PEANUTBUTTER_BUFFER"));
        assert!(script.contains(&format!(
            "if (( __pb_status == {REPLACE_BUFFER_EXIT_CODE} ))"
        )));
        assert!(script.contains("BUFFER=\"$__pb_cmd\""));
    }

    #[test]
    fn zsh_script_emits_pb_alias_and_compdef() {
        let script = zsh_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("pb() {"));
        assert!(script.contains("PEANUTBUTTER_HISTORY="));
        assert!(script.contains("command '/tmp/peanutbutter' \"$@\""));
        assert!(script.contains("compdef _pb_complete peanutbutter pb"));
        assert!(script.contains("'/tmp/peanutbutter' complete-edit"));
        assert!(script.contains("'/tmp/peanutbutter' complete-theme"));
        assert!(script.contains(
            "compadd -- --theme execute init edit new completions lint gc stats settings lsp"
        ));
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
        // Function-local export (`set -lx`) so the var does not persist after
        // the binding returns; replace path uses `commandline -r`.
        assert!(script.contains("set -lx PEANUTBUTTER_BUFFER (commandline)"));
        assert!(script.contains(&format!("test $__pb_status -eq {REPLACE_BUFFER_EXIT_CODE}")));
        assert!(script.contains("commandline -r -- $__pb_cmd"));
    }

    #[test]
    fn fish_script_emits_pb_alias_and_completions() {
        let script = fish_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("function pb"));
        assert!(script.contains("PEANUTBUTTER_HISTORY"));
        assert!(script.contains("command '/tmp/peanutbutter' $argv"));
        assert!(script.contains("complete -c peanutbutter"));
        assert!(script.contains("complete -c pb -w peanutbutter"));
        // complete-edit is called from a helper function to avoid single-quote nesting
        assert!(script.contains("__pb_complete_edit"));
        assert!(script.contains("'/tmp/peanutbutter' complete-edit"));
        assert!(script.contains("__pb_complete_theme"));
        assert!(script.contains("complete -c peanutbutter -f -l theme"));
        assert!(
            script.contains("-a 'execute init edit new completions lint gc stats settings lsp'")
        );
    }

    #[test]
    fn bash_pb_function_harvests_history_for_new() {
        let script = bash_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("__pb_dispatch()"));
        assert!(script.contains("pb() { __pb_dispatch \"$@\"; }"));
        assert!(script.contains("peanutbutter() { __pb_dispatch \"$@\"; }"));
        assert!(script.contains("$1\" == \"new\""));
        assert!(script.contains("fc -lnr 1 50"));
        assert!(script.contains("PEANUTBUTTER_HISTORY="));
        assert!(script.contains("\\037"));
    }

    #[test]
    fn zsh_pb_function_harvests_history_for_new() {
        let script = zsh_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("__pb_dispatch()"));
        assert!(script.contains("pb() { __pb_dispatch \"$@\"; }"));
        assert!(script.contains("peanutbutter() { __pb_dispatch \"$@\"; }"));
        assert!(script.contains("$1\" == \"new\""));
        assert!(script.contains("fc -lnr -50"));
        assert!(script.contains("PEANUTBUTTER_HISTORY="));
    }

    #[test]
    fn fish_pb_function_harvests_history_for_new() {
        let script = fish_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("function __pb_dispatch"));
        assert!(script.contains("function pb"));
        assert!(script.contains("function peanutbutter"));
        assert!(script.contains("history --max=50"));
        assert!(script.contains("PEANUTBUTTER_HISTORY"));
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

    #[test]
    fn powershell_script_registers_psreadline_insert_handler() {
        let script =
            powershell_integration_script("C+b", Path::new("C:/Tools/peanutbutter.exe")).unwrap();
        assert!(script.contains("Set-PSReadLineKeyHandler -Chord 'Ctrl+b'"));
        assert!(script.contains("[Microsoft.PowerShell.PSConsoleReadLine]::Insert($cmd)"));
        assert!(script.contains("(& $script:__pb_exe execute) -join [Environment]::NewLine"));
        assert!(script.contains("$script:__pb_exe = 'C:/Tools/peanutbutter.exe'"));
    }

    #[test]
    fn powershell_script_emits_pb_alias_history_and_completions() {
        let script = powershell_integration_script("C+b", Path::new("C:/pb.exe")).unwrap();
        assert!(script.contains("function pb { __pb_dispatch @args }"));
        assert!(script.contains("function peanutbutter { __pb_dispatch @args }"));
        assert!(script.contains("Get-History -Count 50"));
        assert!(script.contains("PEANUTBUTTER_HISTORY"));
        assert!(script.contains("Register-ArgumentCompleter -CommandName peanutbutter,pb"));
        assert!(script.contains("complete-edit $wordToComplete"));
        assert!(script.contains("complete-theme $wordToComplete"));
        assert!(script.contains("'execute','init','edit','new','completions','lint','gc','stats','settings','lsp','--theme'"));
        assert!(script.contains("'bash','zsh','fish','powershell'"));
    }

    #[test]
    fn powershell_script_uses_requested_binding() {
        let script = powershell_integration_script("C+f", Path::new("C:/pb.exe")).unwrap();
        assert!(script.contains("Set-PSReadLineKeyHandler -Chord 'Ctrl+f'"));
    }

    #[test]
    fn powershell_script_rejects_invalid_binding() {
        assert!(powershell_integration_script("notabinding", Path::new("C:/pb.exe")).is_err());
    }
}
