//! Taypeer's command client and private database-worker entry point.

mod args;
mod host;
mod input;
mod output;
mod secret_input;
mod session;

use args::{Action, Cli};
use clap::Parser;
use host::Host;
use input::Input;
use output::{CliError, print_error, print_result};

fn main() {
    let cli = Cli::parse();
    if matches!(cli.command, Action::Worker) {
        let result = taypeer_runtime::run_worker(
            &mut std::io::stdin().lock(),
            &mut std::io::stdout().lock(),
        );
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    let language = cli.lang;
    let json = cli.json;
    if let Err(error) = run(cli) {
        print_error(&error, json, language);
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    let mut host = Host::new(Input {
        password_stdin: cli.password_stdin,
        language: cli.lang,
    })?;
    if let Some(path) = cli.file {
        host.open(&path, None)?;
    }
    if matches!(cli.command, Action::Session) {
        let result = session::run(&mut host, cli.json, cli.lang);
        return result.and(host.close_all());
    }
    let result = host.execute(cli.command);
    let closed = host.close_all();
    match result {
        Ok(mut value) => {
            if let Err(error) = closed {
                taypeer_runtime::erase_view(&mut value);
                return Err(error);
            }
            print_result(value, cli.json, cli.lang)
        }
        Err(error) => Err(error),
    }
}
