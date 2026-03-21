use std::net::SocketAddr;

use reqwest::{Client, StatusCode};

use crate::{
    error::{AppError, AppResult},
    types::{ApiErrorResponse, ApiStatusResponse, ClipboardReadResponse, ClipboardWriteRequest},
};

const DEFAULT_TIMEOUT_SECS: u64 = 5;
const DEFAULT_SCHEME: &str = "http";

#[derive(Clone)]
pub struct TransportClient {
    base_url: String,
    token: String,
    http: Client,
}

impl std::fmt::Debug for TransportClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransportClient")
            .field("base_url", &self.base_url)
            .field("token", &"<redacted>")
            .field("http", &self.http)
            .finish()
    }
}

impl TransportClient {
    pub fn new(address: SocketAddr, token: impl Into<String>) -> AppResult<Self> {
        let token = token.into();
        let scheme = std::env::var("TAILSNIP_TRANSPORT_SCHEME")
            .ok()
            .unwrap_or_else(|| DEFAULT_SCHEME.to_string());

        Self::new_with_scheme(address, token, &scheme)
    }

    pub fn new_with_scheme(
        address: SocketAddr,
        token: impl Into<String>,
        scheme: &str,
    ) -> AppResult<Self> {
        let normalized_scheme = normalize_scheme(scheme)?;

        if !address.ip().is_loopback() {
            eprintln!(
                "tailsnip warning: non-loopback remote address {} configured; ensure network path and transport security are appropriate",
                address
            );
        }

        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .map_err(|e| AppError::Transport(format!("failed to build http client: {}", e)))?;

        Ok(Self {
            base_url: format!("{}://{}", normalized_scheme, address),
            token: token.into(),
            http,
        })
    }

    pub async fn read_clipboard(&self) -> AppResult<String> {
        let response = self
            .http
            .get(format!("{}/clipboard/read", self.base_url))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(map_request_error)?;

        if response.status().is_success() {
            let payload = response
                .json::<ClipboardReadResponse>()
                .await
                .map_err(|err| {
                    AppError::Transport(format!(
                        "failed to decode clipboard read response: {}",
                        err
                    ))
                })?;

            return Ok(payload.text);
        }

        Err(map_error_response(response).await)
    }

    pub async fn write_clipboard(&self, text: &str) -> AppResult<()> {
        let response = self
            .http
            .post(format!("{}/clipboard/write", self.base_url))
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&ClipboardWriteRequest {
                text: text.to_string(),
            })
            .send()
            .await
            .map_err(map_request_error)?;

        if response.status().is_success() {
            let payload = response.json::<ApiStatusResponse>().await.map_err(|err| {
                AppError::Transport(format!("failed to decode clipboard write response: {err}",))
            })?;

            if payload.ok {
                return Ok(());
            }

            return Err(AppError::RemoteFailure(payload.message));
        }

        Err(map_error_response(response).await)
    }
}

fn normalize_scheme(scheme: &str) -> AppResult<&str> {
    if scheme.eq_ignore_ascii_case("http") {
        return Ok("http");
    }
    if scheme.eq_ignore_ascii_case("https") {
        return Ok("https");
    }

    Err(AppError::Transport(format!(
        "invalid transport scheme '{scheme}'; expected 'http' or 'https'"
    )))
}

fn map_request_error(err: reqwest::Error) -> AppError {
    if err.is_timeout() {
        return AppError::TransportTimeout("request exceeded timeout".to_string());
    }

    if err.is_connect() {
        return AppError::TransportUnreachable(format!("connection failed: {}", err));
    }

    AppError::Transport(format!("request failed: {err}"))
}

async fn map_error_response(response: reqwest::Response) -> AppError {
    let status = response.status();

    let parsed = response.json::<ApiErrorResponse>().await.ok();

    let message = parsed
        .map(|p| p.error)
        .unwrap_or_else(|| fallback_status_message(status));

    match status {
        StatusCode::UNAUTHORIZED => AppError::TransportAuth(message),
        StatusCode::SERVICE_UNAVAILABLE => AppError::RemoteFailure(message),
        StatusCode::INTERNAL_SERVER_ERROR => AppError::RemoteFailure(message),
        _ => AppError::Transport(format!(
            "remote request failed with status {}: {}",
            status.as_u16(),
            message
        )),
    }
}

fn fallback_status_message(status: StatusCode) -> String {
    match status {
        StatusCode::UNAUTHORIZED => "authentication failed".into(),
        StatusCode::SERVICE_UNAVAILABLE => "remote clipboard is unavailable".into(),
        StatusCode::INTERNAL_SERVER_ERROR => "remote daemon encountered an internal error".into(),
        _ => format!("unexpected http status {}", status.as_u16()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_messsage_for_unauthorized_status_is_clear() {
        let status = StatusCode::UNAUTHORIZED;
        let message = fallback_status_message(status);
        assert_eq!(message, "authentication failed");
    }

    #[test]
    fn fallback_message_for_service_unavailable_status_is_clear() {
        let status = StatusCode::SERVICE_UNAVAILABLE;
        let message = fallback_status_message(status);
        assert_eq!(message, "remote clipboard is unavailable");
    }

    #[test]
    fn client_builds_for_valid_address() {
        let address: SocketAddr = "127.0.0.1:3947".parse().unwrap();
        let client = TransportClient::new(address, "testtoken");
        assert!(client.is_ok());
    }

    #[test]
    fn client_supports_https_scheme() {
        let address: SocketAddr = "127.0.0.1:3947".parse().unwrap();
        let client = TransportClient::new_with_scheme(address, "testtoken", "https").unwrap();
        assert_eq!(client.base_url, "https://127.0.0.1:3947");
    }

    #[test]
    fn client_rejects_invalid_scheme() {
        let address: SocketAddr = "127.0.0.1:3947".parse().unwrap();
        let err = TransportClient::new_with_scheme(address, "testtoken", "ftp").unwrap_err();
        assert!(matches!(err, AppError::Transport(msg) if msg.contains("invalid transport scheme")));
    }
}
