use crate::{
    clipboard::{Clipboard, ClipboardBackend},
    config::Config,
    error::{AppError, AppResult},
    transport::TransportClient,
};

pub async fn run(target: &str) -> AppResult<()> {
    let config = Config::load()?;
    run_with_config(target, &config).await
}

async fn run_with_config(target: &str, config: &Config) -> AppResult<()> {
    let address = config.resolve_device(target)?;
    let text = read_local_clipboard().await?;

    let client = TransportClient::new(address, config.token.clone())?;
    client.write_clipboard(&text).await?;

    println!("sent clipboard to {target}");
    Ok(())
}

async fn read_local_clipboard() -> AppResult<String> {
    let clipboard = if cfg!(target_os = "macos") {
        Clipboard::from_backend(ClipboardBackend::MacOsPbcopy)
    } else {
        Clipboard::detect()?
    };

    tokio::task::spawn_blocking(move || clipboard.read_text())
        .await
        .map_err(|e| AppError::Message(format!("clipboard read task failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        net::SocketAddr,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode, header::AUTHORIZATION},
        response::IntoResponse,
        routing::post,
    };
    use serial_test::serial;
    use tokio::net::TcpListener;

    use crate::{
        config::DaemonConfig,
        types::{ApiErrorResponse, ApiStatusResponse, ClipboardWriteRequest},
    };

    use super::*;

    #[derive(Clone)]
    struct WriteServerState {
        expected_token: String,
        received_text: Arc<Mutex<Vec<String>>>,
    }

    struct PathGuard {
        original: Option<std::ffi::OsString>,
    }

    impl Drop for PathGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(path) => unsafe { std::env::set_var("PATH", path) },
                None => unsafe { std::env::remove_var("PATH") },
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn sends_local_clipboard_to_remote_peer() {
        let temp_dir = tempfile::tempdir().unwrap();
        install_script(
            temp_dir.path(),
            "pbpaste",
            "#!/bin/sh\nprintf 'hello from clipboard'",
        )
        .unwrap();
        let _path_guard = prepend_path(temp_dir.path());

        let received_text = Arc::new(Mutex::new(Vec::new()));
        let state = WriteServerState {
            expected_token: "secret-token".into(),
            received_text: received_text.clone(),
        };

        let (address, _shutdown) = spawn_write_server(state).await;
        let config = test_config(address);

        run_with_config("macbook", &config).await.unwrap();

        assert_eq!(
            received_text.lock().unwrap().as_slice(),
            ["hello from clipboard"]
        );
    }

    #[tokio::test]
    async fn rejects_unknown_target_alias() {
        let config = test_config("127.0.0.1:3947".parse().unwrap());
        let err = run_with_config("missing", &config).await.unwrap_err();

        assert!(matches!(err, AppError::DeviceNotFound(alias) if alias == "missing"));
    }

    #[tokio::test]
    #[serial]
    async fn surfaces_unreachable_remote_peer() {
        let temp_dir = tempfile::tempdir().unwrap();
        install_script(
            temp_dir.path(),
            "pbpaste",
            "#!/bin/sh\nprintf 'hello from clipboard'",
        )
        .unwrap();
        let _path_guard = prepend_path(temp_dir.path());

        let address = reserve_unused_local_address().await;
        let config = test_config(address);

        let err = run_with_config("macbook", &config).await.unwrap_err();

        assert!(
            matches!(err, AppError::TransportUnreachable(message) if message.contains("connection failed"))
        );
    }

    fn test_config(address: SocketAddr) -> Config {
        let mut devices = BTreeMap::new();
        devices.insert("macbook".into(), address);

        Config {
            token: "secret-token".into(),
            daemon: DaemonConfig {
                listen: "127.0.0.1:3947".parse().unwrap(),
            },
            devices,
        }
    }

    async fn spawn_write_server(
        state: WriteServerState,
    ) -> (SocketAddr, tokio::sync::oneshot::Sender<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let app = Router::new()
            .route("/clipboard/write", post(handle_write_request))
            .with_state(state);

        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        (address, shutdown_tx)
    }

    async fn handle_write_request(
        State(state): State<WriteServerState>,
        headers: HeaderMap,
        Json(payload): Json<ClipboardWriteRequest>,
    ) -> impl IntoResponse {
        let auth = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok());

        if auth != Some(&format!("Bearer {}", state.expected_token)) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    ok: false,
                    error: "invalid authentication token".into(),
                }),
            )
                .into_response();
        }

        state.received_text.lock().unwrap().push(payload.text);

        (
            StatusCode::OK,
            Json(ApiStatusResponse {
                ok: true,
                message: "clipboard updated".into(),
            }),
        )
            .into_response()
    }

    async fn reserve_unused_local_address() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    }

    fn prepend_path(dir: &Path) -> PathGuard {
        let original = std::env::var_os("PATH");
        let mut paths = vec![PathBuf::from(dir)];
        if let Some(existing) = &original {
            paths.extend(std::env::split_paths(existing));
        }

        let joined = std::env::join_paths(paths).unwrap();
        unsafe { std::env::set_var("PATH", joined) };

        PathGuard { original }
    }

    fn install_script(dir: &Path, name: &str, contents: &str) -> std::io::Result<()> {
        let path = dir.join(name);
        fs::write(&path, contents)?;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
    }
}
