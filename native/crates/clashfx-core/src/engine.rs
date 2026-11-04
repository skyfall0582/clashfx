use crate::{
    autostart,
    child::OwnedChild,
    entry_repair::{self, Repair, Target},
    logfile::LogTail,
    mihomo::{self, Controller},
    model::{
        Activity, AppConfig, AutoPolicy, Candidate, CoreSettings, ProbeResult, RoutingMode,
        Snapshot, SubscriptionUsage, SwitchTransition,
    },
    policy::{self, CoreLog, FailureMonitor, ProbeReason},
    proxy_compat::ProxyCompatibility,
    store::{Store, atomic_write},
    subscription::{self, Profile},
    sysproxy,
};
use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use futures_util::{SinkExt, StreamExt, stream};
use std::{
    collections::{BTreeMap, HashMap},
    net::IpAddr,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

#[derive(Clone)]
pub enum Command {
    Start,
    Stop,
    ImportUrl {
        name: String,
        url: String,
        notes: String,
    },
    ImportFile {
        name: String,
        path: PathBuf,
        notes: String,
    },
    UpdateSubscription(String),
    RemoveSubscription(String),
    EditSubscription {
        id: String,
        name: String,
        url: Option<String>,
        notes: String,
    },
    ReplaceSubscriptionFile {
        id: String,
        path: PathBuf,
    },
    MovePoolSubscriptionTo {
        id: String,
        index: usize,
    },
    MovePoolNodeTo {
        id: String,
        name: String,
        index: usize,
    },
    SetPoolNodes {
        id: String,
        names: Vec<String>,
        in_pool: bool,
    },
    SavePolicy(AutoPolicy),
    SetRoutingMode(RoutingMode),
    SetRuleSubscription(String),
    SaveCore {
        binary_path: String,
        controller_port: u16,
        mixed_port: u16,
    },
    SelectNode(String),
    TestNodes(Option<String>),
    Probe,
    Resume,
    SystemProxy(bool),
    SetLaunchOnStartup(bool),
    ClearLogs,
    ClearLogSource {
        core: bool,
    },
    Shutdown,
}

struct Request {
    command: Command,
    reply: Option<oneshot::Sender<std::result::Result<(), String>>>,
}

struct BackendInner {
    sender: mpsc::UnboundedSender<Request>,
    snapshot: Arc<RwLock<Snapshot>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Drop for BackendInner {
    fn drop(&mut self) {
        self.sender
            .send(Request {
                command: Command::Shutdown,
                reply: None,
            })
            .ok();
        if let Ok(thread) = self.thread.get_mut()
            && let Some(thread) = thread.take()
        {
            thread.join().ok();
        }
    }
}

#[derive(Clone)]
pub struct Backend(Arc<BackendInner>);

impl Backend {
    pub fn launch(store: Store) -> Result<Self> {
        let lock = store.lock()?;
        sysproxy::restore(&store).context("无法恢复上次运行的系统代理")?;
        let config = store.load()?;
        let profiles = subscription::load_profiles(&config, &store)?;
        let mut initial = Snapshot::new(config, store.root.to_string_lossy().into_owned());
        match autostart::is_enabled() {
            Ok(enabled) => initial.launch_on_startup = enabled,
            Err(error) => initial.last_error = Some(format!("无法读取开机自启设置：{error}")),
        }
        initial.nodes = subscription::node_views(&initial.config, &profiles)?;
        initial.candidates = subscription::candidates(&initial.config, &profiles)?;
        initial.rule_sources = subscription::rule_sources(&initial.config, &profiles);
        initial.effective_rule_subscription = subscription::rule_source(&initial.config, &profiles);
        let snapshot = Arc::new(RwLock::new(initial));
        let (sender, receiver) = mpsc::unbounded_channel();
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let shared = snapshot.clone();
        let thread = std::thread::Builder::new()
            .name("clashfx-engine".into())
            .spawn(move || {
                let _lock = lock;
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(3)
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => runtime.block_on(
                        Engine::new(store, shared, profiles, event_sender)
                            .run(receiver, event_receiver),
                    ),
                    Err(error) => {
                        let mut snapshot = shared.write().unwrap();
                        snapshot.last_error = Some(format!("后台启动失败：{error}"));
                        snapshot.revision += 1;
                    }
                }
            })?;
        Ok(Self(Arc::new(BackendInner {
            sender,
            snapshot,
            thread: Mutex::new(Some(thread)),
        })))
    }

    pub fn snapshot(&self) -> Snapshot {
        self.0.snapshot.read().unwrap().clone()
    }
    pub fn send(&self, command: Command) {
        self.0
            .sender
            .send(Request {
                command,
                reply: None,
            })
            .ok();
    }

    pub fn request(
        &self,
        command: Command,
    ) -> impl std::future::Future<Output = std::result::Result<(), String>> + Send + 'static {
        let (reply, receiver) = oneshot::channel();
        self.0
            .sender
            .send(Request {
                command,
                reply: Some(reply),
            })
            .ok();
        async move {
            receiver
                .await
                .unwrap_or_else(|_| Err("后台已退出，请重新打开应用".into()))
        }
    }
    pub fn shutdown(&self) {
        self.send(Command::Shutdown);
        if let Some(thread) = self.0.thread.lock().unwrap().take() {
            thread.join().ok();
        }
    }
}

struct ProbeReport {
    reason: ProbeReason,
    choice: Option<Candidate>,
    active_healthy: bool,
    results: BTreeMap<String, ProbeResult>,
}

enum Internal {
    Log(u64, CoreLog),
    LogConnection(u64, bool),
    ProbeFinished(u64, ProbeReport),
    ProbeProgress(u64, String, Option<ProbeResult>),
    HealthFinished(u64),
    LatencyFinished(u64, BTreeMap<String, ProbeResult>),
    EntryRepaired(u64, String, Result<Option<Repair>>),
}

struct Engine {
    store: Store,
    shared: Arc<RwLock<Snapshot>>,
    state: Snapshot,
    profiles: HashMap<String, Profile>,
    child: Option<OwnedChild>,
    proxy_guard: Option<sysproxy::ProxyGuard>,
    proxy_compatibility: Option<ProxyCompatibility>,
    controller: Controller,
    events: mpsc::UnboundedSender<Internal>,
    log_task: Option<JoinHandle<()>>,
    log_tail: LogTail,
    probe_task: Option<JoinHandle<()>>,
    health_task: Option<JoinHandle<()>>,
    latency_task: Option<JoinHandle<()>>,
    latency_generation: u64,
    generation: u64,
    core_generation: u64,
    monitor: FailureMonitor,
    next_check: chrono::DateTime<Utc>,
    next_failure_check: chrono::DateTime<Utc>,
    next_proxy_check: chrono::DateTime<Utc>,
    retry_at: Option<chrono::DateTime<Utc>>,
    recovery: Option<(String, u32)>,
    subscription_retry: HashMap<String, chrono::DateTime<Utc>>,
    entry_pins: BTreeMap<String, IpAddr>,
    entry_retry: HashMap<String, chrono::DateTime<Utc>>,
    entry_task: Option<JoinHandle<()>>,
    entry_generation: u64,
    last_publish: std::time::Instant,
    dirty: bool,
}

