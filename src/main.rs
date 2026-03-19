mod clipboard;
mod cmd;
mod config;
mod daemon;
mod error;
mod transport;
mod types;

use error::AppResult;

fn main() {
    if let Err(err) = run() {
        eprintln!("tailsnip: {err}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    cmd::run()
}
