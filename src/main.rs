#![forbid(unsafe_code)]

use learn::{commands, config::Config, error::Result, state::AppState, web, writer};
use std::process::ExitCode;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();
    fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("learn=info".parse().expect("valid log directive"))
                .add_directive(
                    "sqlx::postgres::notice=warn"
                        .parse()
                        .expect("valid log directive"),
                ),
        )
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => serve().await,
        Some("configcheck") => Config::from_env().map(|_| ()),
        Some("help" | "-h" | "--help") => {
            println!("{}", commands::USAGE);
            Ok(())
        }
        Some(command) => {
            let state = AppState::new(Config::from_env()?).await?;
            println!("{}", commands::run(&state, command, &args[1..]).await?);
            Ok(())
        }
    }
}

/// The site, and XABC writing on its schedule unless `LEARN_WRITER` is off.
async fn serve() -> Result<()> {
    let state = AppState::new(Config::from_env()?).await?;
    let schedule = state.writes().then(|| writer::spawn(state.clone()));
    let result = web::serve(state.clone()).await;
    state.shutdown_workers().await;
    if let Some(schedule) = schedule {
        let _ = schedule.await;
    }
    result
}