impl Engine {
    fn new(
        store: Store,
        shared: Arc<RwLock<Snapshot>>,
        profiles: HashMap<String, Profile>,
        events: mpsc::UnboundedSender<Internal>,
    ) -> Self {
        let state = shared.read().unwrap().clone();
        let controller =
            Controller::new(&state.config.core).expect("validated controller settings");
        let log_tail = LogTail::new(store.root.join("core.log"));
        Self {
            store,
            shared,
            state,
            profiles,
            child: None,
            proxy_guard: None,
            proxy_compatibility: None,
            controller,
            events,
            log_task: None,
            log_tail,
            probe_task: None,
            health_task: None,
            latency_task: None,
            latency_generation: 0,
            generation: 0,
            core_generation: 0,
            monitor: FailureMonitor::default(),
            next_check: Utc::now(),
            next_failure_check: Utc::now(),
            next_proxy_check: Utc::now(),
            retry_at: None,
            recovery: None,
            subscription_retry: HashMap::new(),
            entry_pins: BTreeMap::new(),
            entry_retry: HashMap::new(),
            entry_task: None,
            entry_generation: 0,
            last_publish: std::time::Instant::now(),
            dirty: false,
        }
    }

    fn publish(&mut self) {
        self.state.retry_at = self.retry_at;
        self.dirty = false;
        self.last_publish = std::time::Instant::now();
        self.state.revision = self.state.revision.wrapping_add(1);
        *self.shared.write().unwrap() = self.state.clone();
    }

    fn redact(&self, message: &str) -> String {
        let mut result = message.replace(&self.state.config.core.secret, "[密钥已隐藏]");
        for subscription in &self.state.config.subscriptions {
            if let Some(url) = &subscription.url {
                result = result.replace(url, "[订阅地址已隐藏]");
            }
        }
        result
    }

    fn activity(&mut self, level: &str, message: impl Into<String>) {
        let message = self.redact(&message.into());
        self.state.activity.insert(
            0,
            Activity {
                at: Utc::now(),
                level: level.into(),
                message,
            },
        );
        self.state.activity.truncate(500);
    }

    fn error(&mut self, error: anyhow::Error) {
        let message = self.redact(&format!("{error:#}"));
        self.state.last_error = Some(message.clone());
        self.activity("error", message);
    }

    fn cancel_probe(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(task) = self.probe_task.take() {
            task.abort();
        }
        if let Some(task) = self.health_task.take() {
            task.abort();
        }
        self.state.checking_nodes.clear();
        self.state.probing = false;
        self.recovery = None;
    }

    fn cancel_latency(&mut self) {
        self.latency_generation = self.latency_generation.wrapping_add(1);
        if let Some(task) = self.latency_task.take() {
            task.abort();
        }
        self.state.testing_nodes = false;
    }

    fn cancel_entry_repair(&mut self) {
        self.entry_generation = self.entry_generation.wrapping_add(1);
        if let Some(task) = self.entry_task.take() {
            task.abort();
        }
    }

    fn health_nodes(&self) -> Vec<Candidate> {
        self.state.active.iter().cloned().collect()
    }

    fn entry_targets(&self) -> Vec<Target> {
        self.state
            .nodes
            .iter()
            .filter_map(|view| {
                let candidate = &view.candidate;
                let node = self
                    .profiles
                    .get(&candidate.subscription_id)?
                    .nodes
                    .iter()
                    .find(|node| node.name == candidate.name)?;
                Target::new(candidate.key.clone(), &node.value)
            })
            .collect()
    }

