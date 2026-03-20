use crate::{
    clipboard::{Clipboard, ClipboardBackend},
    config::Config,
    error::{AppError, AppResult},
    transport::TransportClient,
};
use tokio::{
    task,
    time::{Duration, timeout},
};

const GET_REMOTE_TIMEOUT: Duration = Duration::from_secs(5);
const GET_LOCAL_WRITE_TIMEOUT: Duration = Duration::from_secs(4);

pub async fn run(target: &str) -> AppResult<()> {
    let config = Config::load()?;
    let address = config.resolve_device(target)?;

    let client = TransportClient::new(address, config.token)?;
    let text = timeout(GET_REMOTE_TIMEOUT, client.read_clipboard())
        .await
        .map_err(|_| {
            AppError::TransportTimeout(format!(
                "timed out reading clipboard from {target} after {}s",
                GET_REMOTE_TIMEOUT.as_secs()
            ))
        })??;

    if text.is_empty() {
        return Err(AppError::Message(format!(
            "remote clipboard from {target} is empty; refusing to overwrite local clipboard"
        )));
    }

    let write_text = text.clone();
    task::spawn_blocking(move || {
        let clipboard = if cfg!(target_os = "macos") {
            Clipboard::from_backend(ClipboardBackend::MacOsPbcopy)
        } else {
            Clipboard::detect_for_write()?
        };
        clipboard.write_text_with_timeout(&write_text, GET_LOCAL_WRITE_TIMEOUT)
    })
    .await
    .map_err(|err| AppError::ClipboardWrite(format!("clipboard task failed: {err}")))??;

    println!("fetched clipboard from {target}");
    Ok(())
}
