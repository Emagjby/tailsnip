use crate::{config::Config, daemon, error::AppResult};

pub async fn run() -> AppResult<()> {
    let config = Config::load()?;
    daemon::serve(&config).await
}