    fn begin_entry_repair(&mut self) -> Result<()> {
        if !self.state.running
            || self.state.config.routing_mode == RoutingMode::Direct
            || !self.state.config.entry_repair.enabled
            || self.entry_task.is_some()
            || self.state.testing_nodes
            || self.state.probing
            || self.health_task.is_some()
        {
            return Ok(());
        }
        let now = Utc::now();
        let targets = self.entry_targets();
        let candidates = self.health_nodes();
        let target = candidates.iter().find_map(|node| {
            let failed = self
                .state
                .probes
                .get(&node.key)
                .is_some_and(|probe| probe.delay_ms.is_none() && probe.checked_at.is_some());
            let target = targets.iter().find(|target| target.key == node.key)?;
            (failed
                && self
                    .entry_retry
                    .get(&target.host)
                    .is_none_or(|retry| *retry <= now))
            .then(|| target.clone())
        });
        let Some(target) = target else {
            return Ok(());
        };
        let runtime =
            serde_yaml_ng::from_str(&std::fs::read_to_string(self.store.runtime_path())?)?;
        let binary = mihomo::find_binary(&self.state.config.core)?;
        let settings = self.state.config.entry_repair.clone();
        let policy = self.state.config.policy.clone();
        let root = self.store.root.join("entry-check");
        let host = target.host.clone();
        self.entry_retry
            .insert(host.clone(), now + Duration::seconds(60));
        self.entry_generation = self.entry_generation.wrapping_add(1);
        let generation = self.entry_generation;
        let events = self.events.clone();
        let active = self.state.active.as_ref().context("没有当前节点")?;
        self.activity(
            "info",
            format!(
                "{} · {} 连接检测失败，正在查找可用入口",
                active.subscription_name, active.name
            ),
        );
        self.entry_task = Some(tokio::spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                entry_repair::repair(root, binary, settings, policy, target, runtime),
            )
            .await
            .context("入口修复超时")
            .and_then(|result| result);
            events
                .send(Internal::EntryRepaired(generation, host, result))
                .ok();
        }));
        self.publish();
        Ok(())
    }

    async fn apply_entry_repair(&mut self, repair: Repair) -> Result<()> {
        let Some(active) = self
            .state
            .active
            .clone()
            .filter(|active| repair.delays.contains_key(&active.key))
        else {
            return Ok(());
        };
        let result = probe_one(&self.controller, &self.state.config.policy, &active).await;
        let recovered = result.delay_ms.is_some();
        self.state.probes.insert(active.key.clone(), result);
        if recovered {
            self.next_check =
                Utc::now() + Duration::seconds(self.state.config.policy.check_interval_secs as i64);
            self.monitor.reset();
            self.state.failure_streak = 0;
            self.retry_at = None;
            self.state.phase = "运行中".into();
            self.activity(
                "info",
                format!(
                    "{} · {} 连通性已恢复，保持当前入口",
                    active.subscription_name, active.name
                ),
            );
            return Ok(());
        }
        if self.entry_pins.get(&repair.host) == Some(&repair.ip) {
            return Ok(());
        }
        let mut pins = self.entry_pins.clone();
        pins.insert(repair.host.clone(), repair.ip);
        let runtime = subscription::build_runtime_for(
            &self.state.config,
            &self.profiles,
            self.state.active.as_ref(),
        )?;
        let runtime = entry_repair::overlay(&runtime, &pins)?;
        let previous = std::fs::read(self.store.runtime_path())?;
        let affected: Vec<_> = self
            .entry_targets()
            .into_iter()
            .filter(|target| target.host == repair.host)
            .map(|target| target.key)
            .collect();
        self.cancel_probe();
        self.cancel_latency();
        atomic_write(&self.store.runtime_path(), runtime.as_bytes())?;
        let apply = async {
            self.controller.reload(&self.store.runtime_path()).await?;
            self.controller.select(&active.key).await?;
            for key in repair.delays.keys() {
                self.controller
                    .probe(key, &self.state.config.policy)
                    .await?;
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(error) = apply {
            atomic_write(&self.store.runtime_path(), &previous)?;
            if let Err(rollback) = self.controller.reload(&self.store.runtime_path()).await {
                self.stop().await?;
                anyhow::bail!("入口修复失败：{error}；恢复原配置失败：{rollback}");
            }
            self.controller.select(&active.key).await?;
            return Err(error.context("入口修复未通过复检，已恢复原配置"));
        }
        self.entry_pins = pins;
        for key in affected {
            self.state.probes.remove(&key);
        }
        for (key, delay) in repair.delays {
            self.state.probes.insert(
                key,
                ProbeResult {
                    delay_ms: Some(delay),
                    checked_at: Some(Utc::now()),
                    error: None,
                },
            );
        }
        self.monitor.reset();
        self.state.failure_streak = 0;
        self.retry_at = None;
        self.next_check =
            Utc::now() + Duration::seconds(self.state.config.policy.check_interval_secs as i64);
        self.state.phase = "运行中".into();
        self.activity(
            "info",
            format!(
                "{} · {} 已应用通过检测的入口地址",
                active.subscription_name, active.name
            ),
        );
        Ok(())
    }

    async fn run(
        mut self,
        mut commands: mpsc::UnboundedReceiver<Request>,
        mut events: mpsc::UnboundedReceiver<Internal>,
    ) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                command = commands.recv() => {
                    let Some(Request { command, reply }) = command else { break; };
                    if matches!(command, Command::Shutdown) { break; }
                    self.state.last_error = None;
                    self.state.busy = true;
                    self.publish();
                    let result = self.command(command).await;
                    let outcome = result.as_ref().map(|_| ()).map_err(|error| self.redact(&format!("{error:#}")));
                    if let Err(error) = result { self.error(error); }
                    self.state.busy = false;
                    if !self.state.probing && self.state.phase != "池内节点全部不可用" {
                        self.state.phase = if self.state.running { "运行中" } else { "未启动" }.into();
                    }
                    self.publish();
                    if let Some(reply) = reply { reply.send(outcome).ok(); }
                }
                event = events.recv() => {
                    if let Some(event) = event {
                        if let Err(error) = self.event(event).await { self.error(error); }
                        self.dirty = true;
                        if self.last_publish.elapsed() >= std::time::Duration::from_millis(100) { self.publish(); }
                    }
                }
                _ = interval.tick() => {
                    if let Err(error) = self.tick().await { self.error(error); self.publish(); }
                }
            }
        }
        if let Err(error) = self.stop().await {
            self.error(error);
        }
        self.publish();
    }

    async fn command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Start => self.start().await?,
            Command::Stop => self.stop().await?,
            Command::ImportUrl { name, url, notes } => {
                let parsed = subscription::validate_url(&url)?;
                anyhow::ensure!(
                    !self
                        .state
                        .config
                        .subscriptions
                        .iter()
                        .any(|sub| sub.url.as_deref() == Some(url.trim())),
                    "此订阅链接已存在，请选择已有订阅更新"
                );
                let name = if name.trim().is_empty() {
                    parsed.host_str().unwrap_or("订阅").to_owned()
                } else {
                    name.trim().to_owned()
                };
                self.state.phase = "正在下载订阅".into();
                self.publish();
                let downloaded = subscription::download(&url).await?;
                self.import(
                    name,
                    Some(url.trim().into()),
                    None,
                    notes,
                    downloaded.body,
                    downloaded.usage,
                )
                .await?;
            }
            Command::ImportFile { name, path, notes } => {
                let body = std::fs::read_to_string(&path).context("无法读取订阅文件")?;
                let name = if name.trim().is_empty() {
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                } else {
                    name.trim().into()
                };
                self.import(
                    name,
                    None,
                    Some(path.to_string_lossy().into_owned()),
                    notes,
                    body,
                    None,
                )
                .await?;
            }
            Command::UpdateSubscription(id) => {
                let subscription = self
                    .state
                    .config
                    .subscriptions
                    .iter()
                    .find(|sub| sub.id == id)
                    .context("订阅不存在")?;
                let url = subscription.url.clone().context("本地订阅请重新导入文件")?;
                let name = subscription.name.clone();
                self.state.phase = format!("正在更新 {name}");
                self.publish();
                let downloaded = subscription::download(&url)
                    .await
                    .with_context(|| format!("更新 {name} 失败"))?;
                let body = downloaded.body;
                let profile = subscription::parse_profile(&body)?;
                let mut config = self.state.config.clone();
                let sub = config
                    .subscriptions
                    .iter_mut()
                    .find(|sub| sub.id == id)
                    .unwrap();
                sub.updated_at = Utc::now();
                sub.usage = downloaded.usage;
                if self
                    .profiles
                    .get(&id)
                    .is_some_and(|previous| previous.root == profile.root)
                {
                    self.store.save(&config)?;
                    self.state.config = config;
                    return Ok(());
                }
                let mut profiles = self.profiles.clone();
                profiles.insert(id.clone(), profile);
                let old = std::fs::read(self.store.profile_path(&id)?)?;
                self.store.write_profile(&id, &body)?;
                if let Err(error) = self.commit(config, profiles).await {
                    atomic_write(&self.store.profile_path(&id)?, &old)?;
                    return Err(error);
                }
                self.activity("info", format!("已更新 {name}；保留原有节点选择"));
            }
            Command::RemoveSubscription(id) => {
                let mut config = self.state.config.clone();
                let name = config
                    .subscriptions
                    .iter()
                    .find(|sub| sub.id == id)
                    .context("订阅不存在")?
                    .name
                    .clone();
                config.subscriptions.retain(|sub| sub.id != id);
                config.pool_order.retain(|entry| entry != &id);
                if config.rule_subscription.as_ref() == Some(&id) {
                    config.rule_subscription = None;
                }
                let mut profiles = self.profiles.clone();
                profiles.remove(&id);
                self.commit(config, profiles).await?;
                std::fs::remove_file(self.store.profile_path(&id)?).ok();
                self.activity("info", format!("已移除 {name}"));
            }
            Command::EditSubscription {
                id,
                name,
                url,
                notes,
            } => {
                let old = self
                    .state
                    .config
                    .subscriptions
                    .iter()
                    .find(|sub| sub.id == id)
                    .context("订阅不存在")?
                    .clone();
                let name = name.trim().to_owned();
                anyhow::ensure!(!name.is_empty(), "请输入订阅名称");
                anyhow::ensure!(
                    !self
                        .state
                        .config
                        .subscriptions
                        .iter()
                        .any(|sub| sub.id != id && sub.name == name),
                    "已有同名订阅，请使用不同名称"
                );
                let url = url
                    .map(|url| url.trim().to_owned())
                    .filter(|url| !url.is_empty());
                anyhow::ensure!(old.url.is_none() || url.is_some(), "订阅链接不能为空");
                if let Some(url) = &url {
                    subscription::validate_url(url)?;
                    anyhow::ensure!(
                        !self
                            .state
                            .config
                            .subscriptions
                            .iter()
                            .any(|sub| sub.id != id && sub.url.as_ref() == Some(url)),
                        "此链接已属于另一个订阅"
                    );
                }
                let mut config = self.state.config.clone();
                let sub = config
                    .subscriptions
                    .iter_mut()
                    .find(|sub| sub.id == id)
                    .unwrap();
                sub.name = name.clone();
                sub.url = url.clone();
                sub.notes = notes;
                if url.is_some() {
                    sub.source_path = None;
                }
                config.validate()?;
                if old.name == name && old.url == url {
                    self.store.save(&config)?;
                    self.state.config = config;
                } else if old.url != url {
                    self.state.phase = format!("正在验证 {name} 的新链接");
                    self.publish();
                    let downloaded =
                        subscription::download(url.as_deref().context("订阅链接不能为空")?).await?;
                    config
                        .subscriptions
                        .iter_mut()
                        .find(|sub| sub.id == id)
                        .unwrap()
                        .usage = downloaded.usage;
                    self.replace_profile(&id, config, downloaded.body).await?;
                } else {
                    self.commit(config, self.profiles.clone()).await?;
                }
                self.activity("info", format!("已保存 {name}"));
            }
            Command::ReplaceSubscriptionFile { id, path } => {
                let body = std::fs::read_to_string(&path).context("无法读取订阅文件")?;
                let mut config = self.state.config.clone();
                let sub = config
                    .subscriptions
                    .iter_mut()
                    .find(|sub| sub.id == id)
                    .context("订阅不存在")?;
                anyhow::ensure!(sub.url.is_none(), "远程订阅请修改链接后更新");
                sub.source_path = Some(path.to_string_lossy().into_owned());
                self.replace_profile(&id, config, body).await?;
                self.activity("info", "已替换本地文件；保留切换池");
            }
            Command::MovePoolSubscriptionTo { id, index } => {
                let mut config = self.state.config.clone();
                let mut order: Vec<_> = config
                    .pool_subscriptions()
                    .iter()
                    .map(|sub| sub.id.clone())
                    .collect();
                let from = order
                    .iter()
                    .position(|entry| entry == &id)
                    .context("订阅不在切换池中")?;
                let target = index.min(order.len() - 1);
                let item = order.remove(from);
                order.insert(target, item);
                config.pool_order = order;
                self.commit(config, self.profiles.clone()).await?;
            }
            Command::MovePoolNodeTo { id, name, index } => {
                let mut config = self.state.config.clone();
                let selected = &mut config
                    .subscriptions
                    .iter_mut()
                    .find(|sub| sub.id == id)
                    .context("订阅不存在")?
                    .pool_nodes;
                let from = selected
                    .iter()
                    .position(|node| node == &name)
                    .context("节点不在切换池中")?;
                let index = index.min(selected.len() - 1);
                let item = selected.remove(from);
                selected.insert(index, item);
                self.commit(config, self.profiles.clone()).await?;
            }
            Command::SetPoolNodes { id, names, in_pool } => {
                let mut config = self.state.config.clone();
                let sub = config
                    .subscriptions
                    .iter_mut()
                    .find(|sub| sub.id == id)
                    .context("订阅不存在")?;
                if in_pool {
                    let profile = self.profiles.get(&id).context("缺少订阅缓存")?;
                    anyhow::ensure!(
                        names
                            .iter()
                            .all(|name| profile.nodes.iter().any(|node| &node.name == name)),
                        "节点列表已更新，请重新添加"
                    );
                    for name in names {
                        if !sub.pool_nodes.contains(&name) {
                            sub.pool_nodes.push(name);
                        }
                    }
                } else {
                    sub.pool_nodes.retain(|name| !names.contains(name));
                }
                if sub.pool_nodes.is_empty() {
                    config.pool_order.retain(|entry| entry != &id);
                } else if !config.pool_order.contains(&id) {
                    config.pool_order.push(id);
                }
                self.commit(config, self.profiles.clone()).await?;
            }
            Command::SavePolicy(policy) => {
                let mut config = self.state.config.clone();
                config.policy = policy;
                config.validate()?;
                self.store.save(&config)?;
                self.cancel_probe();
                self.state.config = config;
                self.monitor.reset();
                self.state.failure_streak = 0;
                self.next_check = Utc::now()
                    + Duration::seconds(self.state.config.policy.check_interval_secs as i64);
                self.activity(
                    "info",
                    if self.state.config.policy.enabled {
                        "自动切换已开启"
                    } else {
                        "自动切换已暂停"
                    },
                );
            }
            Command::SaveCore {
                binary_path,
                controller_port,
                mixed_port,
            } => {
                anyhow::ensure!(!self.state.running, "请先停止内核，再修改内核或端口");
                let mut config = self.state.config.clone();
                config.core = CoreSettings {
                    binary_path,
                    controller_port,
                    mixed_port,
                    ..config.core.clone()
                };
                config.validate()?;
                self.store.save(&config)?;
                self.controller = Controller::new(&config.core)?;
                self.state.config = config;
                self.activity("info", "已保存内核设置");
            }
            Command::SetRoutingMode(mode) => {
                let mut config = self.state.config.clone();
                config.routing_mode = mode;
                self.commit(config, self.profiles.clone()).await?;
                self.activity("info", format!("已切换为{}模式", mode.label()));
            }
            Command::SetRuleSubscription(id) => {
                anyhow::ensure!(
                    self.state
                        .rule_sources
                        .iter()
                        .any(|(source, _)| source == &id),
                    "该订阅没有可用的分流规则"
                );
                let mut config = self.state.config.clone();
                config.rule_subscription = Some(id);
                self.commit(config, self.profiles.clone()).await?;
                self.describe_rules();
            }
            Command::SelectNode(key) => {
                let node = self
                    .state
                    .nodes
                    .iter()
                    .find(|node| node.candidate.key == key)
                    .context("节点已失效，请更新订阅")?
                    .candidate
                    .clone();
                if !self.state.running {
                    self.start_with(Some(node.clone())).await?;
                }
                self.cancel_probe();
                self.cancel_entry_repair();
                self.switch(node, "手动选择").await?;
                self.state.manual_hold_until = Some(
                    Utc::now()
                        + Duration::seconds(self.state.config.policy.manual_hold_secs as i64),
                );
            }
            Command::TestNodes(subscription) => {
                anyhow::ensure!(self.state.running, "请先选择节点启动连接，再测速");
                self.check_system_proxy().await?;
                self.cancel_latency();
                self.state.testing_nodes = true;
                let nodes: Vec<_> = self
                    .state
                    .nodes
                    .iter()
                    .filter(|node| {
                        subscription
                            .as_ref()
                            .is_none_or(|id| id == &node.candidate.subscription_id)
                    })
                    .map(|node| node.candidate.clone())
                    .collect();
                let controller = self.controller.clone();
                let policy = self.state.config.policy.clone();
                let generation = self.latency_generation;
                let active_key = self.state.active.as_ref().map(|node| node.key.clone());
                let events = self.events.clone();
                self.latency_task = Some(tokio::spawn(async move {
                    let results = stream::iter(nodes)
                        .map(|node| {
                            let controller = controller.clone();
                            let policy = policy.clone();
                            let active = active_key.as_ref() == Some(&node.key);
                            async move {
                                let result = if active {
                                    confirm_one(&controller, &policy, &node).await
                                } else {
                                    probe_one(&controller, &policy, &node).await
                                };
                                (node.key, result)
                            }
                        })
                        .buffer_unordered(policy.probe_concurrency)
                        .collect()
                        .await;
                    events
                        .send(Internal::LatencyFinished(generation, results))
                        .ok();
                }));
            }
            Command::Probe => {
                anyhow::ensure!(
                    !self.state.candidates.is_empty(),
                    "请先向自动切换池添加节点"
                );
                anyhow::ensure!(self.state.running, "请先启动内核");
                self.state.manual_hold_until = None;
                self.cancel_probe();
                self.begin_probe(ProbeReason::Manual).await?;
            }
            Command::Resume => {
                anyhow::ensure!(
                    !self.state.candidates.is_empty(),
                    "请先向自动切换池添加节点"
                );
                self.state.manual_hold_until = None;
                self.state.cooldown_until = None;
                let mut config = self.state.config.clone();
                config.policy.enabled = true;
                self.store.save(&config)?;
                self.state.config = config;
                self.activity("info", "已恢复自动切换");
                if self.state.running {
                    self.cancel_probe();
                    self.begin_probe(ProbeReason::Manual).await?;
                }
            }
            Command::SystemProxy(enabled) => {
                if enabled {
                    self.start().await?;
                    if !self.state.system_proxy {
                        let compatibility =
                            ProxyCompatibility::start(self.state.config.core.mixed_port)?;
                        let guard =
                            sysproxy::enable(&self.store, self.state.config.core.mixed_port);
                        match guard {
                            Ok(guard) => {
                                self.proxy_guard = Some(guard);
                                self.proxy_compatibility = Some(compatibility);
                            }
                            Err(error) => {
                                compatibility.stop().await;
                                return Err(error);
                            }
                        }
                    }
                } else {
                    sysproxy::restore(&self.store)?;
                    if let Some(compatibility) = self.proxy_compatibility.take() {
                        compatibility.stop().await;
                    }
                    self.proxy_guard = None;
                }
                self.state.system_proxy = enabled;
                self.activity(
                    "info",
                    if enabled {
                        "系统代理已开启"
                    } else {
                        "系统代理已恢复"
                    },
                );
            }
            Command::SetLaunchOnStartup(enabled) => {
                autostart::set_enabled(enabled, &self.store.root).context("无法设置开机自启")?;
                self.state.launch_on_startup = enabled;
                self.activity(
                    "info",
                    if enabled {
                        "开机自启已开启"
                    } else {
                        "开机自启已关闭"
                    },
                );
            }
            Command::ClearLogs => {
                self.log_tail.skip_to_end();
                self.state.activity.clear();
                self.state.core_logs.clear();
            }
            Command::ClearLogSource { core } => {
                if core {
                    self.log_tail.skip_to_end();
                    self.state.core_logs.clear();
                } else {
                    self.state.activity.clear();
                }
            }
            Command::Shutdown => {}
        }
        if !self.state.probing {
            self.state.phase = if self.state.running {
                "运行中"
            } else {
                "未启动"
            }
            .into();
        }
        Ok(())
    }

    async fn replace_profile(
        &mut self,
        id: &str,
        mut config: AppConfig,
        body: String,
    ) -> Result<()> {
        let profile = subscription::parse_profile(&body)?;
        config
            .subscriptions
            .iter_mut()
            .find(|sub| sub.id == id)
            .context("订阅不存在")?
            .updated_at = Utc::now();
        let mut profiles = self.profiles.clone();
        profiles.insert(id.to_owned(), profile);
        let old = std::fs::read(self.store.profile_path(id)?)?;
        self.store.write_profile(id, &body)?;
        if let Err(error) = self.commit(config, profiles).await {
            atomic_write(&self.store.profile_path(id)?, &old)?;
            return Err(error);
        }
        Ok(())
    }

    async fn import(
        &mut self,
        name: String,
        url: Option<String>,
        source_path: Option<String>,
        notes: String,
        body: String,
        usage: Option<SubscriptionUsage>,
    ) -> Result<()> {
        let profile = subscription::parse_profile(&body)?;
        let base = name;
        let mut name = base.clone();
        let mut suffix = 2;
        while self
            .state
            .config
            .subscriptions
            .iter()
            .any(|sub| sub.name == name)
        {
            name = format!("{base} ({suffix})");
            suffix += 1;
        }
        let mut subscription = subscription::new_subscription(name.clone(), url, &profile);
        subscription.source_path = source_path;
        subscription.notes = notes;
        subscription.usage = usage;
        let id = subscription.id.clone();
        self.store.write_profile(&id, &body)?;
        let mut config = self.state.config.clone();
        config.subscriptions.push(subscription);
        let mut profiles = self.profiles.clone();
        profiles.insert(id.clone(), profile);
        if let Err(error) = self.commit(config, profiles).await {
            std::fs::remove_file(self.store.profile_path(&id)?).ok();
            return Err(error);
        }
        self.activity("info", format!("已导入 {name}"));
        Ok(())
    }

    async fn commit(
        &mut self,
        mut config: AppConfig,
        profiles: HashMap<String, Profile>,
    ) -> Result<()> {
        config.validate()?;
        let nodes = subscription::node_views(&config, &profiles)?;
        let candidates = subscription::candidates(&config, &profiles)?;
        let active = self
            .state
            .active
            .as_ref()
            .and_then(|active| {
                if self.state.manual_hold_until.is_some()
                    || candidates.is_empty()
                    || config.routing_mode != self.state.config.routing_mode
                    || config.rule_subscription != self.state.config.rule_subscription
                {
                    nodes
                        .iter()
                        .find(|node| node.candidate.key == active.key)
                        .map(|node| &node.candidate)
                } else {
                    candidates.iter().find(|node| node.key == active.key)
                }
            })
            .cloned()
            .or_else(|| candidates.first().cloned());
        config.last_active = active.as_ref().map(|node| node.key.clone());
        self.cancel_probe();
        self.cancel_latency();
        self.cancel_entry_repair();
        if self.state.running && active.is_some() {
            let body = subscription::build_runtime_for(&config, &profiles, active.as_ref())?;
            let body = entry_repair::overlay(&body, &self.entry_pins)?;
            let next = self.store.root.join("next.yaml");
            atomic_write(&next, body.as_bytes())?;
            mihomo::validate(&mihomo::find_binary(&config.core)?, &self.store, &next).await?;
            let previous = std::fs::read(self.store.runtime_path())?;
            atomic_write(&self.store.runtime_path(), body.as_bytes())?;
            let apply = async {
                self.controller.reload(&self.store.runtime_path()).await?;
                self.controller
                    .select(&active.as_ref().context("没有池内节点")?.key)
                    .await?;
                self.store.save(&config)?;
                Ok::<(), anyhow::Error>(())
            }
            .await;
            if let Err(error) = apply {
                atomic_write(&self.store.runtime_path(), &previous)?;
                if let Err(rollback) = self.controller.reload(&self.store.runtime_path()).await {
                    self.stop().await?;
                    anyhow::bail!("配置应用失败：{error}；恢复失败，内核已停止：{rollback}");
                }
                if let Some(previous) = &self.state.active {
                    self.controller.select(&previous.key).await?;
                }
                return Err(error);
            }
        } else {
            self.store.save(&config)?;
            if self.state.running {
                self.stop().await?;
            }
        }
        self.state.rule_sources = subscription::rule_sources(&config, &profiles);
        self.state.effective_rule_subscription = subscription::rule_source(&config, &profiles);
        self.state.config = config;
        self.profiles = profiles;
        self.state.nodes = nodes;
        let hosts: std::collections::BTreeSet<_> = self
            .entry_targets()
            .into_iter()
            .map(|target| target.host)
            .collect();
        self.entry_pins.retain(|host, _| hosts.contains(host));
        self.entry_retry.clear();
        self.state.candidates = candidates;
        self.state.probes.retain(|key, _| {
            self.state
                .nodes
                .iter()
                .any(|node| &node.candidate.key == key)
        });
        self.state.active = if self.state.running { active } else { None };
        self.monitor.reset();
        self.state.failure_streak = 0;
        self.retry_at = None;
        self.next_check = Utc::now();
        Ok(())
    }

    async fn start(&mut self) -> Result<()> {
        self.start_with(None).await
    }

    async fn start_with(&mut self, manual: Option<Candidate>) -> Result<()> {
        if self.state.running {
            return Ok(());
        }
        let explicit = manual.is_some();
        let active = manual
            .or_else(|| {
                self.state
                    .config
                    .last_active
                    .as_ref()
                    .and_then(|key| {
                        self.state
                            .nodes
                            .iter()
                            .find(|node| &node.candidate.key == key)
                            .map(|node| node.candidate.clone())
                    })
                    .or_else(|| self.state.candidates.first().cloned())
                    .or_else(|| self.state.nodes.first().map(|node| node.candidate.clone()))
            })
            .context("请先导入含有可用节点的订阅")?;
        let runtime =
            subscription::build_runtime_for(&self.state.config, &self.profiles, Some(&active))?;
        let runtime = entry_repair::overlay(&runtime, &self.entry_pins)?;
        atomic_write(&self.store.runtime_path(), runtime.as_bytes())?;
        self.state.phase = "正在启动内核".into();
        self.publish();
        let mut child = mihomo::start(&self.state.config.core, &self.store).await?;
        if let Err(error) = self.controller.select(&active.key).await {
            child.kill().await.ok();
            return Err(error);
        }
        self.child = Some(child);
        self.state.running = true;
        self.state.active = Some(active);
        self.core_generation += 1;
        self.log_task = Some(spawn_log_reader(
            self.controller.clone(),
            self.events.clone(),
            self.core_generation,
        ));
        self.activity("info", "Mihomo 内核已启动");
        self.describe_rules();
        self.next_check =
            Utc::now() + Duration::seconds(self.state.config.policy.check_interval_secs as i64);
        if self.state.config.routing_mode != RoutingMode::Direct
            && self.state.config.policy.enabled
            && !explicit
            && !self.state.candidates.is_empty()
        {
            self.begin_probe(ProbeReason::Startup).await?;
        } else if self.state.config.routing_mode != RoutingMode::Direct {
            self.start_health_scan(self.health_nodes());
        }
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        sysproxy::restore(&self.store)?;
        self.cancel_probe();
        self.cancel_latency();
        self.cancel_entry_repair();
        self.entry_retry.clear();
        if let Some(task) = self.log_task.take() {
            task.abort();
        }
        self.core_generation += 1;
        if let Some(compatibility) = self.proxy_compatibility.take() {
            compatibility.stop().await;
        }
        if let Some(mut child) = self.child.take() {
            child.kill().await.ok();
            child.wait().await.ok();
        }
        self.state.running = false;
        self.state.active = None;
        self.state.log_connected = false;
        self.state.probing = false;
        self.state.manual_hold_until = None;
        self.state.cooldown_until = None;
        self.state.phase = "未启动".into();
        self.monitor.reset();
        self.state.failure_streak = 0;
        self.state.system_proxy = false;
        self.proxy_guard = None;
        self.activity("info", "内核已停止");
        Ok(())
    }

    fn automatic_allowed(&self) -> bool {
        let now = Utc::now();
        self.state.running
            && self.state.config.routing_mode != RoutingMode::Direct
            && !self.state.candidates.is_empty()
            && self.state.config.policy.enabled
            && !self.state.probing
            && !self.state.testing_nodes
            && self.entry_task.is_none()
            && self
                .state
                .manual_hold_until
                .is_none_or(|until| until <= now)
    }

    async fn check_system_proxy(&mut self) -> Result<()> {
        self.next_proxy_check = Utc::now() + Duration::seconds(60);
        if !self.state.running || !self.state.system_proxy || self.proxy_guard.is_none() {
            return Ok(());
        }
        let child = self.child.as_mut().context("系统代理内核未运行")?;
        anyhow::ensure!(child.try_wait()?.is_none(), "系统代理内核已退出");
        match sysproxy::check(&self.store, self.state.config.core.mixed_port)
            .context("系统代理自检失败")?
        {
            sysproxy::ProxyHealth::Healthy => {}
            sysproxy::ProxyHealth::Repaired => {
                self.activity("warn", "检测到 Windows 系统代理被关闭，已自动恢复");
                self.dirty = true;
            }
            sysproxy::ProxyHealth::TakenOver => {
                self.state.system_proxy = false;
                self.proxy_guard = None;
                if let Some(compatibility) = self.proxy_compatibility.take() {
                    compatibility.stop().await;
                }
                self.activity("info", "系统代理地址已被更改，已停止接管并保留当前设置");
                self.dirty = true;
            }
        }
        Ok(())
    }

    async fn begin_probe(&mut self, reason: ProbeReason) -> Result<()> {
        if self.state.probing || !self.state.running {
            return Ok(());
        }
        self.check_system_proxy().await?;
        if let Some(task) = self.health_task.take() {
            task.abort();
        }
        self.state.checking_nodes.clear();
        self.generation += 1;
        let generation = self.generation;
        let controller = self.controller.clone();
        let mut policy = self.state.config.policy.clone();
        if self
            .state
            .cooldown_until
            .is_some_and(|until| until > Utc::now())
        {
            policy.recovery_enabled = false;
        }
        let candidates = self.state.candidates.clone();
        let active = self.state.active.clone();
        let events = self.events.clone();
        self.state.probing = true;
        self.state.phase = "正在检测池内节点".into();
        self.probe_task = Some(tokio::spawn(async move {
            let progress = ProbeProgress {
                events: events.clone(),
                generation,
            };
            let report =
                run_probes(controller, policy, candidates, active, reason, &progress).await;
            events
                .send(Internal::ProbeFinished(generation, report))
                .ok();
        }));
        Ok(())
    }

    fn start_health_scan(&mut self, nodes: Vec<Candidate>) {
        if nodes.is_empty() || !self.state.running {
            return;
        }
        if let Some(task) = self.health_task.take() {
            task.abort();
        }
        let controller = self.controller.clone();
        let policy = self.state.config.policy.clone();
        let progress = ProbeProgress {
            events: self.events.clone(),
            generation: self.generation,
        };
        self.health_task = Some(tokio::spawn(async move {
            stream::iter(nodes)
                .for_each_concurrent(policy.probe_concurrency, |node| {
                    let controller = &controller;
                    let policy = &policy;
                    let progress = &progress;
                    async move {
                        progress.confirm(controller, policy, &node).await;
                    }
                })
                .await;
            progress
                .events
                .send(Internal::HealthFinished(progress.generation))
                .ok();
        }));
    }

    async fn event(&mut self, event: Internal) -> Result<()> {
        match event {
            Internal::EntryRepaired(generation, host, result)
                if generation == self.entry_generation && self.state.running =>
            {
                self.entry_task = None;
                self.entry_retry
                    .insert(host, Utc::now() + Duration::seconds(60));
                match result {
                    Ok(Some(repair)) => self.apply_entry_repair(repair).await?,
                    Ok(None) => self.activity("warn", "未找到可用入口，保留当前配置并稍后重试"),
                    Err(error) => self.activity("warn", format!("入口修复未完成：{error}")),
                }
            }
            Internal::ProbeProgress(generation, key, result)
                if generation == self.generation && self.state.running =>
            {
                if let Some(result) = result {
                    self.state.checking_nodes.remove(&key);
                    self.state.probes.insert(key, result);
                } else {
                    self.state.checking_nodes.insert(key);
                }
            }
            Internal::HealthFinished(generation) if generation == self.generation => {
                self.health_task = None;
                if self
                    .state
                    .active
                    .as_ref()
                    .and_then(|active| self.state.probes.get(&active.key))
                    .is_some_and(|probe| probe.delay_ms.is_some())
                {
                    self.monitor.reset();
                    self.state.failure_streak = 0;
                    self.next_failure_check = Utc::now() + Duration::seconds(3);
                }
            }
            Internal::LatencyFinished(generation, results)
                if generation == self.latency_generation && self.state.running =>
            {
                self.latency_task = None;
                self.state.testing_nodes = false;
                self.state.probes.extend(results);
            }
            Internal::Log(generation, log) if generation == self.core_generation => {
                let triggered = self.monitor.record(
                    policy::classify(&log),
                    Utc::now(),
                    &self.state.config.policy,
                );
                self.state.failure_streak = self.monitor.count();
                if triggered && Utc::now() < self.next_failure_check {
                    self.next_check = self.next_check.min(self.next_failure_check);
                }
                if triggered && self.automatic_allowed() && Utc::now() >= self.next_failure_check {
                    self.next_failure_check = Utc::now() + Duration::seconds(3);
                    self.activity(
                        "warn",
                        format!(
                            "连续 {} 次连接失败，开始检测池内节点",
                            self.state.failure_streak
                        ),
                    );
                    self.begin_probe(ProbeReason::Failures).await?;
                } else if triggered
                    && self.state.running
                    && self.state.config.routing_mode != RoutingMode::Direct
                    && !self.state.probing
                    && !self.state.testing_nodes
                    && self.health_task.is_none()
                    && self.entry_task.is_none()
                    && Utc::now() >= self.next_failure_check
                {
                    self.next_failure_check = Utc::now() + Duration::seconds(3);
                    self.start_health_scan(self.health_nodes());
                }
            }
            Internal::LogConnection(generation, connected)
                if generation == self.core_generation && self.state.log_connected != connected =>
            {
                self.state.log_connected = connected;
                if !connected {
                    self.activity("warn", "日志连接中断，正在重连；定时检测仍然有效");
                }
            }
            Internal::ProbeFinished(generation, report)
                if generation == self.generation && self.state.running =>
            {
                self.probe_task = None;
                self.state.probing = false;
                self.state.checking_nodes.clear();
                self.state.probes.extend(report.results);
                self.next_check = Utc::now()
                    + Duration::seconds(self.state.config.policy.check_interval_secs as i64);
                self.state.phase = "运行中".into();
                if let Some(choice) = report.choice {
                    self.retry_at = None;
                    let same = self
                        .state
                        .active
                        .as_ref()
                        .is_some_and(|active| active.key == choice.key);
                    if same {
                        self.recovery = None;
                        self.monitor.reset();
                        self.state.failure_streak = 0;
                        if report.reason == ProbeReason::Failures {
                            self.next_failure_check = Utc::now() + Duration::seconds(3);
                        }
                        if matches!(report.reason, ProbeReason::Failures | ProbeReason::Manual) {
                            self.activity("info", "当前节点通过主动检测，保持当前连接");
                        }
                    } else if report.reason == ProbeReason::Scheduled && report.active_healthy {
                        let count = match &self.recovery {
                            Some((key, count)) if key == &choice.key => count + 1,
                            _ => 1,
                        };
                        self.recovery = Some((choice.key.clone(), count));
                        if count >= self.state.config.policy.recovery_successes {
                            self.switch(choice, "优先节点恢复稳定").await?;
                        } else {
                            self.activity(
                                "info",
                                format!(
                                    "{} 恢复检测 {count}/{}",
                                    choice.name, self.state.config.policy.recovery_successes
                                ),
                            );
                        }
                    } else {
                        self.switch(choice, "连通性检测通过").await?;
                    }
                } else {
                    self.recovery = None;
                    self.retry_at = Some(
                        Utc::now()
                            + Duration::seconds(
                                self.state.config.policy.unavailable_retry_secs as i64,
                            ),
                    );
                    self.state.phase = "池内节点全部不可用".into();
                    self.activity(
                        "warn",
                        format!(
                            "所选节点均未通过检测，{} 秒后重试",
                            self.state.config.policy.unavailable_retry_secs
                        ),
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn switch(&mut self, node: Candidate, reason: &str) -> Result<()> {
        self.state.switching = true;
        self.publish();
        let result = self.apply_switch(node, reason).await;
        self.state.switching = false;
        self.publish();
        result
    }

    async fn apply_switch(&mut self, node: Candidate, reason: &str) -> Result<()> {
        let previous = self.state.active.clone();
        self.controller.select(&node.key).await?;
        anyhow::ensure!(
            self.controller.selected().await? == node.key,
            "内核没有确认节点切换"
        );
        let mut config = self.state.config.clone();
        config.last_active = Some(node.key.clone());
        if let Err(error) = self.store.save(&config) {
            if let Some(previous) = &previous
                && let Err(rollback) = self.controller.select(&previous.key).await
            {
                self.state.active = Some(node);
                anyhow::bail!("切换已生效但保存失败：{error}；恢复原节点也失败：{rollback}");
            }
            return Err(error);
        }
        let changed = previous
            .as_ref()
            .is_none_or(|previous| previous.key != node.key);
        self.state.config = config;
        self.state.active = Some(node.clone());
        self.next_failure_check = Utc::now();
        if changed {
            self.state.switch_count += 1;
            self.state.last_switch = Some(SwitchTransition {
                from: previous,
                to: node.clone(),
                at: Utc::now(),
                reason: reason.into(),
            });
            self.state.cooldown_until =
                Some(Utc::now() + Duration::seconds(self.state.config.policy.cooldown_secs as i64));
            self.activity(
                "info",
                format!(
                    "已切换到 {} · {}（{reason}）",
                    node.subscription_name, node.name
                ),
            );
        }
        self.monitor.reset();
        self.state.failure_streak = 0;
        self.recovery = None;
        self.retry_at = None;
        Ok(())
    }

    async fn tick(&mut self) -> Result<()> {
        // File capture includes startup and controller-disconnection diagnostics.
        // WebSocket events remain exclusively responsible for failure monitoring.
        for mut entry in self.log_tail.poll()? {
            entry.message = self.redact(&entry.message);
            self.state.core_logs.insert(0, entry);
            self.dirty = true;
        }
        self.state.core_logs.truncate(2000);
        if let Some(child) = &mut self.child
            && let Some(status) = child.try_wait()?
        {
            self.stop().await?;
            anyhow::bail!("Mihomo 内核意外退出：{status}");
        }
        let now = Utc::now();
        if now >= self.next_proxy_check {
            self.check_system_proxy().await?;
        }
        let mut changed = false;
        if self
            .state
            .manual_hold_until
            .is_some_and(|until| until <= now)
        {
            self.state.manual_hold_until = None;
            changed = true;
        }
        if self.state.cooldown_until.is_some_and(|until| until <= now) {
            self.state.cooldown_until = None;
            changed = true;
        }
        if self.automatic_allowed() {
            if self.retry_at.is_some_and(|at| at <= now) {
                self.retry_at = None;
                self.begin_probe(ProbeReason::Retry).await?;
                changed = true;
            } else if self.retry_at.is_none() && now >= self.next_check {
                self.begin_probe(ProbeReason::Scheduled).await?;
                changed = true;
            }
        }
        if !self.automatic_allowed()
            && self.state.running
            && self.state.config.routing_mode != RoutingMode::Direct
            && !self.state.probing
            && !self.state.testing_nodes
            && self.entry_task.is_none()
            && self.health_task.is_none()
            && now >= self.next_check
        {
            self.start_health_scan(self.health_nodes());
            self.next_check =
                now + Duration::seconds(self.state.config.policy.check_interval_secs as i64);
        }
        if !self.state.probing
            && self.entry_task.is_none()
            && !self.state.testing_nodes
            && let Some(id) = self
                .state
                .config
                .subscriptions
                .iter()
                .find(|sub| {
                    sub.url.is_some()
                        && (now - sub.updated_at).num_seconds() >= 86400
                        && self
                            .subscription_retry
                            .get(&sub.id)
                            .is_none_or(|retry| *retry <= now)
                })
                .map(|sub| sub.id.clone())
        {
            self.subscription_retry
                .insert(id.clone(), now + Duration::seconds(3600));
            self.state.busy = true;
            self.publish();
            let result = self.command(Command::UpdateSubscription(id)).await;
            self.state.busy = false;
            if result.is_err() {
                self.state.phase = if self.state.running {
                    "运行中"
                } else {
                    "未启动"
                }
                .into();
            }
            self.publish();
            result?;
        }
        if changed || self.dirty {
            self.publish();
        }
        self.begin_entry_repair()?;
        Ok(())
    }

    fn describe_rules(&mut self) {
        if self.state.config.routing_mode != RoutingMode::Rule {
            return;
        }
        if let Some(id) = &self.state.effective_rule_subscription {
            if let Some(sub) = self
                .state
                .config
                .subscriptions
                .iter()
                .find(|sub| &sub.id == id)
            {
                self.activity("info", format!("规则模式：使用 {} 的分流规则", sub.name));
            }
        } else {
            self.activity(
                "warn",
                "订阅未提供分流规则：本地网络直连，其余流量走当前代理节点",
            );
        }
    }
}

fn spawn_log_reader(
    controller: Controller,
    events: mpsc::UnboundedSender<Internal>,
    generation: u64,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = 1;
        loop {
            if let Ok(mut socket) = controller.logs().await {
                backoff = 1;
                events.send(Internal::LogConnection(generation, true)).ok();
                while let Some(message) = socket.next().await {
                    match message {
                        Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                            if let Ok(log) = serde_json::from_str::<CoreLog>(&text)
                                && events.send(Internal::Log(generation, log)).is_err()
                            {
                                return;
                            }
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Ping(payload)) => {
                            socket
                                .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                                .await
                                .ok();
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Close(_)) | Err(_) => break,
                        _ => {}
                    }
                }
            }
            if events
                .send(Internal::LogConnection(generation, false))
                .is_err()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(15);
        }
    })
}

async fn run_probes(
    controller: Controller,
    policy: AutoPolicy,
    candidates: Vec<Candidate>,
    active: Option<Candidate>,
    reason: ProbeReason,
    progress: &ProbeProgress,
) -> ProbeReport {
    let mut report = ProbeReport {
        reason,
        choice: None,
        active_healthy: false,
        results: BTreeMap::new(),
    };
    let mut order = policy::probe_order(&candidates, active.as_ref(), reason);
    if reason == ProbeReason::Failures
        && let Some(active) = &active
    {
        order.retain(|node| node.key != active.key);
        let mut current = Box::pin(progress.confirm(&controller, &policy, active));
        let mut fallback = Box::pin(probe_candidates(
            &controller,
            &policy,
            &order,
            None,
            progress,
            true,
        ));
        let (result, backup) = tokio::select! {
            result = &mut current => (result, None),
            backup = &mut fallback => (current.await, Some(backup)),
        };
        report.active_healthy = result.delay_ms.is_some();
        report.results.insert(active.key.clone(), result);
        if report.active_healthy {
            report.choice = Some(active.clone());
        } else {
            let (choice, results) = match backup {
                Some(backup) => backup,
                None => fallback.await,
            };
            report.choice = choice;
            report.results.extend(results);
        }
        return report;
    }
    if reason != ProbeReason::Manual
        && let Some(active) = &active
    {
        let result = progress.confirm(&controller, &policy, active).await;
        report.active_healthy = result.delay_ms.is_some();
        report.results.insert(active.key.clone(), result);
        if report.active_healthy {
            if reason != ProbeReason::Scheduled || !policy.recovery_enabled {
                report.choice = Some(active.clone());
                return report;
            }
            order = candidates
                .iter()
                .take_while(|node| node.key != active.key)
                .cloned()
                .collect();
        } else {
            order = policy::probe_order(&candidates, Some(active), ProbeReason::Failures);
        }
        order.retain(|node| node.key != active.key);
    }
    let (choice, results) = probe_candidates(
        &controller,
        &policy,
        &order,
        active.as_ref(),
        progress,
        !report.active_healthy && reason != ProbeReason::Manual,
    )
    .await;
    report.choice = choice.or_else(|| report.active_healthy.then_some(active).flatten());
    report.results.extend(results);
    report
}

async fn probe_candidates(
    controller: &Controller,
    policy: &AutoPolicy,
    order: &[Candidate],
    active: Option<&Candidate>,
    progress: &ProbeProgress,
    first_available: bool,
) -> (Option<Candidate>, BTreeMap<String, ProbeResult>) {
    let mut checked = BTreeMap::new();
    let mut offset = 0;
    while offset < order.len() {
        let subscription_id = order[offset].subscription_id.clone();
        let end = order[offset..]
            .iter()
            .take_while(|node| node.subscription_id == subscription_id)
            .count()
            + offset;
        let nodes = &order[offset..end];
        let mut results = stream::iter(nodes.iter().cloned())
            .map(|node| {
                let controller = controller.clone();
                let policy = policy.clone();
                let is_active = active.as_ref().is_some_and(|active| active.key == node.key);
                async move {
                    let result = if is_active {
                        progress.confirm(&controller, &policy, &node).await
                    } else {
                        progress.probe(&controller, &policy, &node).await
                    };
                    (node, result)
                }
            })
            .buffer_unordered(policy.probe_concurrency);
        while let Some((node, result)) = results.next().await {
            let healthy = result.delay_ms.is_some();
            checked.insert(node.key.clone(), result);
            if first_available && healthy {
                return (Some(node), checked);
            }
        }
        if let Some(node) = nodes.iter().find(|node| {
            checked
                .get(&node.key)
                .is_some_and(|result| result.delay_ms.is_some())
        }) {
            return (Some(node.clone()), checked);
        }
        offset = end;
    }
    (None, checked)
}

struct ProbeProgress {
    events: mpsc::UnboundedSender<Internal>,
    generation: u64,
}

impl ProbeProgress {
    async fn confirm(
        &self,
        controller: &Controller,
        policy: &AutoPolicy,
        node: &Candidate,
    ) -> ProbeResult {
        self.check(controller, policy, node, true).await
    }

    async fn probe(
        &self,
        controller: &Controller,
        policy: &AutoPolicy,
        node: &Candidate,
    ) -> ProbeResult {
        self.check(controller, policy, node, false).await
    }

    async fn check(
        &self,
        controller: &Controller,
        policy: &AutoPolicy,
        node: &Candidate,
        confirm: bool,
    ) -> ProbeResult {
        self.events
            .send(Internal::ProbeProgress(
                self.generation,
                node.key.clone(),
                None,
            ))
            .ok();
        let result = if confirm {
            confirm_one(controller, policy, node).await
        } else {
            probe_one(controller, policy, node).await
        };
        self.events
            .send(Internal::ProbeProgress(
                self.generation,
                node.key.clone(),
                Some(result.clone()),
            ))
            .ok();
        result
    }
}

async fn confirm_one(
    controller: &Controller,
    policy: &AutoPolicy,
    node: &Candidate,
) -> ProbeResult {
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(policy.probe_timeout_ms);
    let result = probe_one(controller, policy, node).await;
    let pause = std::time::Duration::from_millis(200);
    if result.delay_ms.is_some()
        || deadline.saturating_duration_since(tokio::time::Instant::now()) <= pause
    {
        return result;
    }
    tokio::time::sleep(pause).await;
    let mut retry = policy.clone();
    retry.probe_timeout_ms = deadline
        .saturating_duration_since(tokio::time::Instant::now())
        .as_millis() as u64;
    probe_one(controller, &retry, node).await
}

async fn probe_one(controller: &Controller, policy: &AutoPolicy, node: &Candidate) -> ProbeResult {
    match controller.probe(&node.key, policy).await {
        Ok(delay) => ProbeResult {
            delay_ms: Some(delay),
            checked_at: Some(Utc::now()),
            error: None,
        },
        Err(error) => ProbeResult {
            delay_ms: None,
            checked_at: Some(Utc::now()),
            error: Some(error.to_string()),
        },
    }
}
