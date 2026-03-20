use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    pub alias: String,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipboardReadResponse {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipboardWriteRequest {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiStatusResponse {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiErrorResponse {
    pub ok: bool,
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_clipboard_write_request() {
        let request = ClipboardWriteRequest {
            text: "hello from tailsnip".to_string(),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(json, r#"{"text":"hello from tailsnip"}"#);
    }

    #[test]
    fn deserializes_clipboard_read_response() {
        let json = r#"{"text":"hello from tailsnip"}"#;
        let response: ClipboardReadResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "hello from tailsnip");
    }

    #[test]
    fn round_trips_status_response() {
        let response = ApiStatusResponse {
            ok: true,
            message: "clipboard updated".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        let deserialized: ApiStatusResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(response, deserialized);
    }

    #[test]
    fn round_trips_error_response() {
        let response = ApiErrorResponse {
            ok: false,
            error: "failed to read clipboard".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        let deserialized: ApiErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(response, deserialized);
    }
}
