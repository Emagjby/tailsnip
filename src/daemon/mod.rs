use axum::{
    Json, Router,
    extract::{Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use constant_time_eq::constant_time_eq;
use std::sync::Arc;
use std::{future::Future, pin::Pin};
use tokio::sync::Mutex;
use tokio::{net::TcpListener, signal, task, time::Duration};

use crate::{
    clipboard::{Clipboard, ClipboardBackend},
    config::Config,
    error::{AppError, AppResult},
    types::{ApiErrorResponse, ApiStatusResponse, ClipboardReadResponse, ClipboardWriteRequest},
};

#[derive(Debug)]
pub struct DaemonState {
    pub token: String,
    pub clipboard_backend: ClipboardBackend,
    pub wayland_owner: Mutex<Option<std::process::Child>>,
}

const CLIPBOARD_HANDLER_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn serve(config: &Config) -> AppResult<()> {
    serve_with_shutdown(config, shutdown_signal()).await
}

async fn serve_with_shutdown(
    config: &Config,
    shutdown: Pin<Box<dyn Future<Output = ()> + Send>>,
) -> AppResult<()> {
    let clipboard = Clipboard::detect().map_err(|err| {
        AppError::DaemonStartup(format!("clipboard backend initialization failed: {err}"))
    })?;

    let display = std::env::var("DISPLAY").unwrap_or_else(|_| "<unset>".to_string());
    let wayland_display =
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "<unset>".to_string());

    println!(
        "tailsnip clipboard backend: {:?} (DISPLAY={}, WAYLAND_DISPLAY={})",
        clipboard.backend(),
        display,
        wayland_display
    );

    let state = Arc::new(DaemonState {
        token: config.token.clone(),
        clipboard_backend: clipboard.backend(),
        wayland_owner: Mutex::new(None),
    });

    let app = build_router(state.clone());

    let listener = TcpListener::bind(config.daemon.listen)
        .await
        .map_err(|err| {
            AppError::DaemonStartup(format!(
                "failed to bind daemon to {}: {err}",
                config.daemon.listen
            ))
        })?;

    println!("tailsnip daemon listening on {}", config.daemon.listen);

    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|err| {
            AppError::DaemonRuntime(format!(
                "daemon server exited unexpectedly on {}: {err}",
                config.daemon.listen
            ))
        });

    cleanup_wayland_owner(&state).await;

    if result.is_ok() {
        println!("tailsnip daemon shutting down");
    }

    result
}

fn build_router(state: Arc<DaemonState>) -> Router {
    Router::new()
        .route("/clipboard/read", get(handle_read))
        .route("/clipboard/write", post(handle_write))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer_auth,
        ))
        .with_state(state)
}

async fn require_bearer_auth(
    State(state): State<Arc<DaemonState>>,
    request: Request,
    next: Next,
) -> Response {
    let auth_header = request.headers().get(AUTHORIZATION);

    match validate_auth_header(auth_header, &state.token) {
        Ok(()) => next.run(request).await,
        Err(err_response) => *err_response,
    }
}

fn validate_auth_header(
    auth_header: Option<&axum::http::HeaderValue>,
    expected_token: &str,
) -> Result<(), Box<Response>> {
    let provided = match auth_header.and_then(|value| value.to_str().ok()) {
        Some(value) => value,
        None => {
            return Err(Box::new(json_error_response(
                StatusCode::UNAUTHORIZED,
                "missing authorization header",
            )));
        }
    };

    let Some(token) = provided.strip_prefix("Bearer ") else {
        return Err(Box::new(json_error_response(
            StatusCode::UNAUTHORIZED,
            "invalid authorization scheme",
        )));
    };

    if !constant_time_eq(expected_token.as_bytes(), token.as_bytes()) {
        return Err(Box::new(json_error_response(
            StatusCode::UNAUTHORIZED,
            "invalid authentication token",
        )));
    }

    Ok(())
}

