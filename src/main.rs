mod clipboard;
mod cmd;
mod config;
mod daemon;
mod error;
mod transport;
mod types;

use error::AppResult;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("tailsnip: {err}");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    cmd::run().await
}
