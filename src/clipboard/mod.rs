use std::process::{Command, Stdio};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardBackend {
    MacOsPbcopy,
    LinuxWayland,
    LinuxXclip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardCapability {
    pub supported: bool,
    pub backend: Option<ClipboardBackend>,
    pub reason: Option<String>,
}

pub struct Clipboard {
    backend: ClipboardBackend,
}

impl Clipboard {
    pub fn detect() -> AppResult<Self> {
        let capability = detect_capability();

        match (capability.supported, capability.backend) {
            (true, Some(backend)) => Ok(Self { backend }),
            _ => Err(AppError::ClipboardUnavailable(
                capability
                    .reason
                    .unwrap_or_else(|| "no supported clipboard backend found".to_string()),
            )),
        }
    }

    pub fn capability() -> ClipboardCapability {
        detect_capability()
    }

    pub fn backend(&self) -> ClipboardBackend {
        self.backend
    }

    pub fn read_text(&self) -> AppResult<String> {
        match self.backend {
            ClipboardBackend::MacOsPbcopy => read_with_command("pbpaste", &[]),
            ClipboardBackend::LinuxWayland => read_with_command("wl-paste", &["--no-newline"]),
            ClipboardBackend::LinuxXclip => {
                read_with_command("xclip", &["-selection", "clipboard", "-o"])
            }
        }
    }

    pub fn write_text(&self, text: &str) -> AppResult<()> {
        match self.backend {
            ClipboardBackend::MacOsPbcopy => write_with_command("pbcopy", &[], text),
            ClipboardBackend::LinuxWayland => write_with_command("wl-copy", &[], text),
            ClipboardBackend::LinuxXclip => {
                write_with_command("xclip", &["-selection", "clipboard"], text)
            }
        }
    }
}

fn detect_capability() -> ClipboardCapability {
    if cfg!(target_os = "macos") {
        let has_pbcopy = command_exists("pbcopy");
        let has_pbpaste = command_exists("pbpaste");

        if has_pbcopy && has_pbpaste {
            return ClipboardCapability {
                supported: true,
                backend: Some(ClipboardBackend::MacOsPbcopy),
                reason: None,
            };
        }

        return ClipboardCapability {
            supported: false,
            backend: None,
            reason: Some("macOS clipboard tools 'pbcopy' and/or 'pbpaste' are unavailable".into()),
        };
    }

    if cfg!(target_os = "linux") {
        if env_var_present("WAYLAND_DISPLAY")
            && command_exists("wl-copy")
            && command_exists("wl-paste")
        {
            return ClipboardCapability {
                supported: true,
                backend: Some(ClipboardBackend::LinuxWayland),
                reason: None,
            };
        }

        if env_var_present("DISPLAY") && command_exists("xclip") {
            return ClipboardCapability {
                supported: true,
                backend: Some(ClipboardBackend::LinuxXclip),
                reason: None,
            };
        }

        return ClipboardCapability {
            supported: false,
            backend: None,
            reason: Some(
                "no supported Linux clipboard backend found; support requires Wayland with 'wl-copy'/'wl-paste' or X11 with 'xclip'".into(),
            ),
        };
    }

    ClipboardCapability {
        supported: false,
        backend: None,
        reason: Some("clipboard support is only implemented for macOS and Linux".into()),
    }
}

fn read_with_command(cmd: &str, args: &[&str]) -> AppResult<String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| AppError::ClipboardRead(format!("failed to execute '{cmd}': {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("'{cmd}' exited with status {}", output.status)
        } else {
            format!("'{cmd}' failed: {stderr}")
        };

        return Err(AppError::ClipboardRead(detail));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| AppError::ClipboardRead(format!("clipboard output was not valid UTF-8: {e}")))
}

fn write_with_command(cmd: &str, args: &[&str], text: &str) -> AppResult<()> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::ClipboardWrite(format!("failed to execute '{cmd}': {e}")))?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| AppError::ClipboardWrite(format!("failed to open stdin for '{cmd}'")))?;

        use std::io::Write;
        stdin.write_all(text.as_bytes()).map_err(|e| {
            AppError::ClipboardWrite(format!("failed to write clipboard input to '{cmd}': {e}"))
        })?;
    }

    let output = child.wait_with_output().map_err(|e| {
        AppError::ClipboardWrite(format!("failed while waiting for '{cmd}' to finish: {e}"))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("'{cmd}' exited with status {}", output.status)
        } else {
            format!("'{cmd}' failed: {stderr}")
        };

        return Err(AppError::ClipboardWrite(detail));
    }

    Ok(())
}

fn command_exists(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn env_var_present(var: &str) -> bool {
    std::env::var_os(var).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_presence_helper_detects_missing_value() {
        let key = "TAILSNIP_TEST_ENV_VAR";
        assert!(!env_var_present(key));
    }

    #[test]
    fn command_exists_returns_false_for_obviously_missing_binary() {
        assert!(!command_exists("definitely-not-a-real-command"));
    }

    #[test]
    fn capability_shape_is_consistent() {
        let capability = Clipboard::capability();

        if capability.supported {
            assert!(capability.backend.is_some());
            assert!(capability.reason.is_none());
        } else {
            assert!(capability.backend.is_none());
            assert!(capability.reason.is_some());
        }
    }
}
