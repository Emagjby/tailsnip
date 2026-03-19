use crate::error::{AppError, AppResult};

pub fn run(target: &str) -> AppResult<()> {
    Err(AppError::Message(format!(
        "send not implemented yet for target: {target}"
    )))
}