async fn handle_read(State(state): State<Arc<DaemonState>>) -> Response {
    match task::spawn_blocking(move || {
        let clipboard = Clipboard::from_backend(state.clipboard_backend);
        clipboard.read_text_with_timeout(CLIPBOARD_HANDLER_TIMEOUT)
    })
    .await
    {
        Ok(Ok(text)) => (StatusCode::OK, Json(ClipboardReadResponse { text })).into_response(),
        Ok(Err(err)) => clipboard_error_response(err, "failed to read clipboard"),
        Err(err) => clipboard_error_response(
            AppError::ClipboardRead(format!("clipboard task failed: {err}")),
            "failed to read clipboard",
        ),
    }
}

async fn handle_write(
    State(state): State<Arc<DaemonState>>,
    Json(payload): Json<ClipboardWriteRequest>,
) -> Response {
    let text = payload.text;

    if state.clipboard_backend == ClipboardBackend::LinuxWayland {
        let spawn_result = task::spawn_blocking(move || {
            let clipboard = Clipboard::from_backend(ClipboardBackend::LinuxWayland);
            clipboard.spawn_wayland_owner(&text)
        })
        .await;

        match spawn_result {
            Ok(Ok(new_child)) => {
                let old_child = {
                    let mut owner_guard = state.wayland_owner.lock().await;
                    let old_child = owner_guard.take();
                    *owner_guard = Some(new_child);
                    old_child
                };

                if let Some(mut old_child) = old_child {
                    let _ = task::spawn_blocking(move || {
                        let _ = old_child.kill();
                        let _ = old_child.wait();
                    })
                    .await;
                }

                return (
                    StatusCode::OK,
                    Json(ApiStatusResponse {
                        ok: true,
                        message: "clipboard updated".into(),
                    }),
                )
                    .into_response();
            }
            Ok(Err(err)) => {
                let (status, message) = clipboard_error_parts(err, "failed to write to clipboard");
                return (
                    status,
                    Json(ApiErrorResponse {
                        ok: false,
                        error: message,
                    }),
                )
                    .into_response();
            }
            Err(err) => {
                let (status, message) = clipboard_error_parts(
                    AppError::ClipboardWrite(format!("clipboard task failed: {err}")),
                    "failed to write to clipboard",
                );
                return (
                    status,
                    Json(ApiErrorResponse {
                        ok: false,
                        error: message,
                    }),
                )
                    .into_response();
            }
        }
    }

    match task::spawn_blocking(move || {
        let clipboard = Clipboard::from_backend(state.clipboard_backend);
        clipboard.write_text_with_timeout(&text, CLIPBOARD_HANDLER_TIMEOUT)
    })
    .await
    {
        Ok(Ok(())) => (
            StatusCode::OK,
            Json(ApiStatusResponse {
                ok: true,
                message: "clipboard updated".into(),
            }),
        )
            .into_response(),
        Ok(Err(err)) => {
            let (status, message) = clipboard_error_parts(err, "failed to write to clipboard");
            (
                status,
                Json(ApiErrorResponse {
                    ok: false,
                    error: message,
                }),
            )
                .into_response()
        }
        Err(err) => {
            let (status, message) = clipboard_error_parts(
                AppError::ClipboardWrite(format!("clipboard task failed: {err}")),
                "failed to write to clipboard",
            );
            (
                status,
                Json(ApiErrorResponse {
                    ok: false,
                    error: message,
                }),
            )
                .into_response()
        }
    }
}

fn clipboard_error_response(err: AppError, default_message: &str) -> Response {
    let (status, message) = clipboard_error_parts(err, default_message);
    (
        status,
        Json(ApiErrorResponse {
            ok: false,
            error: message,
        }),
    )
        .into_response()
}

fn clipboard_error_parts(err: AppError, default_message: &str) -> (StatusCode, String) {
    match err {
        AppError::ClipboardUnavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message),
        AppError::ClipboardRead(message) | AppError::ClipboardWrite(message) => {
            (StatusCode::INTERNAL_SERVER_ERROR, message)
        }
        other => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}: {other}", default_message),
        ),
    }
}

fn json_error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ApiErrorResponse {
            ok: false,
            error: message.into(),
        }),
    )
        .into_response()
}

