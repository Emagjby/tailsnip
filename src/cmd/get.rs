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
    run_with_config(target, &config).await
}

async fn run_with_config(target: &str, config: &Config) -> AppResult<()> {
    let address = config.resolve_device(target)?;

    let client = TransportClient::new(address, config.token.clone())?;
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

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        net::SocketAddr,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode, header::AUTHORIZATION},
        response::IntoResponse,
        routing::get,
    };
    use serial_test::serial;
    use tokio::net::TcpListener;

    use crate::{
        config::DaemonConfig,
        types::{ApiErrorResponse, ClipboardReadResponse},
    };

    use super::*;

    #[derive(Clone)]
    struct ReadServerState {
        expected_token: String,
        response_text: String,
        status: StatusCode,
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
    async fn fetches_remote_clipboard_and_writes_locally() {
        let temp_dir = tempfile::tempdir().unwrap();
        let capture_path = temp_dir.path().join("captured.txt");
        install_script(
            temp_dir.path(),
            "pbcopy",
            &format!("#!/bin/sh\n/bin/cat > \"{}\"", capture_path.display()),
        )
        .unwrap();
        let _path_guard = set_path_only(temp_dir.path());

        let (address, _shutdown) = spawn_read_server(ReadServerState {
            expected_token: "secret-token".into(),
            response_text: "hello from peer".into(),
            status: StatusCode::OK,
        })
        .await;
        let config = test_config(address);

        run_with_config("macbook", &config).await.unwrap();

        assert_eq!(fs::read_to_string(capture_path).unwrap(), "hello from peer");
    }

    #[tokio::test]
    async fn rejects_empty_remote_clipboard() {
        let (address, _shutdown) = spawn_read_server(ReadServerState {
            expected_token: "secret-token".into(),
            response_text: String::new(),
            status: StatusCode::OK,
        })
        .await;
        let config = test_config(address);

        let err = run_with_config("macbook", &config).await.unwrap_err();

        assert!(
            matches!(err, AppError::Message(message) if message.contains("remote clipboard from macbook is empty"))
        );
    }

    #[tokio::test]
    #[serial]
    async fn surfaces_remote_auth_failures() {
        let temp_dir = tempfile::tempdir().unwrap();
        install_script(temp_dir.path(), "pbcopy", "#!/bin/sh\n/bin/cat >/dev/null").unwrap();
        let _path_guard = set_path_only(temp_dir.path());

        let (address, _shutdown) = spawn_read_server(ReadServerState {
            expected_token: "different-token".into(),
            response_text: "ignored".into(),
            status: StatusCode::OK,
        })
        .await;
        let config = test_config(address);

        let err = run_with_config("macbook", &config).await.unwrap_err();

        assert!(
            matches!(err, AppError::TransportAuth(message) if message.contains("invalid authentication token"))
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

    async fn spawn_read_server(
        state: ReadServerState,
    ) -> (SocketAddr, tokio::sync::oneshot::Sender<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let app = Router::new()
            .route("/clipboard/read", get(handle_read_request))
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

    async fn handle_read_request(
        State(state): State<ReadServerState>,
        headers: HeaderMap,
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

        if state.status != StatusCode::OK {
            return (
                state.status,
                Json(ApiErrorResponse {
                    ok: false,
                    error: state.response_text,
                }),
            )
                .into_response();
        }

        (
            StatusCode::OK,
            Json(ClipboardReadResponse {
                text: state.response_text,
            }),
        )
            .into_response()
    }

    fn set_path_only(dir: &Path) -> PathGuard {
        let original = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", PathBuf::from(dir)) };

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
