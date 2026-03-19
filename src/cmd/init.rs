use crate::{config, error::AppResult};

pub fn run() -> AppResult<()> {
    let path = config::ensure_template_config()?;
    println!("created config at {}", path.display());
    Ok(())
}
