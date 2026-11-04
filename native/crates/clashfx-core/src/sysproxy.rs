use crate::store::{Store, atomic_write};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Backup {
    #[serde(default)]
    session: String,
    owned_server: String,
    enabled: Option<u32>,
    server: Option<String>,
    bypass: Option<String>,
}

pub struct ProxyGuard {
    #[cfg(windows)]
    child: std::process::Child,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProxyHealth {
    Healthy,
    Repaired,
    TakenOver,
}

impl Drop for ProxyGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            self.child.kill().ok();
            self.child.wait().ok();
        }
    }
}

#[cfg(windows)]
fn start_guard(mut command: std::process::Command) -> Result<ProxyGuard> {
    use std::{
        io::{BufRead, BufReader},
        os::windows::process::CommandExt,
        process::Stdio,
    };
    command
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut guard = ProxyGuard {
        child: command.spawn().context("无法启动系统代理恢复进程")?,
    };
    let stdout = guard
        .child
        .stdout
        .take()
        .context("恢复进程未提供就绪信号")?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let ready = BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
            .any(|line| line.trim() == "CLASHFX_GUARD_READY");
        sender.send(ready).ok();
    });
    anyhow::ensure!(
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or(false),
        "系统代理恢复进程未就绪，未修改系统代理"
    );
    Ok(guard)
}

#[cfg(windows)]
fn wait_for_parent(parent: u32) -> Result<()> {
    use std::{
        io::Write,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use windows_sys::Win32::{
        Foundation::WAIT_OBJECT_0,
        System::Threading::{INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    unsafe {
        let raw = OpenProcess(PROCESS_SYNCHRONIZE, 0, parent);
        anyhow::ensure!(!raw.is_null(), "主进程已经退出");
        let process = OwnedHandle::from_raw_handle(raw);
        println!("CLASHFX_GUARD_READY");
        std::io::stdout().flush()?;
        anyhow::ensure!(
            WaitForSingleObject(process.as_raw_handle(), INFINITE) == WAIT_OBJECT_0,
            "无法等待主进程退出"
        );
    }
    Ok(())
}

#[cfg(windows)]
pub fn run_guard_process() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 6
            && args[0] == "--proxy-guard"
            && args[2] == "--data-dir"
            && args[4] == "--session",
        "无效的代理恢复参数"
    );
    let parent: u32 = args[1].parse()?;
    uuid::Uuid::parse_str(&args[5])?;
    let store = Store::open(Some((&args[3]).into()))?;
    wait_for_parent(parent)?;
    for _ in 0..100 {
        if let Ok(_lock) = store.lock() {
            return restore_for(&store, Some(&args[5]));
        }
        if !store.root.join("proxy-backup.json").exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn run_guard_process() -> Result<()> {
    anyhow::bail!("System proxy recovery is only supported on Windows")
}

#[cfg(windows)]
fn settings() -> Result<winreg::RegKey> {
    use winreg::{
        RegKey,
        enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
    };
    Ok(RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
        KEY_READ | KEY_WRITE,
    )?)
}

#[cfg(windows)]
fn refresh() {
    use windows_sys::Win32::Networking::WinInet::{
        INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED, InternetSetOptionW,
    };
    unsafe {
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null(),
            0,
        );
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null(),
            0,
        );
    }
}

#[cfg(windows)]
pub fn enable(store: &Store, port: u16) -> Result<ProxyGuard> {
    let key = settings()?;
    let backup_path = store.root.join("proxy-backup.json");
    anyhow::ensure!(
        !backup_path.exists(),
        "仍有未恢复的系统代理记录，请先关闭系统代理"
    );
    let server = format!("127.0.0.1:{port}");
    let session = uuid::Uuid::new_v4().to_string();
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("--proxy-guard")
        .arg(std::process::id().to_string())
        .arg("--data-dir")
        .arg(&store.root)
        .arg("--session")
        .arg(&session);
    let guard = start_guard(command)?;
    let backup = Backup {
        session,
        owned_server: server.clone(),
        enabled: key.get_value("ProxyEnable").ok(),
        server: key.get_value("ProxyServer").ok(),
        bypass: key.get_value("ProxyOverride").ok(),
    };
    atomic_write(&backup_path, &serde_json::to_vec(&backup)?)?;
    let result = (|| -> Result<()> {
        key.set_value("ProxyServer", &server)?;
        key.set_value("ProxyOverride", &"<local>;localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.2?.*;172.30.*;172.31.*;192.168.*")?;
        key.set_value("ProxyEnable", &1u32)?;
        refresh();
        Ok(())
    })();
    if result.is_err() {
        restore(store).ok();
    }
    result?;
    Ok(guard)
}

