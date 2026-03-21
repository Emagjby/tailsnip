use std::{
    path::Path,
    process::{Child, Command, Output, Stdio},
    sync::{
        OnceLock,
        mpsc::{SyncSender, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

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

const CLIPBOARD_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(25);
const PBCOPY_GRACE_TIMEOUT: Duration = Duration::from_millis(150);
const WL_COPY_GRACE_TIMEOUT: Duration = Duration::from_millis(150);
const XCLIP_GRACE_TIMEOUT: Duration = Duration::from_millis(150);
const CHILD_REAPER_QUEUE_CAPACITY: usize = 64;

static CHILD_REAPER_TX: OnceLock<SyncSender<Child>> = OnceLock::new();

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

    pub fn detect_for_write() -> AppResult<Self> {
        if cfg!(target_os = "linux") {
            if env_var_present("WAYLAND_DISPLAY") && command_exists("wl-copy") {
                return Ok(Self {
                    backend: ClipboardBackend::LinuxWayland,
                });
            }

            if env_var_present("DISPLAY") && command_exists("xclip") {
                return Ok(Self {
                    backend: ClipboardBackend::LinuxXclip,
                });
            }

            return Err(AppError::ClipboardUnavailable(
                "no supported Linux clipboard write backend found; write support requires Wayland with 'wl-copy' or X11 with 'xclip'".into(),
            ));
        }

        Self::detect()
    }

    pub fn backend(&self) -> ClipboardBackend {
        self.backend
    }

    pub fn from_backend(backend: ClipboardBackend) -> Self {
        Self { backend }
    }

    pub fn read_text(&self) -> AppResult<String> {
        self.read_text_with_timeout(CLIPBOARD_COMMAND_TIMEOUT)
    }

    pub fn read_text_with_timeout(&self, command_timeout: Duration) -> AppResult<String> {
        match self.backend {
            ClipboardBackend::MacOsPbcopy => read_with_command("pbpaste", &[], command_timeout),
            ClipboardBackend::LinuxWayland => {
                read_with_command("wl-paste", &["--no-newline"], command_timeout)
            }
            ClipboardBackend::LinuxXclip => {
                read_with_command("xclip", &["-selection", "clipboard", "-o"], command_timeout)
            }
        }
    }

    pub fn write_text_with_timeout(&self, text: &str, command_timeout: Duration) -> AppResult<()> {
        match self.backend {
            ClipboardBackend::MacOsPbcopy => write_with_osascript_or_pbcopy(text, command_timeout),
            ClipboardBackend::LinuxWayland => write_with_wl_copy_owner(text),
            ClipboardBackend::LinuxXclip => write_with_command_inner(
                "xclip",
                &["-selection", "clipboard"],
                text,
                Some(XCLIP_GRACE_TIMEOUT),
                command_timeout,
            ),
        }
    }

    pub fn spawn_wayland_owner(&self, text: &str) -> AppResult<Child> {
        if self.backend != ClipboardBackend::LinuxWayland {
            return Err(AppError::ClipboardWrite(
                "wayland owner process is only supported for Linux Wayland backend".into(),
            ));
        }

        spawn_wl_copy_owner(text)
    }
}

fn write_with_osascript_or_pbcopy(text: &str, command_timeout: Duration) -> AppResult<()> {
    if command_exists("osascript") {
        let child = Command::new("osascript")
            .arg("-e")
            .arg("on run argv")
            .arg("-e")
            .arg("set the clipboard to item 1 of argv")
            .arg("-e")
            .arg("end run")
            .arg("--")
            .arg(text)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AppError::ClipboardWrite(format!("failed to execute 'osascript': {e}")))?;

        let output = wait_with_output_timeout(child, "osascript", false, command_timeout)?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        eprintln!(
            "tailsnip debug: write_with_osascript_or_pbcopy osascript non-success status={} stderr='{}'",
            output.status, stderr
        );

        if !stderr.is_empty() {
            return Err(AppError::ClipboardWrite(format!(
                "'osascript' failed: {stderr}"
            )));
        }
    } else {
        eprintln!(
            "tailsnip debug: write_with_osascript_or_pbcopy falling back because osascript is unavailable"
        );
    }

    write_with_command_inner(
        "pbcopy",
        &[],
        text,
        Some(PBCOPY_GRACE_TIMEOUT),
        command_timeout,
    )
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

fn read_with_command(cmd: &str, args: &[&str], command_timeout: Duration) -> AppResult<String> {
    let child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::ClipboardRead(format!("failed to execute '{cmd}': {e}")))?;

    let output = wait_with_output_timeout(child, cmd, true, command_timeout)?;

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

fn write_with_wl_copy_owner(text: &str) -> AppResult<()> {
    let mut child = spawn_wl_copy_owner(text)?;

    if !wait_until_exit_or_deadline(&mut child, WL_COPY_GRACE_TIMEOUT, "wl-copy", false)? {
        enqueue_child_reap(child);
        return Ok(());
    }

    let output = child.wait_with_output().map_err(|e| {
        AppError::ClipboardWrite(format!("failed to wait for 'wl-copy' to finish: {e}"))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("'wl-copy' exited with status {}", output.status)
        } else {
            format!("'wl-copy' failed: {stderr}")
        };

        return Err(AppError::ClipboardWrite(detail));
    }

    Ok(())
}

fn spawn_wl_copy_owner(text: &str) -> AppResult<Child> {
    let mut child = Command::new("wl-copy")
        .arg("--foreground")
        .arg("--type")
        .arg("text/plain")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::ClipboardWrite(format!("failed to execute 'wl-copy': {e}")))?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::ClipboardWrite("failed to open stdin for 'wl-copy'".into()))?;

        use std::io::Write;
        stdin.write_all(text.as_bytes()).map_err(|e| {
            AppError::ClipboardWrite(format!("failed to write clipboard input to 'wl-copy': {e}"))
        })?;
    }

    if wait_until_exit_or_deadline(&mut child, WL_COPY_GRACE_TIMEOUT, "wl-copy", false)? {
        let output = child.wait_with_output().map_err(|e| {
            AppError::ClipboardWrite(format!("failed to wait for 'wl-copy' to finish: {e}"))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if stderr.is_empty() {
                format!("'wl-copy' exited with status {}", output.status)
            } else {
                format!("'wl-copy' failed: {stderr}")
            };

            return Err(AppError::ClipboardWrite(detail));
        }

        return Err(AppError::ClipboardWrite(
            "'wl-copy --foreground' exited within the 150ms grace period; clipboard ownership was not retained".into(),
        ));
    }

    Ok(child)
}

