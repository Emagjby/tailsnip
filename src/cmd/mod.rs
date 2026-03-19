use crate::error::{AppError, AppResult};

pub mod daemon;
pub mod devices;
pub mod get;
pub mod init;
pub mod send;

pub fn run() -> AppResult<()> {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some("devices") => devices::run(),
        Some("send") => {
            let target = args
                .next()
                .ok_or_else(|| AppError::Message("missing target device".into()))?;
            send::run(&target)
        }
        Some("get") => {
            let target = args
                .next()
                .ok_or_else(|| AppError::Message("missing target device".into()))?;
            get::run(&target)
        }
        Some("daemon") => daemon::run(),
        Some("init") => init::run(),
        Some(cmd) => Err(AppError::Message(format!("unknown command: {cmd}"))),
        None => Err(AppError::Message(
            "no command provided (expected: devices, send, get, daemon, init)".into(),
        )),
    }
}
