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
use tokio::{
    net::TcpListener,
    task,
    time::Duration,
};
use tokio::sync::Mutex;

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
    let clipboard = Clipboard::detect()?;

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

    let app = build_router(state);

    let listener = TcpListener::bind(config.daemon.listen)
        .await
        .map_err(|err| {
            AppError::DaemonStartup(format!(
                "failed to bind daemon to {}: {err}",
                config.daemon.listen
            ))
        })?;

    println!("tailsnip daemon listening on {}", config.daemon.listen);

    axum::serve(listener, app).await.map_err(|err| {
        AppError::DaemonRuntime(format!(
            "daemon server exited unexpectedly on {}: {err}",
            config.daemon.listen
        ))
    })
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
        Err(err_response) => err_response,
    }
}

fn validate_auth_header(
    auth_header: Option<&axum::http::HeaderValue>,
    expected_token: &str,
) -> Result<(), Response> {
    let provided = match auth_header.and_then(|value| value.to_str().ok()) {
        Some(value) => value,
        None => {
            return Err(json_error_response(
                StatusCode::UNAUTHORIZED,
                "missing authorization header",
            ));
        }
    };

    let Some(token) = provided.strip_prefix("Bearer ") else {
        return Err(json_error_response(
            StatusCode::UNAUTHORIZED,
            "invalid authorization scheme",
        ));
    };

    if !constant_time_eq(expected_token.as_bytes(), token.as_bytes()) {
        return Err(json_error_response(
            StatusCode::UNAUTHORIZED,
            "invalid authentication token",
        ));
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
                let (status, message) =
                    clipboard_error_parts(err, "failed to write to clipboard");
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

#[cfg(test)]
mod tests {
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
}