fn write_with_command_inner(
    cmd: &str,
    args: &[&str],
    text: &str,
    persistent_success_grace: Option<Duration>,
    command_timeout: Duration,
) -> AppResult<()> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::ClipboardWrite(format!("failed to execute '{cmd}': {e}")))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::ClipboardWrite(format!("failed to open stdin for '{cmd}'")))?;

        use std::io::Write;
        stdin.write_all(text.as_bytes()).map_err(|e| {
            AppError::ClipboardWrite(format!("failed to write clipboard input to '{cmd}': {e}"))
        })?;
    }

    if let Some(grace_timeout) = persistent_success_grace
        && !wait_until_exit_or_deadline(&mut child, grace_timeout, cmd, false)?
    {
        enqueue_child_reap(child);
        return Ok(());
    }

    let output = wait_with_output_timeout(child, cmd, false, command_timeout)?;

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

fn wait_until_exit_or_deadline(
    child: &mut Child,
    deadline: Duration,
    cmd: &str,
    is_read: bool,
) -> AppResult<bool> {
    let started = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(true),
            Ok(None) => {
                if started.elapsed() >= deadline {
                    return Ok(false);
                }
                thread::sleep(COMMAND_POLL_INTERVAL);
            }
            Err(e) => {
                return if is_read {
                    Err(AppError::ClipboardRead(format!(
                        "failed while waiting for '{cmd}': {e}"
                    )))
                } else {
                    Err(AppError::ClipboardWrite(format!(
                        "failed while waiting for '{cmd}': {e}"
                    )))
                };
            }
        }
    }
}

fn wait_with_output_timeout(
    mut child: Child,
    cmd: &str,
    is_read: bool,
    timeout: Duration,
) -> AppResult<Output> {
    if !wait_until_exit_or_deadline(&mut child, timeout, cmd, is_read)? {
        let _ = child.kill();
        let _ = child.wait();

        return if is_read {
            Err(AppError::ClipboardRead(format!(
                "'{cmd}' timed out after {}s",
                timeout.as_secs()
            )))
        } else {
            Err(AppError::ClipboardWrite(format!(
                "'{cmd}' timed out after {}s",
                timeout.as_secs()
            )))
        };
    }

    child.wait_with_output().map_err(|e| {
        if is_read {
            AppError::ClipboardRead(format!("failed to wait for '{cmd}' to finish: {e}"))
        } else {
            AppError::ClipboardWrite(format!("failed to wait for '{cmd}' to finish: {e}"))
        }
    })
}

fn command_exists(cmd: &str) -> bool {
    if cmd.contains('/') {
        return is_executable(Path::new(cmd));
    }

    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };

    for dir in std::env::split_paths(&path_var) {
        if is_executable(&dir.join(cmd)) {
            return true;
        }
    }

    false
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(metadata) => metadata.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        }
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn env_var_present(var: &str) -> bool {
    std::env::var_os(var).is_some()
}

fn enqueue_child_reap(child: Child) {
    if let Err(err) = child_reaper_sender().send(child) {
        eprintln!("tailsnip debug: child reaper channel unavailable; waiting inline");
        let mut child = err.0;
        let _ = child.wait();
    }
}

fn child_reaper_sender() -> &'static SyncSender<Child> {
    CHILD_REAPER_TX.get_or_init(|| {
        let (sender, receiver) = sync_channel::<Child>(CHILD_REAPER_QUEUE_CAPACITY);
        thread::spawn(move || child_reaper_worker(receiver));

        sender
    })
}

fn child_reaper_worker(receiver: std::sync::mpsc::Receiver<Child>) {
    for mut child in receiver {
        let _ = child.wait();
    }
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
        let capability = detect_capability();

        if capability.supported {
            assert!(capability.backend.is_some());
            assert!(capability.reason.is_none());
        } else {
            assert!(capability.backend.is_none());
            assert!(capability.reason.is_some());
        }
    }
}
