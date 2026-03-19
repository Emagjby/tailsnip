use crate::config::Config;
use crate::error::AppResult;
use crate::types::DeviceEntry;

pub fn run() -> AppResult<()> {
    let config = Config::load()?;
    let output = render_devices(&config.device_entries());
    print!("{output}");
    Ok(())
}

fn render_devices(devices: &[DeviceEntry]) -> String {
    let mut out = String::new();

    for device in devices {
        out.push_str(&device.alias);
        out.push('\t');
        out.push_str(&device.address);
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_devices_in_order_with_tab_separator() {
        let devices = vec![
            DeviceEntry {
                alias: "desktop".to_string(),
                address: "100.64.0.11:3947".to_string(),
            },
            DeviceEntry {
                alias: "macbook".to_string(),
                address: "100.64.0.10:3947".to_string(),
            },
        ];

        let out = render_devices(&devices);

        assert_eq!(
            out,
            "desktop\t100.64.0.11:3947\nmacbook\t100.64.0.10:3947\n"
        );
    }

    #[test]
    fn renders_empty_device_list_as_empty_output() {
        let out = render_devices(&[]);
        assert!(out.is_empty());
    }
}
