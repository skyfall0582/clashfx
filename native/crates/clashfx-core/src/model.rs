use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const AUTO_GROUP: &str = "CLASHFX_AUTO";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingMode {
    #[default]
    Rule,
    Global,
    Direct,
}

impl RoutingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Global => "global",
            Self::Direct => "direct",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Rule => "规则",
            Self::Global => "全局",
            Self::Direct => "直连",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoPolicy {
    pub enabled: bool,
    pub failure_threshold: u32,
    pub failure_window_secs: u64,
    pub probe_timeout_ms: u64,
    pub probe_urls: Vec<String>,
    pub cooldown_secs: u64,
    pub check_interval_secs: u64,
    pub recovery_enabled: bool,
    pub recovery_successes: u32,
    pub unavailable_retry_secs: u64,
    pub manual_hold_secs: u64,
    pub probe_concurrency: usize,
}

impl Default for AutoPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            failure_threshold: 3,
            failure_window_secs: 30,
            probe_timeout_ms: 3000,
            probe_urls: vec![
                "https://www.gstatic.com/generate_204".into(),
                "https://cp.cloudflare.com/generate_204".into(),
            ],
            cooldown_secs: 60,
            check_interval_secs: 300,
            recovery_enabled: false,
            recovery_successes: 3,
            unavailable_retry_secs: 30,
            manual_hold_secs: 300,
            probe_concurrency: 4,
        }
    }
}

impl AutoPolicy {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=30).contains(&self.failure_threshold),
            "连续失败次数须为 1–30"
        );
        anyhow::ensure!(
            (5..=300).contains(&self.failure_window_secs),
            "失败统计窗口须为 5–300 秒"
        );
        anyhow::ensure!(
            (500..=30000).contains(&self.probe_timeout_ms),
            "检测超时须为 500–30000 毫秒"
        );
        anyhow::ensure!(
            (5..=3600).contains(&self.cooldown_secs),
            "切换冷却须为 5–3600 秒"
        );
        anyhow::ensure!(
            (10..=3600).contains(&self.check_interval_secs),
            "定时检测须为 10–3600 秒"
        );
        anyhow::ensure!(
            (1..=10).contains(&self.recovery_successes),
            "恢复确认次数须为 1–10"
        );
        anyhow::ensure!(
            (5..=3600).contains(&self.unavailable_retry_secs),
            "不可用重试间隔须为 5–3600 秒"
        );
        anyhow::ensure!(self.manual_hold_secs <= 86400, "手动保持时间不能超过一天");
        anyhow::ensure!(
            (1..=16).contains(&self.probe_concurrency),
            "测速并发数须为 1–16"
        );
        anyhow::ensure!(
            !self.probe_urls.is_empty() && self.probe_urls.len() <= 3,
            "请设置 1–3 个检测地址"
        );
        for raw in &self.probe_urls {
            let url = reqwest::Url::parse(raw).map_err(|_| anyhow::anyhow!("检测地址格式无效"))?;
            anyhow::ensure!(
                matches!(url.scheme(), "http" | "https"),
                "检测地址须使用 HTTP(S)"
            );
            anyhow::ensure!(
                url.username().is_empty() && url.password().is_none(),
                "检测地址不能含用户名或密码"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreSettings {
    pub binary_path: String,
    pub controller_port: u16,
    pub mixed_port: u16,
    pub secret: String,
}

impl Default for CoreSettings {
    fn default() -> Self {
        Self {
            binary_path: String::new(),
            controller_port: 19097,
            mixed_port: 17897,
            secret: uuid::Uuid::new_v4().to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub notes: String,
    pub url: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub pool_nodes: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct EntryRepairSettings {
    pub enabled: bool,
    pub resolvers: Vec<String>,
}

impl Default for EntryRepairSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            resolvers: vec![
                "https://dns.alidns.com/resolve".into(),
                "https://cloudflare-dns.com/dns-query".into(),
                "https://dns.google/resolve".into(),
            ],
        }
    }
}

impl EntryRepairSettings {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.resolvers.len() <= 8,
            "At most eight entry DNS resolvers are supported"
        );
        for resolver in &self.resolvers {
            let url = reqwest::Url::parse(resolver)?;
            let local = url.host_str().is_some_and(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            });
            anyhow::ensure!(
                url.host_str().is_some()
                    && (url.scheme() == "https" || (url.scheme() == "http" && local))
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.fragment().is_none(),
                "Entry DNS resolvers require HTTPS (HTTP is allowed on loopback)"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub routing_mode: RoutingMode,
    pub rule_subscription: Option<String>,
    pub core: CoreSettings,
    pub policy: AutoPolicy,
    pub entry_repair: EntryRepairSettings,
    pub subscriptions: Vec<Subscription>,
    pub pool_order: Vec<String>,
    pub last_active: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 4,
            routing_mode: RoutingMode::Rule,
            rule_subscription: None,
            core: CoreSettings::default(),
            policy: AutoPolicy::default(),
            entry_repair: EntryRepairSettings::default(),
            subscriptions: Vec::new(),
            pool_order: Vec::new(),
            last_active: None,
        }
    }
}

