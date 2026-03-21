use std::{
    fmt::{Display, Formatter},
    path::PathBuf,
};

#[derive(Debug)]
pub enum AppError {
    Message(String),

    ConfigMissing {
        path: PathBuf,
        hint: String,
    },
    ConfigHomeMissing(String),
    ConfigDirCreate {
        path: PathBuf,
        source: std::io::Error,
    },
    ConfigAlreadyExists(PathBuf),
    ConfigIo {
        path: PathBuf,
        source: std::io::Error,
    },
    ConfigParse {
        path: PathBuf,
        source: toml::de::Error,
    },
    ConfigValidation(String),

    ClipboardUnavailable(String),
    ClipboardRead(String),
    ClipboardWrite(String),

    DeviceNotFound(String),

    Transport(String),
    TransportAuth(String),
    TransportTimeout(String),
    TransportUnreachable(String),
    RemoteFailure(String),

    DaemonStartup(String),
    DaemonRuntime(String),
}

impl Display for AppError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(msg) => write!(f, "{msg}"),

            Self::ConfigMissing { path, hint } => {
                write!(f, "config file not found at '{}': {hint}", path.display(),)
            }
            Self::ConfigHomeMissing(reason) => {
                write!(f, "unable to determine config directory: {reason}")
            }
            Self::ConfigDirCreate { path, source } => {
                write!(
                    f,
                    "failed to create config directory '{}': {source}",
                    path.display()
                )
            }
            Self::ConfigAlreadyExists(path) => {
                write!(f, "config file already exists at '{}'", path.display())
            }
            Self::ConfigIo { path, source } => {
                write!(
                    f,
                    "failed to access config file '{}': {source}",
                    path.display()
                )
            }
            Self::ConfigParse { path, source } => {
                write!(
                    f,
                    "failed to parse config file '{}': {source}",
                    path.display()
                )
            }
            Self::ConfigValidation(message) => write!(f, "invalid config: {message}"),

            Self::ClipboardUnavailable(reason) => {
                write!(f, "clipboard unavailable: {reason}")
            }
            Self::ClipboardRead(reason) => {
                write!(f, "failed to read clipboard: {reason}")
            }
            Self::ClipboardWrite(reason) => {
                write!(f, "failed to write clipboard: {reason}")
            }

            Self::DeviceNotFound(alias) => {
                write!(f, "unknown device alias '{alias}'")
            }

            Self::Transport(reason) => {
                write!(f, "transport error: {reason}")
            }
            Self::TransportAuth(reason) => {
                write!(f, "authentication failed: {reason}")
            }
            Self::TransportTimeout(reason) => {
                write!(f, "transport timed out: {reason}")
            }
            Self::TransportUnreachable(reason) => {
                write!(f, "remote device unreachable: {reason}")
            }
            Self::RemoteFailure(reason) => {
                write!(f, "remote operation failed: {reason}")
            }

            Self::DaemonStartup(reason) => {
                write!(f, "failed to start daemon: {reason}")
            }
            Self::DaemonRuntime(reason) => {
                write!(f, "daemon runtime error: {reason}")
            }
        }
    }
}

impl std::error::Error for AppError {}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_device_not_found_error() {
        let error = AppError::DeviceNotFound("macbook".to_string());
        assert_eq!(format!("{}", error), "unknown device alias 'macbook'");
    }

    #[test]
    fn formats_clipboard_unavailable_error() {
        let error = AppError::ClipboardUnavailable(
            "no supported backend found; install wl-clipboard or xclip".into(),
        );
        assert_eq!(
            format!("{}", error),
            "clipboard unavailable: no supported backend found; install wl-clipboard or xclip"
        );
    }

    #[test]
    fn formats_remote_failure_error() {
        let error = AppError::RemoteFailure("clipboard write rejected".into());
        assert_eq!(
            format!("{}", error),
            "remote operation failed: clipboard write rejected"
        );
    }
}