async fn cleanup_wayland_owner(state: &Arc<DaemonState>) {
    let old_child = {
        let mut owner_guard = state.wayland_owner.lock().await;
        owner_guard.take()
    };

    if let Some(mut child) = old_child {
        let _ = task::spawn_blocking(move || {
            let _ = child.kill();
            let _ = child.wait();
        })
        .await;
    }
}

fn shutdown_signal() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    #[cfg(unix)]
    {
        Box::pin(async {
            let mut terminate = signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");

            tokio::select! {
                _ = signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
        })
    }

    #[cfg(not(unix))]
    {
        Box::pin(async {
            let _ = signal::ctrl_c().await;
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    use axum::{
        body::Body,
        http::{Request as HttpRequest, StatusCode},
    };
    use serial_test::serial;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn router_builds_with_registered_routes() {
        let state = Arc::new(DaemonState {
            token: "testtoken".into(),
            clipboard_backend: ClipboardBackend::LinuxXclip,
            wayland_owner: Mutex::new(None),
        });
        let _router = build_router(state);
    }

    #[test]
    fn accepts_valid_bearer_token() {
        let header = HeaderValue::from_str("Bearer secret-token").unwrap();
        let result = validate_auth_header(Some(&header), "secret-token");
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_missing_auth_header() {
        let result = validate_auth_header(None, "secret-token");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_wrong_auth_scheme() {
        let header = HeaderValue::from_str("Basic secret-token").unwrap();
        let result = validate_auth_header(Some(&header), "secret-token");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_wrong_token() {
        let header = HeaderValue::from_str("Bearer wrong-token").unwrap();
        let result = validate_auth_header(Some(&header), "secret-token");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn auth_middleware_rejects_missing_header_for_read_route() {
        let state = Arc::new(DaemonState {
            token: "testtoken".into(),
            clipboard_backend: ClipboardBackend::LinuxXclip,
            wayland_owner: Mutex::new(None),
        });

        let response = build_router(state)
            .oneshot(
                HttpRequest::builder()
                    .uri("/clipboard/read")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn serve_reports_clipboard_startup_failures_as_daemon_startup_errors() {
        let _path_guard = set_path_only(None);

        let config = Config {
            token: "testtoken".into(),
            daemon: crate::config::DaemonConfig {
                listen: "127.0.0.1:0".parse().unwrap(),
            },
            devices: std::collections::BTreeMap::new(),
        };

        let err = serve_with_shutdown(&config, Box::pin(async {}))
            .await
            .unwrap_err();

        assert!(
            matches!(err, AppError::DaemonStartup(message) if message.contains("clipboard backend initialization failed"))
        );
    }

    #[tokio::test]
    #[serial]
    async fn serve_returns_cleanly_after_shutdown_signal() {
        let temp_dir = tempfile::tempdir().unwrap();
        install_fake_clipboard_tools(&temp_dir);
        let _path_guard = set_path_only(Some(temp_dir.path()));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let config = Config {
            token: "testtoken".into(),
            daemon: crate::config::DaemonConfig { listen: address },
            devices: std::collections::BTreeMap::new(),
        };

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            serve_with_shutdown(
                &config,
                Box::pin(async move {
                    let _ = rx.await;
                }),
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = tx.send(());

        assert!(task.await.unwrap().is_ok());
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

    fn set_path_only(path: Option<&Path>) -> PathGuard {
        let original = std::env::var_os("PATH");

        match path {
            Some(path) => unsafe { std::env::set_var("PATH", path) },
            None => unsafe { std::env::remove_var("PATH") },
        }

        PathGuard { original }
    }

    fn install_fake_clipboard_tools(temp_dir: &TempDir) {
        install_script(temp_dir.path(), "pbcopy", "#!/bin/sh\ncat >/dev/null").unwrap();
        install_script(temp_dir.path(), "pbpaste", "#!/bin/sh\nprintf 'stub'").unwrap();
    }

    fn install_script(dir: &Path, name: &str, contents: &str) -> std::io::Result<()> {
        let path = PathBuf::from(dir).join(name);
        fs::write(&path, contents)?;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
    }
}
