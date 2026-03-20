use crate::{
    clipboard::{Clipboard, ClipboardBackend},
    config::Config,
    error::{AppError, AppResult},
    transport::TransportClient,
};

pub async fn run(target: &str) -> AppResult<()> {
    let config = Config::load()?;
    let address = config.resolve_device(target)?;

    let clipboard = if cfg!(target_os = "macos") {
        Clipboard::from_backend(ClipboardBackend::MacOsPbcopy)
    } else {
        Clipboard::detect()?
    };
    let text = tokio::task::spawn_blocking(move || clipboard.read_text())
        .await
        .map_err(|e| AppError::Message(format!("clipboard read task failed: {e}")))??;

    let client = TransportClient::new(address, config.token)?;
    client.write_clipboard(&text).await?;

    println!("sent clipboard to {target}");
    Ok(())
}
