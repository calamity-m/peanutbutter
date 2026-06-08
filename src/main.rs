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

fn main() {
    let raw_theme = raw_theme_arg();
    let app_config = match config::load_with_theme_override(raw_theme.as_deref()) {
        Ok(config) => config,
        Err(err) => {
            print_error(err);
            let code = if std::env::args().any(|arg| arg == "lint") {
                2
            } else {
                1
            };
            std::process::exit(code);
        }
    };
    let paths = app_config.paths.clone();
    let mut clap_command = cli::Cli::command().after_help(cli::after_help(&paths));
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
    let is_execute = matches!(&command, cli::Command::Execute);

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
        cli::Command::Init { force } => {
            let mut stdout = io::stdout();
            let result = cli::run_init_command(&paths, force, &mut stdout);
            let _ = stdout.flush();
            result
        }
        cli::Command::Bash { binding } => {
            match completions::bash_integration_for_current_exe(&binding) {
                Ok(script) => {
                    print!("{script}");
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        cli::Command::Zsh { binding } => {
            match completions::zsh_integration_for_current_exe(&binding) {
                Ok(script) => {
                    print!("{script}");
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        cli::Command::Fish { binding } => {
            match completions::fish_integration_for_current_exe(&binding) {
                Ok(script) => {
                    print!("{script}");
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        cli::Command::Powershell { binding } => {
            match completions::powershell_integration_for_current_exe(&binding) {
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
        cli::Command::New { name, command } => match peanutbutter::new::run_new_command(
            &paths,
            &app_config.theme,
            app_config.ui.height,
            name,
            command,
        ) {
            Ok(()) => Ok(()),
            Err(err) => {
                print_error(err);
                std::process::exit(2);
            }
        },
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
        cli::Command::Stats {
            top,
            sort,
            output,
            json,
        } => {
            let mut stdout = io::stdout();
            peanutbutter::stats::run(
                &paths,
                peanutbutter::stats::StatsOptions {
                    top_n: top,
                    sort,
                    output,
                    json,
                },
                &mut stdout,
            )
        }
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
