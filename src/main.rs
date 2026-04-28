use clap::{CommandFactory, FromArgMatches};
use peanutbutter::BINARY_NAME;
use peanutbutter::cli;
use peanutbutter::config;
use std::io::{self, Write};

fn main() {
    let app_config = match config::load() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{BINARY_NAME}: {err}");
            std::process::exit(1);
        }
    };
    let paths = app_config.paths.clone();
    let mut clap_command = cli::Cli::command().after_help(cli::after_help(&paths));
    let matches = clap_command.clone().get_matches();
    let cli = cli::Cli::from_arg_matches(&matches).unwrap_or_else(|err| err.exit());
    let Some(command) = cli.command else {
        clap_command.print_help().unwrap_or_else(|err| {
            eprintln!("{BINARY_NAME}: {err}");
            std::process::exit(1);
        });
        println!();
        std::process::exit(0);
    };
    let is_execute = matches!(&command, cli::Command::Execute);

    let result = match command {
        cli::Command::Execute => {
            let mut stdout = io::stdout();
            let result = cli::run_execute_command(&paths, &mut stdout);
            let _ = stdout.flush();
            match result {
                Ok(result) => {
                    if let Some(warning) = result.persist_warning {
                        eprintln!("{BINARY_NAME}: warning: could not save frecency: {warning}");
                    }
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        cli::Command::Bash { binding } => match cli::bash_integration_for_current_exe(&binding) {
            Ok(script) => {
                print!("{script}");
                Ok(())
            }
            Err(err) => Err(err),
        },
        cli::Command::Edit { path } => cli::run_edit_command(&paths, path.as_deref()).map(|_| ()),
        cli::Command::CompleteEdit { current } => {
            match cli::complete_edit(&paths, current.as_deref().unwrap_or("")) {
                Ok(candidates) => {
                    for candidate in candidates {
                        println!("{candidate}");
                    }
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
    };

    if let Err(err) = result {
        eprintln!("{BINARY_NAME}: {err}");
        if is_execute {
            eprintln!("{BINARY_NAME}: execute failed");
        }
        std::process::exit(1);
    }
}
