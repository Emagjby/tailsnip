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
const DEFAULT_DAEMON_LISTEN: &str = "127.0.0.1:3947";

#[derive(Debug, Clone)]
pub struct Config {
    #[allow(dead_code)]
    pub token: String,
    pub daemon: DaemonConfig,
    pub devices: BTreeMap<String, SocketAddr>,
}

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub listen: SocketAddr,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    token: Option<String>,
    daemon: Option<RawDaemonConfig>,
    devices: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct RawDaemonConfig {
    listen: Option<String>,
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

        let daemon = validate_daemon_config(raw.daemon)?;

        let raw_devices = match raw.devices {
            Some(devices) if devices.is_empty() => {
                return Err(AppError::ConfigValidation(
                    "devices must contain at least one entry".into(),
                ));
            }
            Some(devices) => devices,
            None => BTreeMap::new(),
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

        Ok(Self {
            token,
            daemon,
            devices,
        })
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

    pub fn resolve_device(&self, alias: &str) -> AppResult<SocketAddr> {
        self.devices
            .get(alias)
            .copied()
            .ok_or_else(|| AppError::DeviceNotFound(alias.to_string()))
    }
}

fn validate_daemon_config(raw: Option<RawDaemonConfig>) -> AppResult<DaemonConfig> {
    let listen_str = raw
        .and_then(|d| d.listen)
        .unwrap_or_else(|| DEFAULT_DAEMON_LISTEN.to_string());

    let listen = listen_str.parse::<SocketAddr>().map_err(|_| {
        AppError::ConfigValidation(format!(
            "daemon.listen has invalid address '{listen_str}' (expected IP:PORT)"
        ))
    })?;

    Ok(DaemonConfig { listen })
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

    file.write_all(template_config().as_bytes())
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

fn template_config() -> String {
    format!(
        "# Replace the token with your real value.\n\n\
token = \"change-me\"\n\n\
[daemon]\n\
listen = \"{DEFAULT_DAEMON_LISTEN}\"\n"
    )
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

[daemon]
listen = "127.0.0.1:3947"

[devices]
macbook = "100.64.0.10:3947"
desktop = "100.64.0.11:3947"
            "#,
        );

        let config = Config::load_from_path(&path).unwrap();

        assert_eq!(config.token, "secret-token");

        assert_eq!(config.daemon.listen.to_string(), "127.0.0.1:3947");

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
    fn allows_missing_devices_table() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"
            "#,
        );

        let config = Config::load_from_path(&path).unwrap();
        assert!(config.devices.is_empty());
    }

    #[test]
    fn applies_default_daemon_listen_address_when_omitted() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"

[devices]
macbook = "100.64.0.10:3947"
            "#,
        );

        let config = Config::load_from_path(&path).unwrap();
        assert_eq!(config.daemon.listen.to_string(), DEFAULT_DAEMON_LISTEN);
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
    fn builds_config_path_from_xdg_config_home_and_creates_template_config() {
        let dir = tempfile::tempdir().unwrap();

        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(exe)
            .arg("--exact")
            .arg("config::tests::template_config_helper_subprocess")
            .arg("--nocapture")
            .env("TAILSNIP_TEMPLATE_HELPER", "1")
            .env("XDG_CONFIG_HOME", dir.path())
            .env_remove("HOME")
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "helper subprocess failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let path = dir.path().join("tailsnip").join("tailsnip.toml");
        assert!(path.exists());

        let contents = fs::read_to_string(path).unwrap();

        assert!(contents.contains("token = \"change-me\""));
        assert!(contents.contains("[daemon]"));
        assert!(contents.contains("listen = \"127.0.0.1:3947\""));
        assert!(!contents.contains("[devices]"));
    }

    #[test]
    fn template_config_helper_subprocess() {
        if std::env::var_os("TAILSNIP_TEMPLATE_HELPER").is_none() {
            return;
        }

        let path = ensure_template_config().unwrap();
        assert!(path.exists());
    }

    #[test]
    fn rejects_invalid_daemon_listen_address() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"

[daemon]
listen = "invalid-address"

[devices]
macbook = "100.10.10.10:1010"
desktop = "100.10.10.11:1010"
            "#,
        );

        let err = Config::load_from_path(&path).unwrap_err();
        assert!(
            matches!(err, AppError::ConfigValidation(msg) if msg.contains("daemon.listen has invalid address 'invalid-address'"))
        );
    }

    #[test]
    fn resolves_known_device_alias() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"

[devices]
macbook = "100.64.0.10:3947"
            "#,
        );

        let config = Config::load_from_path(&path).unwrap();
        let address = config.resolve_device("macbook").unwrap();
        assert_eq!(address.to_string(), "100.64.0.10:3947");
    }

    #[test]
    fn rejects_unknown_device_alias() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"

[devices]
macbook = "100.64.0.10:3947"
            "#,
        );

        let config = Config::load_from_path(&path).unwrap();
        let err = config.resolve_device("desktop").unwrap_err();
        assert!(matches!(err, AppError::DeviceNotFound(alias) if alias == "desktop"));
    }

    #[test]
    fn returns_sorted_device_entries_for_operator_output() {
        let (_dir, path) = write_config(
            r#"
token = "secret-token"

[devices]
desktop = "100.64.0.11:3947"
air = "100.64.0.12:3947"
macbook = "100.64.0.10:3947"
            "#,
        );

        let config = Config::load_from_path(&path).unwrap();
        let aliases: Vec<_> = config
            .device_entries()
            .into_iter()
            .map(|entry| entry.alias)
            .collect();

        assert_eq!(aliases, ["air", "desktop", "macbook"]);
    }
}
