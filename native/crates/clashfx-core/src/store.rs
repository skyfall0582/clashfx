use crate::model::AppConfig;
use anyhow::{Context, Result};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open(path: Option<PathBuf>) -> Result<Self> {
        let root = match path {
            Some(path) => path,
            None => directories::ProjectDirs::from("app", "ClashFx", "ClashFx")
                .context("无法确定配置目录")?
                .data_local_dir()
                .to_path_buf(),
        };
        fs::create_dir_all(root.join("profiles"))?;
        Ok(Self {
            root: dunce::canonicalize(root)?,
        })
    }

    pub fn lock(&self) -> Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("instance.lock"))?;
        file.try_lock_exclusive()
            .context("ClashFx 已在使用这个配置目录")?;
        Ok(file)
    }

    pub fn load(&self) -> Result<AppConfig> {
        let path = self.root.join("settings.json");
        if !path.exists() {
            let config = AppConfig::default();
            self.save(&config)?;
            return Ok(config);
        }
        let original = fs::read(&path)?;
        let mut value: serde_json::Value =
            serde_json::from_slice(&original).context("配置读取失败；原文件已保留")?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1);
        if version == 1 {
            let mut order = Vec::new();
            if let Some(subscriptions) = value
                .get_mut("subscriptions")
                .and_then(serde_json::Value::as_array_mut)
            {
                for subscription in subscriptions {
                    let enabled = subscription
                        .get("enabled")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true);
                    let region = subscription
                        .get("region_filter")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let filter = if region.is_empty() {
                        None
                    } else {
                        Some(
                            regex::RegexBuilder::new(region)
                                .case_insensitive(true)
                                .build()
                                .context("旧地区筛选无效，原配置已保留")?,
                        )
                    };
                    let pool: Vec<_> = subscription
                        .get("selected_nodes")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(serde_json::Value::as_str)
                        .filter(|name| {
                            enabled && filter.as_ref().is_none_or(|filter| filter.is_match(name))
                        })
                        .map(str::to_owned)
                        .collect();
                    if !pool.is_empty()
                        && let Some(id) = subscription.get("id").and_then(serde_json::Value::as_str)
                    {
                        order.push(id.to_owned());
                    }
                    let subscription = subscription.as_object_mut().context("旧订阅格式无效")?;
                    subscription.remove("enabled");
                    subscription.remove("region_filter");
                    subscription.remove("selected_nodes");
                    subscription.insert("pool_nodes".into(), serde_json::to_value(pool)?);
                }
            }
            value["schema_version"] = 2.into();
            value["pool_order"] = serde_json::to_value(order)?;
        }
        if version <= 2
            && let Some(policy) = value
                .get_mut("policy")
                .and_then(serde_json::Value::as_object_mut)
        {
            if policy
                .get("check_interval_secs")
                .and_then(serde_json::Value::as_u64)
                == Some(60)
            {
                policy.insert("check_interval_secs".into(), 300.into());
            }
            policy.insert("recovery_enabled".into(), false.into());
        }
        if version <= 3 {
            if let Some(policy) = value
                .get_mut("policy")
                .and_then(serde_json::Value::as_object_mut)
                && policy
                    .get("probe_timeout_ms")
                    .and_then(serde_json::Value::as_u64)
                    == Some(if version == 3 { 8000 } else { 4000 })
            {
                policy.insert("probe_timeout_ms".into(), 3000.into());
            }
            value["schema_version"] = 4.into();
        }
        let config: AppConfig =
            serde_json::from_value(value).context("配置读取失败；原文件已保留")?;
        config.validate()?;
        if version <= 3 {
            let backup = self.root.join(format!("settings.v{version}.backup.json"));
            if !backup.exists() {
                atomic_write(&backup, &original)?;
            }
            self.save(&config)?;
        }
        Ok(config)
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        config.validate()?;
        atomic_write(
            &self.root.join("settings.json"),
            &serde_json::to_vec_pretty(config)?,
        )
    }

    pub fn profile_path(&self, id: &str) -> Result<PathBuf> {
        anyhow::ensure!(uuid::Uuid::parse_str(id).is_ok(), "订阅 ID 无效");
        Ok(self.root.join("profiles").join(format!("{id}.yaml")))
    }

    pub fn runtime_path(&self) -> PathBuf {
        self.root.join("runtime.yaml")
    }

    pub fn write_profile(&self, id: &str, body: &str) -> Result<()> {
        atomic_write(&self.profile_path(id)?, body.as_bytes())
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("tmp");
    let mut file = File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, path).with_context(|| format!("无法保存 {}", path.display()))?;
    Ok(())
}
