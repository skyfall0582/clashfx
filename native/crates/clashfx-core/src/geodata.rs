use crate::store::{Store, atomic_write};
use anyhow::{Context, Result};
use std::{fs, path::PathBuf};

const FILES: [&str; 4] = ["Country.mmdb", "geoip.dat", "geosite.dat", "ASN.mmdb"];

fn bundled() -> Option<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        paths.push(parent.join("geodata"));
    }
    paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bin/geodata"));
    paths
        .into_iter()
        .find(|path| path.join("Country.mmdb").is_file())
}

pub fn prepare(store: &Store) -> Result<()> {
    let Some(source) = bundled() else {
        return Ok(());
    };
    for name in FILES {
        let path = store.root.join(name);
        let existing = fs::read(&path).ok();
        let valid = existing.as_ref().is_some_and(|bytes| {
            bytes.len() > 1024
                && (!name.ends_with(".mmdb")
                    || bytes[bytes.len().saturating_sub(131072)..]
                        .windows(14)
                        .any(|window| window == b"\xab\xcd\xefMaxMind.com"))
                && !bytes.starts_with(b"<!DOCTYPE")
                && !bytes.starts_with(b"<html")
        });
        if valid {
            continue;
        }
        let bytes = fs::read(source.join(name))
            .with_context(|| format!("安装包缺少 {name}，请重新解压完整安装包"))?;
        if let Some(existing) = existing {
            atomic_write(&path.with_extension("invalid.bak"), &existing)?;
        }
        atomic_write(&path, &bytes).with_context(|| format!("无法准备规则数据库 {name}"))?;
    }
    Ok(())
}
