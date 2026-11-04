use crate::{
    child::OwnedChild,
    geodata,
    model::{AUTO_GROUP, AutoPolicy, CoreSettings},
    store::Store,
};
use anyhow::{Context, Result};
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde_json::json;
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::client::IntoClientRequest};

#[derive(Clone)]
pub struct Controller {
    settings: CoreSettings,
    client: reqwest::Client,
}

impl Controller {
    pub fn new(settings: &CoreSettings) -> Result<Self> {
        Ok(Self {
            settings: settings.clone(),
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .build()?,
        })
    }

    fn url(&self, segments: &[&str]) -> reqwest::Url {
        let mut url = reqwest::Url::parse(&format!(
            "http://127.0.0.1:{}/",
            self.settings.controller_port
        ))
        .expect("loopback URL");
        url.path_segments_mut()
            .expect("hierarchical URL")
            .extend(segments);
        url
    }

    fn auth(&self, method: reqwest::Method, segments: &[&str]) -> reqwest::RequestBuilder {
        self.client
            .request(method, self.url(segments))
            .bearer_auth(&self.settings.secret)
    }

    pub async fn ready(&self) -> Result<()> {
        self.auth(reqwest::Method::GET, &["version"])
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn select(&self, key: &str) -> Result<()> {
        self.auth(reqwest::Method::PUT, &["proxies", AUTO_GROUP])
            .json(&json!({"name": key}))
            .send()
            .await
            .context("节点切换请求失败")?
            .error_for_status()
            .context("内核拒绝节点切换")?;
        self.auth(reqwest::Method::PUT, &["proxies", "GLOBAL"])
            .json(&json!({"name": AUTO_GROUP}))
            .send()
            .await?
            .error_for_status()
            .context("无法同步全局模式的代理节点")?;
        Ok(())
    }

    pub async fn selected(&self) -> Result<String> {
        let value: serde_json::Value = self
            .auth(reqwest::Method::GET, &["proxies", AUTO_GROUP])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        value
            .get("now")
            .and_then(|value| value.as_str())
            .map(String::from)
            .context("策略组没有当前节点")
    }

    pub async fn reload(&self, path: &Path) -> Result<()> {
        self.auth(reqwest::Method::PUT, &["configs"])
            .query(&[("force", "true")])
            .json(&json!({"path": path.to_string_lossy()}))
            .timeout(Duration::from_secs(45))
            .send()
            .await
            .context("配置重载请求失败")?
            .error_for_status()
            .context("内核拒绝重载配置")?;
        Ok(())
    }

    pub async fn probe(&self, key: &str, policy: &AutoPolicy) -> Result<u64> {
        let budget = Duration::from_millis(policy.probe_timeout_ms);
        let deadline = tokio::time::Instant::now() + budget;
        let fallback_delay = budget / policy.probe_urls.len().max(1) as u32;
        let attempt = async {
            let mut urls = policy.probe_urls.iter();
            let mut pending = FuturesUnordered::new();
            if let Some(url) = urls.next() {
                pending.push(self.probe_url(key, url, deadline));
            }
            let mut next_fallback = tokio::time::Instant::now() + fallback_delay;
            while !pending.is_empty() {
                tokio::select! {
                    Some(result) = pending.next() => {
                        if let Ok(delay) = result { return Ok(delay); }
                        if let Some(url) = urls.next() {
                            pending.push(self.probe_url(key, url, deadline));
                            next_fallback = tokio::time::Instant::now() + fallback_delay;
                        }
                    }
                    _ = tokio::time::sleep_until(next_fallback), if urls.len() > 0 => {
                        if let Some(url) = urls.next() {
                            pending.push(self.probe_url(key, url, deadline));
                            next_fallback = tokio::time::Instant::now() + fallback_delay;
                        }
                    }
                }
            }
            anyhow::bail!("所有检测地址均未通过 HTTP 204 检查")
        };
        tokio::time::timeout_at(deadline, attempt)
            .await
            .context("节点连通性检测超时")?
    }

    async fn probe_url(&self, key: &str, url: &str, deadline: tokio::time::Instant) -> Result<u64> {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let value: serde_json::Value = self
            .auth(reqwest::Method::GET, &["proxies", key, "delay"])
            .query(&[
                ("url", url.to_owned()),
                ("timeout", remaining.as_millis().to_string()),
                ("expected", "204".into()),
            ])
            .timeout(remaining)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        value
            .get("delay")
            .and_then(|value| value.as_u64())
            .context("检测未返回延迟")
    }

    pub async fn logs(&self) -> Result<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>> {
        let mut request = format!(
            "ws://127.0.0.1:{}/logs?level=info",
            self.settings.controller_port
        )
        .into_client_request()?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", self.settings.secret).parse()?,
        );
        let (stream, _) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::connect_async(request),
        )
        .await??;
        Ok(stream)
    }
}

