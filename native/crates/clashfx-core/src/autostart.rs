use anyhow::Result;
use std::path::Path;

#[cfg(windows)]
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

#[cfg(windows)]
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
#[cfg(windows)]
const VALUE_NAME: &str = "ClashFx";

#[cfg(windows)]
pub fn is_enabled() -> Result<bool> {
    match RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY) {
        Ok(key) => read_registration(&key),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
pub fn set_enabled(enabled: bool, data_dir: &Path) -> Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?.0;
    let command = if enabled {
        Some(startup_command(&std::env::current_exe()?, data_dir))
    } else {
        None
    };
    write_registration(&key, command.as_deref())
}

#[cfg(windows)]
fn startup_command(executable: &Path, data_dir: &Path) -> String {
    fn quoted(path: &Path) -> String {
        let value = path.as_os_str().to_string_lossy();
        let trailing = value.len() - value.trim_end_matches('\\').len();
        format!("\"{value}{}\"", "\\".repeat(trailing))
    }
    format!("{} --data-dir {}", quoted(executable), quoted(data_dir))
}

#[cfg(windows)]
fn read_registration(key: &RegKey) -> Result<bool> {
    match key.get_value::<String, _>(VALUE_NAME) {
        Ok(command) => Ok(!command.is_empty()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
fn write_registration(key: &RegKey, command: Option<&str>) -> Result<()> {
    if let Some(command) = command {
        key.set_value(VALUE_NAME, &command)?;
    } else if let Err(error) = key.delete_value(VALUE_NAME)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(error.into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn is_enabled() -> Result<bool> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn set_enabled(_: bool, _: &Path) -> Result<()> {
    anyhow::bail!("开机自启仅支持 Windows")
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn registration_defaults_off_and_preserves_other_startup_entries() -> Result<()> {
        let path = format!("Software\\ClashFx\\Tests\\{}", uuid::Uuid::new_v4());
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let key = root.create_subkey(&path)?.0;
        let result = (|| -> Result<()> {
            assert!(!read_registration(&key)?);
            key.set_value("OtherApp", &"other.exe")?;
            let command = startup_command(
                Path::new(r"C:\Program Files\ClashFx\ClashFx.exe"),
                Path::new(r"D:\Proxy data\"),
            );
            assert_eq!(
                command,
                "\"C:\\Program Files\\ClashFx\\ClashFx.exe\" --data-dir \"D:\\Proxy data\\\\\""
            );
            write_registration(&key, Some(&command))?;
            assert!(read_registration(&key)?);
            assert_eq!(key.get_value::<String, _>(VALUE_NAME)?, command);
            write_registration(&key, None)?;
            assert!(!read_registration(&key)?);
            assert_eq!(key.get_value::<String, _>("OtherApp")?, "other.exe");
            write_registration(&key, None)?;
            Ok(())
        })();
        drop(key);
        root.delete_subkey(&path)?;
        result
    }
}