impl AppConfig {
    pub fn pool_subscriptions(&self) -> Vec<&Subscription> {
        self.pool_order
            .iter()
            .filter_map(|id| self.subscriptions.iter().find(|sub| &sub.id == id))
            .chain(
                self.subscriptions
                    .iter()
                    .filter(|sub| !self.pool_order.contains(&sub.id)),
            )
            .filter(|sub| !sub.pool_nodes.is_empty())
            .collect()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.schema_version == 4, "不支持此配置版本");
        anyhow::ensure!(
            self.core.controller_port > 1024 && self.core.mixed_port > 1024,
            "端口须大于 1024"
        );
        anyhow::ensure!(
            self.core.controller_port != self.core.mixed_port,
            "控制端口与代理端口不能相同"
        );
        anyhow::ensure!(!self.core.secret.is_empty(), "控制接口密钥不能为空");
        self.policy.validate()?;
        self.entry_repair.validate()?;
        let mut ids = std::collections::HashSet::new();
        for subscription in &self.subscriptions {
            anyhow::ensure!(
                uuid::Uuid::parse_str(&subscription.id).is_ok(),
                "订阅 ID 无效"
            );
            anyhow::ensure!(ids.insert(&subscription.id), "订阅 ID 重复");
            anyhow::ensure!(!subscription.name.trim().is_empty(), "订阅名称不能为空");
        }
        let mut order = std::collections::HashSet::new();
        for id in &self.pool_order {
            anyhow::ensure!(
                self.subscriptions.iter().any(|sub| &sub.id == id),
                "切换池订阅不存在"
            );
            anyhow::ensure!(order.insert(id), "切换池订阅重复");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub key: String,
    pub subscription_id: String,
    pub subscription_name: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct NodeView {
    pub candidate: Candidate,
    pub kind: String,
    pub in_pool: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ProbeResult {
    pub delay_ms: Option<u64>,
    pub checked_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Activity {
    pub at: DateTime<Utc>,
    pub level: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct SwitchTransition {
    pub from: Option<Candidate>,
    pub to: Candidate,
    pub at: DateTime<Utc>,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub revision: u64,
    pub config: AppConfig,
    pub nodes: Vec<NodeView>,
    pub rule_sources: Vec<(String, usize)>,
    pub effective_rule_subscription: Option<String>,
    pub candidates: Vec<Candidate>,
    pub active: Option<Candidate>,
    pub running: bool,
    pub probing: bool,
    pub switching: bool,
    pub checking_nodes: std::collections::BTreeSet<String>,
    pub last_switch: Option<SwitchTransition>,
    pub retry_at: Option<DateTime<Utc>>,
    pub testing_nodes: bool,
    pub busy: bool,
    pub log_connected: bool,
    pub system_proxy: bool,
    pub launch_on_startup: bool,
    pub failure_streak: u32,
    pub switch_count: u64,
    pub phase: String,
    pub last_error: Option<String>,
    pub manual_hold_until: Option<DateTime<Utc>>,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub probes: BTreeMap<String, ProbeResult>,
    pub activity: Vec<Activity>,
    pub core_logs: Vec<Activity>,
    pub data_dir: String,
}

impl Snapshot {
    pub fn new(config: AppConfig, data_dir: String) -> Self {
        Self {
            revision: 0,
            config,
            nodes: Vec::new(),
            rule_sources: Vec::new(),
            effective_rule_subscription: None,
            candidates: Vec::new(),
            active: None,
            running: false,
            probing: false,
            switching: false,
            checking_nodes: Default::default(),
            last_switch: None,
            retry_at: None,
            testing_nodes: false,
            busy: false,
            log_connected: false,
            system_proxy: false,
            launch_on_startup: false,
            failure_streak: 0,
            switch_count: 0,
            phase: "未启动".into(),
            last_error: None,
            manual_hold_until: None,
            cooldown_until: None,
            probes: BTreeMap::new(),
            activity: Vec::new(),
            core_logs: Vec::new(),
            data_dir,
        }
    }
}
