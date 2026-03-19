use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    error::{AppError, AppResult},
    types::DeviceEntry,
};

const CONFIG_FILE_NAME: &str = "tailsnip.toml";
const APP_NAME: &str = "tailsnip";
const TEMPLATE_CONFIG: &str = r#"
# Replace the token and device addresses with your real values.

token = "change-me"

[devices]
# macbook = "100.64.0.10:3947"
# desktop = "100.64.0.11:3947"
"#;

#[derive(Debug, Clone)]
pub struct Config {
    #[allow(dead_code)]
    pub token: String,
    pub devices: BTreeMap<String, SocketAddr>,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    token: Option<String>,
    devices: Option<BTreeMap<String, String>>,
}

impl Config {
    pub fn load() -> AppResult<Self> {
        let path = config_file_path()?;
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &Path) -> AppResult<Self> {
        let contents = fs::read_to_string(path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                AppError::ConfigMissing {
                    path: path.to_path_buf(),
                    hint: "run 'tailsnip init' to create a template config".into(),
                }
            } else {
                AppError::ConfigIo {
                    path: path.to_path_buf(),
                    source,
                }
            }
        })?;

        let raw: RawConfig = toml::from_str(&contents).map_err(|source| AppError::ConfigParse {
            path: path.to_path_buf(),
            source,
        })?;

        Self::validate(raw)
    }

    fn validate(raw: RawConfig) -> AppResult<Self> {
        let token = match raw.token {
            Some(token) if !token.trim().is_empty() => token,
            Some(_) => return Err(AppError::ConfigValidation("token cannot be empty".into())),
            None => {
                return Err(AppError::ConfigValidation(
                    "missing required field: token".into(),
                ));
            }
        };

        let raw_devices = match raw.devices {
            Some(devices) if !devices.is_empty() => devices,
            Some(_) => {
                return Err(AppError::ConfigValidation(
                    "devices must contain at least one entry".into(),
                ));
            }
            None => {
                return Err(AppError::ConfigValidation(
                    "missing required table: devices".into(),
                ));
            }
        };

        let mut devices = BTreeMap::new();

        for (alias, address_raw) in raw_devices {
            validate_alias(&alias)?;

            if address_raw.trim().is_empty() {
                return Err(AppError::ConfigValidation(format!(
                    "device '{alias}' has an empty address"
                )));
            }

            let address = address_raw.parse::<SocketAddr>().map_err(|_| {
                AppError::ConfigValidation(format!(
                    "device '{alias}' has invalid address '{address_raw}' (expected HOST:PORT with a valid IP address)"
                ))
            })?;

            devices.insert(alias, address);
        }

        Ok(Self { token, devices })
    }

    pub fn device_entries(&self) -> Vec<DeviceEntry> {
        self.devices
            .iter()
            .map(|(alias, address)| DeviceEntry {
                alias: alias.clone(),
                address: address.to_string(),
            })
            .collect()
    }
}

pub fn ensure_template_config() -> AppResult<PathBuf> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir).map_err(|source| AppError::ConfigDirCreate {
        path: dir.clone(),
        source,
    })?;

    let path = dir.join(CONFIG_FILE_NAME);

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                AppError::ConfigAlreadyExists(path.clone())
            } else {
                AppError::ConfigIo {
                    path: path.clone(),
                    source: e,
                }
            }
        })?;

    file.write_all(TEMPLATE_CONFIG.as_bytes())
        .map_err(|e| AppError::ConfigIo {
            path: path.clone(),
            source: e,
        })?;

    Ok(path)
}

pub fn config_file_path() -> AppResult<PathBuf> {
    Ok(config_dir()?.join(CONFIG_FILE_NAME))
}

fn config_dir() -> AppResult<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg).join(APP_NAME);
        return Ok(path);
    }

    let home = std::env::var_os("HOME").ok_or_else(|| {
        AppError::ConfigHomeMissing("HOME is not set and XDG_CONFIG_HOME is not available".into())
    })?;

    Ok(PathBuf::from(home).join(".config").join(APP_NAME))
}

fn validate_alias(alias: &str) -> AppResult<()> {
    if alias.trim().is_empty() {
        return Err(AppError::ConfigValidation(
            "devices alias cannot be empty".into(),
        ));
    }

    let is_valid = alias
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_');

    if !is_valid {
        return Err(AppError::ConfigValidation(format!(
            "invalid device alias '{alias}' (allowed: lowercase letters, digits, '-' and '_')"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    fn write_config(contents: &str) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tailsnip.toml");
        fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn loads_valid_config() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"

[devices]
macbook = "100.64.0.10:3947"
desktop = "100.64.0.11:3947"
            "#,
        );

        let config = Config::load_from_path(&path).unwrap();

        assert_eq!(config.token, "secret-token");
        assert_eq!(config.devices.len(), 2);
        assert_eq!(
            config.devices.get("macbook").unwrap().to_string(),
            "100.64.0.10:3947"
        );
    }

    #[test]
    fn rejects_missing_token() {
        let (_dir, path) = write_config(
            r#"
[devices]
macbook = "100.64.0.10:3947"
            "#,
        );

        let err = Config::load_from_path(&path).unwrap_err();
        assert!(
            matches!(err, AppError::ConfigValidation(msg) if msg.contains("missing required field: token"))
        );
    }

    #[test]
    fn rejects_empty_token() {
        let (_dir, path) = write_config(
            r#"
token = ""

[devices]
macbook = "100.64.0.10:3947"
            "#,
        );

        let err = Config::load_from_path(&path).unwrap_err();
        assert!(
            matches!(err, AppError::ConfigValidation(msg) if msg.contains("token cannot be empty"))
        );
    }

    #[test]
    fn rejects_missing_devices_table() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"
            "#,
        );

        let err = Config::load_from_path(&path).unwrap_err();
        assert!(
            matches!(err, AppError::ConfigValidation(msg) if msg.contains("missing required table: devices"))
        );
    }

    #[test]
    fn rejects_empty_devices_table() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"

[devices]
            "#,
        );

        let err = Config::load_from_path(&path).unwrap_err();
        assert!(
            matches!(err, AppError::ConfigValidation(msg) if msg.contains("devices must contain at least one entry"))
        );
    }

    #[test]
    fn rejects_invalid_alias() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"

[devices]
"My Laptop" = "100.64.0.10:3947"
            "#,
        );

        let err = Config::load_from_path(&path).unwrap_err();
        assert!(
            matches!(err, AppError::ConfigValidation(msg) if msg.contains("invalid device alias 'My Laptop'"))
        );
    }

    #[test]
    fn rejects_invalid_device_address() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"

[devices]
macbook = "invalid-address"
            "#,
        );

        let err = Config::load_from_path(&path).unwrap_err();
        assert!(
            matches!(err, AppError::ConfigValidation(msg) if msg.contains("device 'macbook' has invalid address 'invalid-address'"))
        );
    }

    #[test]
    fn builds_config_path_from_xdg_config_home() {
        let dir = tempfile::tempdir().unwrap();

        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
            std::env::remove_var("HOME");
        }

        let path = config_file_path().unwrap();
        assert_eq!(path, dir.path().join("tailsnip").join("tailsnip.toml"));
    }

    #[test]
    fn creates_template_config() {
        let dir = tempfile::tempdir().unwrap();

        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
            std::env::remove_var("HOME");
        }

        let path = ensure_template_config().unwrap();
        let contents = fs::read_to_string(path).unwrap();

        assert!(contents.contains("token = \"change-me\""));
        assert!(contents.contains("[devices]"));
    }
}