#[cfg(windows)]
pub fn restore(store: &Store) -> Result<()> {
    restore_for(store, None)
}

#[cfg(windows)]
pub(crate) fn check(store: &Store, port: u16) -> Result<ProxyHealth> {
    let health = check_with_key(store, port, &settings()?)?;
    if health == ProxyHealth::Repaired {
        refresh();
    }
    Ok(health)
}

#[cfg(windows)]
fn check_with_key(store: &Store, port: u16, key: &winreg::RegKey) -> Result<ProxyHealth> {
    let path = store.root.join("proxy-backup.json");
    let backup: Backup = serde_json::from_slice(&std::fs::read(&path)?)?;
    anyhow::ensure!(
        backup.owned_server == format!("127.0.0.1:{port}"),
        "系统代理恢复记录与当前内核不一致"
    );
    let server: String = key.get_value("ProxyServer")?;
    if server != backup.owned_server {
        std::fs::remove_file(path)?;
        return Ok(ProxyHealth::TakenOver);
    }
    if key.get_value::<u32, _>("ProxyEnable")? == 1 {
        return Ok(ProxyHealth::Healthy);
    }
    key.set_value("ProxyEnable", &1u32)?;
    Ok(ProxyHealth::Repaired)
}

#[cfg(windows)]
fn restore_for(store: &Store, session: Option<&str>) -> Result<()> {
    restore_with_key(store, session, &settings()?)?;
    refresh();
    Ok(())
}