pub fn find_binary(settings: &CoreSettings) -> Result<PathBuf> {
    if !settings.binary_path.trim().is_empty() {
        let path = PathBuf::from(&settings.binary_path);
        anyhow::ensure!(path.is_file(), "设置的 Mihomo 内核文件不存在");
        return Ok(path);
    }
    let name = if cfg!(windows) {
        "mihomo.exe"
    } else {
        "mihomo"
    };
    let mut paths = Vec::new();
    if let Ok(path) = std::env::var("MIHOMO_PATH") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        paths.push(parent.join(name));
        paths.push(parent.join("bin").join(name));
    }
    paths.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../bin")
            .join(name),
    );
    paths
        .into_iter()
        .find(|path| path.is_file())
        .context("找不到 mihomo.exe；请将它与 ClashFx.exe 放在同一文件夹后重试")
}

fn command(binary: &Path, store: &Store, runtime: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .arg("-d")
        .arg(&store.root)
        .arg("-f")
        .arg(runtime)
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

pub async fn validate(binary: &Path, store: &Store, runtime: &Path) -> Result<()> {
    geodata::prepare(store)?;
    let mut validation = command(binary, store, runtime);
    validation
        .arg("-t")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = OwnedChild::spawn(&mut validation)?;
    let output = tokio::time::timeout(Duration::from_secs(45), child.output())
        .await
        .context("内核配置校验超时")??;
    if !output.status.success() {
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if let Ok(mut log) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(store.root.join("core.log"))
        {
            writeln!(log, "{output}").ok();
        }
        if output.contains("MMDB") || output.contains("GeoIP") || output.contains("GeoSite") {
            anyhow::bail!(
                "规则数据库加载失败，请检查完整安装包中的 geodata 文件夹；详细原因已写入日志"
            );
        }
        let detail = output
            .lines()
            .rev()
            .find(|line| line.contains("level=fatal") || line.contains("level=error"))
            .or_else(|| output.lines().last())
            .unwrap_or("未知配置错误");
        let detail = regex::Regex::new(r#"https?://[^\s\"\\]+"#)
            .expect("URL pattern")
            .replace_all(detail, "[下载地址]");
        anyhow::bail!(
            "内核配置校验失败：{}；详情见日志",
            detail.chars().take(220).collect::<String>()
        );
    }
    Ok(())
}

pub async fn start(settings: &CoreSettings, store: &Store) -> Result<OwnedChild> {
    let binary = find_binary(settings)?;
    validate(&binary, store, &store.runtime_path()).await?;
    for port in [settings.controller_port, settings.mixed_port] {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port))
            .with_context(|| format!("本地端口 {port} 已被其他程序占用"))?;
        drop(listener);
    }
    let log_path = store.root.join("core.log");
    if std::fs::metadata(&log_path).is_ok_and(|meta| meta.len() > 2 * 1024 * 1024) {
        std::fs::rename(&log_path, store.root.join("core.previous.log"))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    let mut launch = command(&binary, store, &store.runtime_path());
    launch.stdout(file.try_clone()?).stderr(file);
    let mut child = OwnedChild::spawn(&mut launch).context("无法启动 Mihomo 内核")?;
    let controller = Controller::new(settings)?;
    for _ in 0..100 {
        if controller.ready().await.is_ok() {
            return Ok(child);
        }
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("内核启动后退出（{status}），详情见配置目录 core.log");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    child.kill().await.ok();
    anyhow::bail!("内核启动超时，详情见配置目录 core.log")
}
