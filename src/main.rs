use clap::{CommandFactory, FromArgMatches};
use owo_colors::OwoColorize;
use peanutbutter::BINARY_NAME;
use peanutbutter::cli;
use peanutbutter::completions;
use peanutbutter::config;
use std::fmt;
use std::io::{self, IsTerminal, Write};

fn raw_theme_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--theme" {
            return args.next();
        }
        if let Some(value) = arg.strip_prefix("--theme=") {
            return Some(value.to_string());
        }
    }
    None
}

fn format_error_message(message: &str, color: bool) -> String {
    if !color {
        return message.to_string();
    }

    let Some((prefix, names)) = message.split_once("expected one of: ") else {
        return message.to_string();
    };
    let names = names
        .split(", ")
        .map(|name| name.bold().cyan().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{prefix}expected one of: {names}")
}

fn print_error(err: impl fmt::Display) {
    let color = std::env::var_os("NO_COLOR").is_none() && io::stderr().is_terminal();
    eprintln!(
        "{BINARY_NAME}: {}",
        format_error_message(&err.to_string(), color)
    );
}

/// Print embedded reference docs to stdout and return the process exit code.
///
/// A broken pipe (e.g. `pb docs syntax | head`) exits quietly with 0 rather than
/// printing a backtrace to stderr, keeping a captured fd 1 clean for an LLM.
fn run_docs(topic: Option<peanutbutter::docs::Topic>) -> i32 {
    let mut stdout = io::stdout();
    match peanutbutter::docs::run(topic, &mut stdout).and_then(|()| stdout.flush()) {
        Ok(()) => 0,
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(err) => {
            print_error(err);
            1
        }
    }
}

fn main() {
    // Load config up front but don't abort yet. Help/version output, the bare
    // `pb` help screen, and `docs` must all work even when the config is
    // unparseable — that is exactly when someone needs `pb --help` or
    // `pb docs config` to recover. The config error is surfaced later, and only
    // for commands that actually need config.
    let raw_theme = raw_theme_arg();
    let config_result = config::load_with_theme_override(raw_theme.as_deref());

    // Attach the dynamic snippet-root/path help only when config loaded; clap's
    // built-in help still renders without it when config is broken.
    let mut clap_command = cli::Cli::command();
    if let Ok(config) = &config_result {
        clap_command = clap_command.after_help(cli::after_help(&config.paths));
    }
    // `get_matches` prints `--help`/`--version` to stdout and exits 0, or prints
    // a parse error and exits 2 — all before any config requirement applies.
    let matches = clap_command.clone().get_matches();
    let cli = cli::Cli::from_arg_matches(&matches).unwrap_or_else(|err| err.exit());
    let theme_name = cli.theme.as_deref();
    let command = match cli.command {
        Some(command) => command,
        None if theme_name.is_some() => cli::Command::Execute,
        None => {
            clap_command.print_help().unwrap_or_else(|err| {
                print_error(err);
                std::process::exit(1);
            });
            println!();
            std::process::exit(0);
        }
    };

    // `docs` is the one real command that needs no config; dispatch it before
    // the config error is enforced.
    if let cli::Command::Docs { topic } = command {
        std::process::exit(run_docs(topic));
    }

    let app_config = match config_result {
        Ok(config) => config,
        Err(err) => {
            print_error(err);
            let code = if matches!(command, cli::Command::Lint { .. }) {
                2
            } else {
                1
            };
            std::process::exit(code);
        }
    };
    let paths = app_config.paths.clone();
    let is_execute = matches!(&command, cli::Command::Execute | cli::Command::History);

    let result = match command {
        cli::Command::Execute => {
            let mut stdout = io::stdout();
            let result = cli::run_execute_command(&paths, &mut stdout, theme_name);
            let _ = stdout.flush();
            match result {
                Ok(result) => {
                    if let Some(warning) = result.persist_warning {
                        eprintln!("{BINARY_NAME}: warning: could not save frecency: {warning}");
                    }
                    if result.replace_buffer {
                        std::process::exit(peanutbutter::REPLACE_BUFFER_EXIT_CODE);
                    }
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        // EXPERIMENTAL ctrl-r trial: same stdout/exit-code contract as Execute.
        cli::Command::History => {
            let mut stdout = io::stdout();
            let result = cli::run_history_command(&paths, &mut stdout, theme_name);
            let _ = stdout.flush();
            match result {
                Ok(result) => {
                    if let Some(warning) = result.persist_warning {
                        eprintln!("{BINARY_NAME}: warning: could not save frecency: {warning}");
                    }
                    if result.replace_buffer {
                        std::process::exit(peanutbutter::REPLACE_BUFFER_EXIT_CODE);
                    }
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        cli::Command::Init { force } => {
            let mut stdout = io::stdout();
            let result = cli::run_init_command(&paths, force, &mut stdout);
            let _ = stdout.flush();
            result
        }
        cli::Command::Completions { shell, binding } => {
            let script = match shell {
                completions::Shell::Bash => completions::bash_integration_for_current_exe(&binding),
                completions::Shell::Zsh => completions::zsh_integration_for_current_exe(&binding),
                completions::Shell::Fish => completions::fish_integration_for_current_exe(&binding),
                completions::Shell::Powershell => {
                    completions::powershell_integration_for_current_exe(&binding)
                }
            };
            match script {
                Ok(script) => {
                    print!("{script}");
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        cli::Command::Edit { path } => {
            peanutbutter::edit::run_edit_command(&paths, path.as_deref()).map(|_| ())
        }
        cli::Command::New { name, command } => {
            match peanutbutter::new::run_new_command(&app_config, name, command) {
                Ok(()) => Ok(()),
                Err(err) => {
                    print_error(err);
                    std::process::exit(2);
                }
            }
        }
        cli::Command::Lint { strict, json } => {
            let mut stdout = io::stdout();
            match peanutbutter::lint::run(
                &app_config,
                peanutbutter::lint::LintOptions { strict, json },
                &mut stdout,
            ) {
                Ok(result) => {
                    let _ = stdout.flush();
                    if result.has_findings() {
                        std::process::exit(1);
                    }
                    Ok(())
                }
                Err(err) => {
                    print_error(err);
                    std::process::exit(2);
                }
            }
        }
        cli::Command::Gc {
            dry_run,
            purge,
            quiet,
        } => {
            let mut stdout = io::stdout();
            peanutbutter::gc::run(
                &paths,
                peanutbutter::gc::GcOptions {
                    dry_run,
                    purge,
                    quiet,
                },
                &mut stdout,
            )
            .map(|_| ())
        }
        cli::Command::Stats { top, sort, output } => {
            let mut stdout = io::stdout();
            peanutbutter::stats::run(
                &paths,
                peanutbutter::stats::StatsOptions {
                    top_n: top,
                    sort,
                    output,
                    theme: app_config.theme.clone(),
                },
                &mut stdout,
            )
        }
        cli::Command::Settings => peanutbutter::settings::run(&app_config),
        cli::Command::Repo => peanutbutter::repo::run(&app_config),
        cli::Command::CompleteEdit { current } => {
            match peanutbutter::edit::complete_edit(&paths, current.as_deref().unwrap_or("")) {
                Ok(candidates) => {
                    for candidate in candidates {
                        println!("{candidate}");
                    }
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        cli::Command::CompleteTheme { current } => match config::theme_completion_names() {
            Ok(candidates) => {
                let current = current.unwrap_or_default();
                for candidate in candidates {
                    if candidate.starts_with(&current) {
                        println!("{candidate}");
                    }
                }
                Ok(())
            }
            Err(err) => Err(err),
        },
        cli::Command::Lsp => {
            peanutbutter::lsp::run_lsp_server();
            Ok(())
        }
        // Normally handled by the early, config-free dispatch in `main`; kept
        // here for exhaustiveness and as a fallback.
        cli::Command::Docs { topic } => std::process::exit(run_docs(topic)),
    };

    if let Err(err) = result {
        print_error(err);
        if is_execute {
            eprintln!("{BINARY_NAME}: execute failed");
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_error_message_colours_theme_names_only_when_enabled() {
        let raw = "unknown theme invalid; expected one of: default, gruvbox";

        assert_eq!(format_error_message(raw, false), raw);
        let colored = format_error_message(raw, true);

        assert!(colored.contains("unknown theme invalid; expected one of: "));
        assert!(colored.contains("\u{1b}["));
        assert!(colored.contains("default"));
        assert!(colored.contains("gruvbox"));
    }
}
