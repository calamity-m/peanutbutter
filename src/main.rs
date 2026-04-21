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
    let args = std::env::args_os().skip(1);
    let command = match cli::parse_args(args) {
        Ok(command) => command,
        Err(err) => {
            eprintln!("{BINARY_NAME}: {err}");
            eprintln!();
            eprint!("{}", cli::help_text(&paths));
            std::process::exit(2);
        }
    };
    let is_execute = matches!(&command, cli::CliCommand::Execute);

    let result = match command {
        cli::CliCommand::Help => {
            print!("{}", cli::help_text(&paths));
            Ok(())
        }
        cli::CliCommand::Execute => {
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
        cli::CliCommand::Bash { binding } => {
            match cli::bash_integration_for_current_exe(&binding) {
                Ok(script) => {
                    print!("{script}");
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        cli::CliCommand::Add(path) => cli::run_add_command(&paths, path.as_deref()).map(|_| ()),
        cli::CliCommand::Del(query) => match cli::run_del_command(&paths, &query) {
            Ok(deleted) => {
                eprintln!("deleted {}", deleted.id);
                Ok(())
            }
            Err(err) => Err(err),
        },
    };

    if let Err(err) = result {
        eprintln!("{BINARY_NAME}: {err}");
        if is_execute {
            eprintln!("{BINARY_NAME}: execute failed");
        }
        std::process::exit(1);
    }
}
