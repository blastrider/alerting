#[path = "alerting/app.rs"]
mod app;
#[path = "alerting/cli.rs"]
mod cli;
#[path = "alerting/notifier/mod.rs"]
mod notifier;
#[path = "alerting/rate_limit.rs"]
mod rate_limit;

use std::error::Error as StdError;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = cli::Cli::parse_args();
    match app::run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            report_error(&err);
            std::process::ExitCode::from(1)
        }
    }
}

fn report_error(err: &alerting::error::Error) {
    eprintln!("Error: {err}");
    let mut source: Option<&dyn StdError> = err.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
}