#[cfg(windows)]
fn restore_with_key(store: &Store, session: Option<&str>, key: &winreg::RegKey) -> Result<()> {
    let path = store.root.join("proxy-backup.json");
    if !path.exists() {
        return Ok(());
    }
    let backup: Backup = serde_json::from_slice(&std::fs::read(&path)?)?;
    if session.is_some_and(|session| session != backup.session) {
        return Ok(());
    }
    restore_key(key, &backup)?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[cfg(windows)]
fn restore_key(key: &winreg::RegKey, backup: &Backup) -> Result<()> {
    let current: Option<String> = key.get_value("ProxyServer").ok();
    if current.as_deref() == Some(&backup.owned_server) {
        if let Some(value) = backup.enabled {
            key.set_value("ProxyEnable", &value)?;
        } else {
            key.delete_value("ProxyEnable").ok();
        }
        for (name, value) in [
            ("ProxyServer", &backup.server),
            ("ProxyOverride", &backup.bypass),
        ] {
            if let Some(value) = value {
                key.set_value(name, value)?;
            } else {
                key.delete_value(name).ok();
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn enable(_: &Store, _: u16) -> Result<ProxyGuard> {
    anyhow::bail!("此版本只支持自动设置 Windows 系统代理")
}

#[cfg(not(windows))]
pub fn restore(_: &Store) -> Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn check(_: &Store, _: u16) -> Result<ProxyHealth> {
    Ok(ProxyHealth::Healthy)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        process::{Command, Stdio},
        thread::sleep,
        time::{Duration, Instant},
    };
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    fn child_test(name: &str) -> Result<Command> {
        let mut command = Command::new(std::env::current_exe()?);
        command.args([
            "--exact",
            &format!("sysproxy::tests::{name}"),
            "--ignored",
            "--nocapture",
        ]);
        Ok(command)
    }

    fn test_store() -> Result<Store> {
        Store::open(Some(std::env::var("CLASHFX_TEST_STORE")?.into()))
    }
    fn test_key() -> Result<RegKey> {
        Ok(RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(std::env::var("CLASHFX_TEST_KEY")?)?
            .0)
    }

    #[test]
    #[ignore = "Child process used by the crash regression test."]
    fn guard_process() -> Result<()> {
        let store = test_store()?;
        wait_for_parent(std::env::var("CLASHFX_TEST_PARENT")?.parse()?)?;
        let _lock = store.lock()?;
        restore_with_key(
            &store,
            Some(&std::env::var("CLASHFX_TEST_SESSION")?),
            &test_key()?,
        )
    }

    #[test]
    #[ignore = "Child process used by the crash regression test."]
    fn crash_process() -> Result<()> {
        use crate::{mihomo, model::CoreSettings};
        let store = test_store()?;
        let _lock = store.lock()?;
        let session = uuid::Uuid::new_v4().to_string();
        let mut command = child_test("guard_process")?;
        command
            .env("CLASHFX_TEST_PARENT", std::process::id().to_string())
            .env("CLASHFX_TEST_SESSION", &session);
        let guard = start_guard(command)?;
        let key = test_key()?;
        let backup = Backup {
            session,
            owned_server: "127.0.0.1:17897".into(),
            enabled: Some(0),
            server: Some("127.0.0.1:7890".into()),
            bypass: Some("<local>".into()),
        };
        atomic_write(
            &store.root.join("proxy-backup.json"),
            &serde_json::to_vec(&backup)?,
        )?;
        key.set_value("ProxyEnable", &1u32)?;
        key.set_value("ProxyServer", &backup.owned_server)?;
        let port = || -> Result<u16> {
            Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?
                .local_addr()?
                .port())
        };
        let settings = CoreSettings {
            controller_port: port()?,
            mixed_port: port()?,
            secret: "crash-fixture".into(),
            ..Default::default()
        };
        atomic_write(&store.runtime_path(), format!("mixed-port: {}\nexternal-controller: 127.0.0.1:{}\nsecret: crash-fixture\nmode: rule\nrules: [MATCH,DIRECT]\n", settings.mixed_port, settings.controller_port).replace("rules: [MATCH,DIRECT]", "rules: ['MATCH,DIRECT']").as_bytes())?;
        let runtime = tokio::runtime::Runtime::new()?;
        let core = runtime.block_on(mihomo::start(&settings, &store))?;
        atomic_write(
            &store.root.join("ready.json"),
            &serde_json::to_vec(&(core.id(), guard.child.id()))?,
        )?;
        loop {
            sleep(Duration::from_secs(1));
        }
    }

    #[test]
    #[ignore = "Runs Mihomo and forcibly terminates an isolated parent process."]
    fn forced_parent_exit_kills_core_and_recovers_proxy() -> Result<()> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0,
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let id = uuid::Uuid::new_v4();
        let store = Store::open(Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../work/crash-{id}")),
        ))?;
        let registry_path = format!("Software\\ClashFx\\Tests\\{id}");
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(&registry_path)?
            .0;
        let mut command = child_test("crash_process")?;
        command
            .env("CLASHFX_TEST_STORE", &store.root)
            .env("CLASHFX_TEST_KEY", &registry_path);
        let log = std::fs::File::create(store.root.join("parent.log"))?;
        let mut parent = command
            .stdout(log.try_clone()?)
            .stderr(log)
            .stdin(Stdio::null())
            .spawn()?;
        let result = (|| -> Result<()> {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !store.root.join("ready.json").exists() {
                anyhow::ensure!(
                    Instant::now() < deadline && parent.try_wait()?.is_none(),
                    "Crash fixture did not start; see {}",
                    store.root.display()
                );
                sleep(Duration::from_millis(50));
            }
            let (core, guard): (u32, u32) =
                serde_json::from_slice(&std::fs::read(store.root.join("ready.json"))?)?;
            let process_handle = |pid| -> Result<OwnedHandle> {
                unsafe {
                    let raw = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
                    anyhow::ensure!(!raw.is_null(), "Child process is not running");
                    Ok(OwnedHandle::from_raw_handle(raw))
                }
            };
            let core = process_handle(core)?;
            let guard = process_handle(guard)?;
            assert_eq!(key.get_value::<u32, _>("ProxyEnable")?, 1);
            parent.kill()?;
            parent.wait()?;
            unsafe {
                assert_eq!(
                    WaitForSingleObject(core.as_raw_handle(), 5000),
                    WAIT_OBJECT_0,
                    "Core survived parent termination"
                );
                assert_eq!(
                    WaitForSingleObject(guard.as_raw_handle(), 5000),
                    WAIT_OBJECT_0,
                    "Recovery process did not finish"
                );
            }
            assert_eq!(key.get_value::<u32, _>("ProxyEnable")?, 0);
            assert_eq!(key.get_value::<String, _>("ProxyServer")?, "127.0.0.1:7890");
            assert!(!store.root.join("proxy-backup.json").exists());
            Ok(())
        })();
        parent.kill().ok();
        parent.wait().ok();
        drop(key);
        RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&registry_path)?;
        result
    }

    #[test]
    fn recovery_does_not_overwrite_a_new_session_or_another_proxy() -> Result<()> {
        let id = uuid::Uuid::new_v4();
        let store = Store::open(Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../work/proxy-owner-{id}")),
        ))?;
        let registry_path = format!("Software\\ClashFx\\Tests\\{id}");
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(&registry_path)?
            .0;
        let backup = Backup {
            session: "new-session".into(),
            owned_server: "127.0.0.1:17897".into(),
            enabled: Some(0),
            server: None,
            bypass: None,
        };
        atomic_write(
            &store.root.join("proxy-backup.json"),
            &serde_json::to_vec(&backup)?,
        )?;
        key.set_value("ProxyEnable", &1u32)?;
        key.set_value("ProxyServer", &backup.owned_server)?;
        restore_with_key(&store, Some("old-session"), &key)?;
        assert!(store.root.join("proxy-backup.json").exists());
        assert_eq!(key.get_value::<u32, _>("ProxyEnable")?, 1);
        key.set_value("ProxyServer", &"127.0.0.1:7890")?;
        restore_with_key(&store, Some("new-session"), &key)?;
        assert_eq!(key.get_value::<String, _>("ProxyServer")?, "127.0.0.1:7890");
        assert_eq!(key.get_value::<u32, _>("ProxyEnable")?, 1);
        drop(key);
        RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&registry_path)?;
        Ok(())
    }

    #[test]
    fn health_check_repairs_disabled_proxy_and_relinquishes_changed_server() -> Result<()> {
        let id = uuid::Uuid::new_v4();
        let store = Store::open(Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../work/proxy-health-{id}")),
        ))?;
        let registry_path = format!("Software\\ClashFx\\Tests\\{id}");
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(&registry_path)?
            .0;
        let result = (|| -> Result<()> {
            let backup = Backup {
                session: id.to_string(),
                owned_server: "127.0.0.1:17897".into(),
                enabled: Some(0),
                server: Some("127.0.0.1:7890".into()),
                bypass: Some("<local>".into()),
            };
            let path = store.root.join("proxy-backup.json");
            let original = serde_json::to_vec(&backup)?;
            atomic_write(&path, &original)?;
            key.set_value("ProxyServer", &backup.owned_server)?;
            key.set_value("ProxyOverride", &"localhost;127.*")?;
            key.set_value("ProxyEnable", &0u32)?;
            assert_eq!(check_with_key(&store, 17897, &key)?, ProxyHealth::Repaired);
            assert_eq!(key.get_value::<u32, _>("ProxyEnable")?, 1);
            assert_eq!(
                key.get_value::<String, _>("ProxyOverride")?,
                "localhost;127.*"
            );
            assert_eq!(std::fs::read(&path)?, original);
            assert_eq!(check_with_key(&store, 17897, &key)?, ProxyHealth::Healthy);

            restore_with_key(&store, None, &key)?;
            assert_eq!(key.get_value::<u32, _>("ProxyEnable")?, 0);
            assert_eq!(key.get_value::<String, _>("ProxyServer")?, "127.0.0.1:7890");

            atomic_write(&path, &original)?;
            key.set_value("ProxyEnable", &1u32)?;
            assert_eq!(check_with_key(&store, 17897, &key)?, ProxyHealth::TakenOver);
            assert!(!path.exists());
            restore_with_key(&store, None, &key)?;
            assert_eq!(key.get_value::<u32, _>("ProxyEnable")?, 1);
            assert_eq!(key.get_value::<String, _>("ProxyServer")?, "127.0.0.1:7890");
            Ok(())
        })();
        drop(key);
        RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&registry_path)?;
        result
    }
}
