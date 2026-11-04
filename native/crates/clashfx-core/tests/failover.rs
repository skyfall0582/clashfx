use anyhow::{Context, Result};
use clashfx_core::{
    engine::{Backend, Command},
    model::{AutoPolicy, RoutingMode, Snapshot},
    store::Store,
    subscription,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

struct FakeProxy {
    port: u16,
    healthy: Arc<AtomicBool>,
    stall: Arc<AtomicBool>,
    reject_next: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn proxy() -> Result<FakeProxy> {
    proxy_at("127.0.0.1:0").await
}

async fn proxy_at(address: &str) -> Result<FakeProxy> {
    let listener = TcpListener::bind(address).await?;
    let port = listener.local_addr()?.port();
    let healthy = Arc::new(AtomicBool::new(true));
    let state = healthy.clone();
    let stall = Arc::new(AtomicBool::new(false));
    let stalled = stall.clone();
    let reject_next = Arc::new(AtomicBool::new(false));
    let reject = reject_next.clone();
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let healthy = state.clone();
            let stalled = stalled.clone();
            count.fetch_add(1, Ordering::SeqCst);
            if reject.swap(false, Ordering::SeqCst) {
                continue;
            }
            tokio::spawn(async move {
                if stalled.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    return;
                }
                if !healthy.load(Ordering::SeqCst) {
                    return;
                }
                let mut bytes = Vec::new();
                if header(&mut socket, &mut bytes).await.is_err() {
                    return;
                }
                if bytes.starts_with(b"CONNECT ") {
                    if socket
                        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                        .await
                        .is_err()
                    {
                        return;
                    }
                    bytes.clear();
                    if header(&mut socket, &mut bytes).await.is_err() {
                        return;
                    }
                }
                if healthy.load(Ordering::SeqCst) {
                    socket.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.ok();
                }
            });
        }
    });
    Ok(FakeProxy {
        port,
        healthy,
        stall,
        reject_next,
        requests,
        task,
    })
}

async fn header(socket: &mut TcpStream, bytes: &mut Vec<u8>) -> Result<()> {
    loop {
        let mut buffer = [0u8; 1024];
        let size = tokio::time::timeout(Duration::from_secs(3), socket.read(&mut buffer)).await??;
        anyhow::ensure!(size > 0 && bytes.len() < 32768, "incomplete HTTP header");
        bytes.extend_from_slice(&buffer[..size]);
        if bytes.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            return Ok(());
        }
    }
}

async fn wait(
    backend: &Backend,
    label: &str,
    seconds: u64,
    predicate: impl Fn(&Snapshot) -> bool,
) -> Result<Snapshot> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let snapshot = backend.snapshot();
        if predicate(&snapshot) {
            println!("PASS: {label}");
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "{label}: phase={}, active={:?}, error={:?}, logs={:?}",
                snapshot.phase,
                snapshot.active.as_ref().map(|node| &node.name),
                snapshot.last_error,
                snapshot
                    .core_logs
                    .iter()
                    .take(8)
                    .map(|line| &line.message)
                    .collect::<Vec<_>>()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn fail_requests(port: u16) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::http(format!("http://127.0.0.1:{port}"))?)
        .timeout(Duration::from_secs(3))
        .build()?;
    for _ in 0..3 {
        client.get("http://clashfx.test/request").send().await.ok();
    }
    Ok(())
}

fn unused_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Measures real Mihomo failover with a stalled active node and a stalled spare."]
async fn three_second_budget_does_not_stack_urls_retries_or_slow_spares() -> Result<()> {
    let active = proxy().await?;
    let slow = proxy().await?;
    let working = proxy().await?;
    slow.stall.store(true, Ordering::SeqCst);
    let store = Store::open(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../work/fast-failover-{}", uuid::Uuid::new_v4())),
    ))?;
    let body = format!(
        "proxies:\n - {{name: Current, type: http, server: 127.0.0.1, port: {}}}\n - {{name: Slow, type: http, server: 127.0.0.1, port: {}}}\n - {{name: Working, type: http, server: 127.0.0.1, port: {}}}\n",
        active.port, slow.port, working.port
    );
    let mut sub = subscription::new_subscription(
        "Fixture".into(),
        None,
        &subscription::parse_profile(&body)?,
    );
    sub.pool_nodes = vec!["Current".into(), "Slow".into(), "Working".into()];
    store.write_profile(&sub.id, &body)?;
    let mut config = store.load()?;
    config.core.controller_port = unused_port()?;
    config.core.mixed_port = unused_port()?;
    config.policy.check_interval_secs = 10;
    config.policy.probe_urls = vec![
        "http://clashfx.test/first".into(),
        "http://clashfx.test/second".into(),
    ];
    config.subscriptions.push(sub);
    store.save(&config)?;
    let backend = Backend::launch(store.clone())?;
    backend
        .request(Command::Start)
        .await
        .map_err(anyhow::Error::msg)?;
    wait(
        &backend,
        "healthy startup does not probe spare nodes",
        5,
        |s| !s.probing && !s.probes.is_empty(),
    )
    .await?;
    assert_eq!(slow.requests.load(Ordering::SeqCst), 0);
    assert_eq!(working.requests.load(Ordering::SeqCst), 0);
    active.stall.store(true, Ordering::SeqCst);
    wait(
        &backend,
        "scheduled check starts on the stalled node",
        12,
        |s| s.probing,
    )
    .await?;
    let started = Instant::now();
    let switched = wait(
        &backend,
        "working spare is selected without waiting for the slow spare",
        5,
        |s| !s.probing && s.active.as_ref().is_some_and(|node| node.name == "Working"),
    )
    .await?;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(3800),
        "failover took {elapsed:?}"
    );
    assert!(switched.checking_nodes.is_empty());
    assert!(slow.requests.load(Ordering::SeqCst) > 0);
    println!(
        "Measured failover: {} ms; evidence: {}",
        elapsed.as_millis(),
        store.root.display()
    );
    backend.shutdown();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Runs Mihomo to verify stable connections avoid background repairs and reloads."]
async fn stable_connection_only_checks_active_and_confirms_failures() -> Result<()> {
    let preferred = proxy().await?;
    let active = proxy().await?;
    let standby = proxy().await?;
    standby.healthy.store(false, Ordering::SeqCst);
    let store = Store::open(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../work/stable-{}", uuid::Uuid::new_v4())),
    ))?;
    let body = format!(
        "proxies:\n - {{name: Preferred, type: http, server: entry.test, port: {}}}\n - {{name: Current, type: http, server: entry.test, port: {}}}\n - {{name: Standby, type: http, server: entry.test, port: {}}}\nhosts:\n entry.test: 127.0.0.1\nproxy-groups:\n - {{name: Unused health checks, type: url-test, proxies: [Preferred, Standby], url: http://clashfx.test/generate_204, interval: 1, lazy: false}}\nrules:\n - MATCH,Proxy\n",
        preferred.port, active.port, standby.port
    );
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!(
        "http://127.0.0.1:{}/subscription",
        listener.local_addr()?.port()
    );
    let updates = Arc::new(AtomicUsize::new(0));
    let update_count = updates.clone();
    let downloaded = body.clone();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = Vec::new();
            if header(&mut socket, &mut request).await.is_ok() {
                update_count.fetch_add(1, Ordering::SeqCst);
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{downloaded}", downloaded.len()).as_bytes()).await.ok();
            }
        }
    });
    let mut sub = subscription::new_subscription(
        "Fixture".into(),
        Some(url),
        &subscription::parse_profile(&body)?,
    );
    sub.pool_nodes = vec!["Preferred".into(), "Current".into(), "Standby".into()];
    sub.updated_at = chrono::Utc::now() - chrono::Duration::hours(2);
    let id = sub.id.clone();
    store.write_profile(&id, &body)?;
    let mut config = store.load()?;
    config.core.controller_port = unused_port()?;
    config.core.mixed_port = unused_port()?;
    config.policy.check_interval_secs = 10;
    config.policy.probe_timeout_ms = 500;
    config.policy.probe_urls = vec!["http://clashfx.test/generate_204".into()];
    config.last_active = Some(subscription::wire_name(&id, "Current"));
    config.pool_order.push(id.clone());
    config.subscriptions.push(sub);
    store.save(&config)?;
    let backend = Backend::launch(store.clone())?;
    backend
        .request(Command::Start)
        .await
        .map_err(anyhow::Error::msg)?;
    wait(
        &backend,
        "startup retains the healthy remembered node",
        8,
        |s| {
            !s.probing
                && s.probes
                    .get(config.last_active.as_ref().unwrap())
                    .is_some_and(|p| p.delay_ms.is_some())
        },
    )
    .await?;
    assert_eq!(backend.snapshot().active.as_ref().unwrap().name, "Current");
    assert_eq!(preferred.requests.load(Ordering::SeqCst), 0);
    assert_eq!(standby.requests.load(Ordering::SeqCst), 0);
    let runtime_modified = std::fs::metadata(store.runtime_path())?.modified()?;
    backend
        .request(Command::TestNodes(None))
        .await
        .map_err(anyhow::Error::msg)?;
    wait(
        &backend,
        "manual latency results include the failed standby",
        8,
        |s| !s.testing_nodes,
    )
    .await?;
    let preferred_checks = preferred.requests.load(Ordering::SeqCst);
    let standby_checks = standby.requests.load(Ordering::SeqCst);
    let before = backend.snapshot().probes[config.last_active.as_ref().unwrap()].checked_at;
    active.reject_next.store(true, Ordering::SeqCst);
    wait(
        &backend,
        "a transient active probe failure is rechecked without switching",
        18,
        |s| {
            !s.probing
                && s.probes
                    .get(config.last_active.as_ref().unwrap())
                    .is_some_and(|p| p.checked_at > before && p.delay_ms.is_some())
        },
    )
    .await?;
    assert_eq!(backend.snapshot().switch_count, 0);
    assert_eq!(preferred.requests.load(Ordering::SeqCst), preferred_checks);
    assert_eq!(standby.requests.load(Ordering::SeqCst), standby_checks);
    assert!(
        !store.root.join("entry-check").exists(),
        "failed standby triggered DNS repair"
    );
    assert_eq!(
        updates.load(Ordering::SeqCst),
        0,
        "a two-hour-old subscription was refreshed"
    );
    backend
        .request(Command::UpdateSubscription(id))
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(updates.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::metadata(store.runtime_path())?.modified()?,
        runtime_modified,
        "unchanged subscription reloaded the core"
    );
    let mut policy = config.policy.clone();
    policy.check_interval_secs = 300;
    backend
        .request(Command::SavePolicy(policy))
        .await
        .map_err(anyhow::Error::msg)?;
    active.healthy.store(false, Ordering::SeqCst);
    let port = config.core.mixed_port;
    let failures = tokio::spawn(async move { fail_requests(port).await });
    wait(&backend, "failure logs start confirmation", 5, |s| {
        s.probing
    })
    .await?;
    active.healthy.store(true, Ordering::SeqCst);
    failures.await??;
    wait(
        &backend,
        "connection recovers before failure-driven confirmation",
        8,
        |s| {
            !s.probing
                && s.activity
                    .iter()
                    .any(|entry| entry.message == "当前节点通过主动检测，保持当前连接")
        },
    )
    .await?;
    active.healthy.store(false, Ordering::SeqCst);
    fail_requests(config.core.mixed_port).await?;
    wait(
        &backend,
        "debounced failures still select a fallback before the five-minute check",
        22,
        |s| s.active.as_ref().is_some_and(|n| n.name == "Preferred"),
    )
    .await?;
    active.healthy.store(true, Ordering::SeqCst);
    preferred.healthy.store(false, Ordering::SeqCst);
    fail_requests(config.core.mixed_port).await?;
    wait(
        &backend,
        "recovery cooldown does not block emergency failover",
        18,
        |s| s.active.as_ref().is_some_and(|n| n.name == "Current"),
    )
    .await?;
    backend.shutdown();
    server.abort();
    println!("Evidence: {}", store.root.display());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Runs Mihomo with local DNS and rotating proxy entry fixtures."]
async fn entry_repair_recovers_rotating_dns_without_changing_subscription_or_manual_node()
-> Result<()> {
    use std::sync::RwLock;
    let first = proxy_at("127.0.0.2:0").await?;
    let port = first.port;
    let bad = proxy_at(&format!("127.0.0.1:{port}")).await?;
    bad.healthy.store(false, Ordering::SeqCst);
    let second = proxy_at(&format!("127.0.0.3:{port}")).await?;
    let address = Arc::new(RwLock::new("127.0.0.2".to_string()));
    let seen_bad = Arc::new(AtomicBool::new(false));
    let seen_good = Arc::new(AtomicBool::new(false));
    let delay_dns = Arc::new(AtomicBool::new(false));
    let dns = TcpListener::bind("127.0.0.1:0").await?;
    let dns_port = dns.local_addr()?.port();
    let current = address.clone();
    let bad_queries = seen_bad.clone();
    let good_queries = seen_good.clone();
    let delayed = delay_dns.clone();
    let dns_task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = dns.accept().await {
            let current = current.clone();
            let bad_queries = bad_queries.clone();
            let good_queries = good_queries.clone();
            let delayed = delayed.clone();
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                if header(&mut socket, &mut bytes).await.is_err() {
                    return;
                }
                let request = String::from_utf8_lossy(&bytes);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let Ok(url) = reqwest::Url::parse(&format!("http://localhost{path}")) else {
                    return;
                };
                let query: std::collections::HashMap<_, _> =
                    url.query_pairs().into_owned().collect();
                let Some(host) = query.get("name") else {
                    return;
                };
                if delayed.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                }
                let ip = if url.path() == "/bad" {
                    bad_queries.store(true, Ordering::SeqCst);
                    "127.0.0.1".to_string()
                } else {
                    good_queries.store(true, Ordering::SeqCst);
                    current.read().unwrap().clone()
                };
                let answers = if query.get("type").map(String::as_str) == Some("A") {
                    serde_json::json!([
                        {"name": host, "type": 5, "data": "entry-pool.test."},
                        {"name": "entry-pool.test.", "type": 1, "data": ip},
                        {"name": "unrelated.test.", "type": 1, "data": "127.0.0.9"}
                    ])
                } else {
                    serde_json::json!([])
                };
                let body = serde_json::json!({"Status": 0, "Answer": answers}).to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.ok();
            });
        }
    });
    let store = Store::open(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../work/entry-repair-{}", uuid::Uuid::new_v4())),
    ))?;
    let mut config = store.load()?;
    config.core.controller_port = unused_port()?;
    config.core.mixed_port = unused_port()?;
    config.policy.enabled = false;
    config.policy.check_interval_secs = 10;
    config.policy.probe_timeout_ms = 500;
    config.policy.probe_urls = vec!["http://entry-probe.test/check".into()];
    config.entry_repair.resolvers = vec![
        format!("http://127.0.0.1:{}/unavailable", unused_port()?),
        format!("http://127.0.0.1:{dns_port}/bad"),
        format!("http://127.0.0.1:{dns_port}/good"),
    ];
    let body = |host: &str| {
        format!(
            "proxies:\n - {{name: Entry, type: http, server: {host}, port: {port}}}\nhosts:\n  '{host}': 127.0.0.1\n  unrelated.test: 192.0.2.10\nrules:\n - MATCH,Proxy\n"
        )
    };
    let original = body("first.entry.test");
    let sub = subscription::new_subscription(
        "Rotating entry".into(),
        None,
        &subscription::parse_profile(&original)?,
    );
    let id = sub.id.clone();
    store.write_profile(&id, &original)?;
    config.subscriptions.push(sub);
    store.save(&config)?;
    let backend = Backend::launch(store.clone())?;
    backend
        .request(Command::Start)
        .await
        .map_err(anyhow::Error::msg)?;
    let key = backend.snapshot().active.unwrap().key;
    let repaired = |snapshot: &Snapshot, count| {
        snapshot
            .activity
            .iter()
            .filter(|entry| entry.message.contains("已应用通过检测的入口地址"))
            .count()
            >= count
    };
    wait(
        &backend,
        "repair works without a switching pool and rejects a TCP-only endpoint",
        40,
        |s| repaired(s, 1),
    )
    .await?;
    assert!(seen_bad.load(Ordering::SeqCst) && seen_good.load(Ordering::SeqCst));
    assert_eq!(std::fs::read_to_string(store.profile_path(&id)?)?, original);
    let runtime = || -> Result<serde_yaml_ng::Value> {
        Ok(serde_yaml_ng::from_str(&std::fs::read_to_string(
            store.runtime_path(),
        )?)?)
    };
    assert_eq!(
        runtime()?["hosts"]["first.entry.test"].as_str(),
        Some("127.0.0.2")
    );
    assert_eq!(
        runtime()?["hosts"]["unrelated.test"].as_str(),
        Some("192.0.2.10")
    );
    assert_eq!(
        runtime()?["proxies"][0]["server"].as_str(),
        Some("first.entry.test")
    );
    *address.write().unwrap() = "127.0.0.3".into();
    first.healthy.store(false, Ordering::SeqCst);
    wait(
        &backend,
        "expired entry is rediscovered automatically",
        85,
        |s| repaired(s, 2),
    )
    .await?;
    assert_eq!(
        runtime()?["hosts"]["first.entry.test"].as_str(),
        Some("127.0.0.3")
    );
    assert_eq!(backend.snapshot().active.unwrap().key, key);
    assert_eq!(backend.snapshot().switch_count, 0);
    let replacement = store.root.join("replacement.yaml");
    std::fs::write(&replacement, body("new-prefix.entry.test"))?;
    backend
        .request(Command::ReplaceSubscriptionFile {
            id: id.clone(),
            path: replacement,
        })
        .await
        .map_err(anyhow::Error::msg)?;
    wait(
        &backend,
        "changed subscription hostname is repaired without a wildcard",
        40,
        |s| repaired(s, 3),
    )
    .await?;
    assert!(runtime()?["hosts"]["first.entry.test"].is_null());
    assert_eq!(
        runtime()?["hosts"]["new-prefix.entry.test"].as_str(),
        Some("127.0.0.3")
    );
    second.healthy.store(false, Ordering::SeqCst);
    backend
        .request(Command::SetRoutingMode(RoutingMode::Global))
        .await
        .map_err(anyhow::Error::msg)?;
    wait(
        &backend,
        "all entry candidates fail without replacing the last configuration",
        30,
        |s| {
            s.activity
                .iter()
                .any(|entry| entry.message.starts_with("未找到可用入口"))
        },
    )
    .await?;
    assert_eq!(
        runtime()?["hosts"]["new-prefix.entry.test"].as_str(),
        Some("127.0.0.3")
    );
    assert_eq!(backend.snapshot().active.unwrap().key, key);
    let third = proxy_at(&format!("127.0.0.4:{port}")).await?;
    *address.write().unwrap() = "127.0.0.4".into();
    delay_dns.store(true, Ordering::SeqCst);
    let attempts = backend
        .snapshot()
        .activity
        .iter()
        .filter(|entry| entry.message.contains("连接检测失败，正在查找可用入口"))
        .count();
    backend
        .request(Command::SetRoutingMode(RoutingMode::Rule))
        .await
        .map_err(anyhow::Error::msg)?;
    wait(
        &backend,
        "repair is in flight before switching to direct mode",
        15,
        |s| {
            s.activity
                .iter()
                .filter(|entry| entry.message.contains("连接检测失败，正在查找可用入口"))
                .count()
                > attempts
        },
    )
    .await?;
    backend
        .request(Command::SetRoutingMode(RoutingMode::Direct))
        .await
        .map_err(anyhow::Error::msg)?;
    backend
        .request(Command::Stop)
        .await
        .map_err(anyhow::Error::msg)?;
    let stopped = std::fs::read(store.runtime_path())?;
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(std::fs::read(store.runtime_path())?, stopped);
    assert!(!repaired(&backend.snapshot(), 4));
    backend.shutdown();
    dns_task.abort();
    drop(third);
    Ok(())
}

#[tokio::test]
#[ignore = "Runs official Mihomo with bundled GeoIP, GeoSite and ASN databases."]
async fn starts_offline_with_missing_and_damaged_geodata() -> Result<()> {
    use clashfx_core::{mihomo, model::CoreSettings, store::atomic_write};
    let store = Store::open(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../work/geodata-{}", uuid::Uuid::new_v4())),
    ))?;
    atomic_write(&store.root.join("Country.mmdb"), b"interrupted download")?;
    let settings = CoreSettings {
        controller_port: unused_port()?,
        mixed_port: unused_port()?,
        secret: "geodata-fixture".into(),
        ..Default::default()
    };
    for dat_mode in [false, true] {
        let yaml = format!(
            "mixed-port: {}\nexternal-controller: 127.0.0.1:{}\nsecret: geodata-fixture\nmode: rule\ngeodata-mode: {dat_mode}\ngeox-url:\n  mmdb: http://127.0.0.1:9/unavailable\n  geoip: http://127.0.0.1:9/unavailable\n  geosite: http://127.0.0.1:9/unavailable\n  asn: http://127.0.0.1:9/unavailable\nrules:\n  - GEOIP,CN,DIRECT\n  - GEOSITE,google,DIRECT\n  - IP-ASN,13335,DIRECT\n  - MATCH,DIRECT\n",
            settings.mixed_port, settings.controller_port
        );
        atomic_write(&store.runtime_path(), yaml.as_bytes())?;
        let mut child =
            tokio::time::timeout(Duration::from_secs(10), mihomo::start(&settings, &store))
                .await??;
        mihomo::Controller::new(&settings)?.ready().await?;
        child.kill().await?;
        child.wait().await?;
    }
    assert_eq!(
        std::fs::read(store.root.join("Country.invalid.bak"))?,
        b"interrupted download"
    );
    let log = std::fs::read_to_string(store.root.join("core.log"))?;
    assert!(
        !log.contains("start download"),
        "Startup attempted an online database download"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Runs an official Mihomo executable and local routing fixtures."]
async fn live_routing_modes_preserve_node_and_show_startup_and_direct_warnings() -> Result<()> {
    let fixture = proxy().await?;
    let destination = TcpListener::bind("127.0.0.1:0").await?;
    let destination_port = destination.local_addr()?.port();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = destination.accept().await {
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                if header(&mut socket, &mut bytes).await.is_ok() {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\ndirect").await.ok();
                }
            });
        }
    });
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../work")
        .join(format!("routing-{}", uuid::Uuid::new_v4()));
    let store = Store::open(Some(root))?;
    let mut config = store.load()?;
    config.core.controller_port = unused_port()?;
    config.core.mixed_port = unused_port()?;
    config.policy.enabled = false;
    for (name, rule) in [("Primary", "DIRECT"), ("Secondary", "REJECT")] {
        let yaml = format!(
            "proxies:\n - {{name: JP, type: http, server: 127.0.0.1, port: {}}}\nrules:\n - DOMAIN,localhost,{rule}\n - MATCH,Proxy\n",
            fixture.port
        );
        let profile = subscription::parse_profile(&yaml)?;
        let sub = subscription::new_subscription(name.into(), None, &profile);
        store.write_profile(&sub.id, &yaml)?;
        config.subscriptions.push(sub);
    }
    store.save(&config)?;
    let backend = Backend::launch(store.clone())?;
    backend
        .request(Command::Start)
        .await
        .map_err(anyhow::Error::msg)?;
    let original_node = backend.snapshot().active.unwrap().key;
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::http(format!(
            "http://127.0.0.1:{}",
            config.core.mixed_port
        ))?)
        .timeout(Duration::from_secs(3))
        .build()?;
    let local = format!("http://localhost:{destination_port}/rules");
    let numeric = format!("http://127.0.0.1:{destination_port}/rules");
    assert_eq!(
        client.get(&local).send().await?.status().as_u16(),
        200,
        "subscription DIRECT rule must be effective by default"
    );
    assert_eq!(
        client.get(&numeric).send().await?.status().as_u16(),
        204,
        "MATCH must use the selected proxy"
    );
    wait(
        &backend,
        "startup diagnostics captured before WebSocket connection",
        5,
        |s| {
            s.core_logs
                .iter()
                .any(|log| log.message.contains("Start initial configuration"))
        },
    )
    .await?;
    backend
        .request(Command::SetRoutingMode(RoutingMode::Global))
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        client.get(&local).send().await?.status().as_u16(),
        204,
        "global mode must route through CLASHFX_AUTO"
    );
    assert_eq!(
        backend.snapshot().active.as_ref().unwrap().key,
        original_node
    );
    backend
        .request(Command::SetRoutingMode(RoutingMode::Direct))
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        client.get(&numeric).send().await?.status().as_u16(),
        200,
        "direct mode must bypass the proxy"
    );
    let warning_port = unused_port()?;
    client
        .get(format!("http://127.0.0.1:{warning_port}/warning"))
        .send()
        .await
        .ok();
    let snapshot = wait(
        &backend,
        "unrelated DIRECT warnings remain visible",
        5,
        |s| {
            s.core_logs.iter().any(|log| {
                matches!(log.level.as_str(), "warn" | "warning")
                    && log.message.contains(&warning_port.to_string())
            })
        },
    )
    .await?;
    assert_eq!(
        snapshot.failure_streak, 0,
        "DIRECT errors must not trigger proxy failover"
    );
    let warnings: Vec<_> = snapshot
        .core_logs
        .iter()
        .filter(|log| {
            log.message.contains("dial DIRECT") && log.message.contains(&warning_port.to_string())
        })
        .collect();
    let unique: std::collections::HashSet<_> =
        warnings.iter().map(|log| (log.at, &log.message)).collect();
    assert_eq!(
        unique.len(),
        warnings.len(),
        "WebSocket and file logs must not duplicate entries"
    );
    backend
        .request(Command::SetRoutingMode(RoutingMode::Rule))
        .await
        .map_err(anyhow::Error::msg)?;
    backend
        .request(Command::SetRuleSubscription(
            config.subscriptions[1].id.clone(),
        ))
        .await
        .map_err(anyhow::Error::msg)?;
    let rejected = client.get(&local).send().await;
    assert!(
        rejected.is_err()
            || rejected
                .as_ref()
                .is_ok_and(|response| !response.status().is_success()),
        "switching the rule source must apply its REJECT rule live: {rejected:?}"
    );
    assert_eq!(client.get(&numeric).send().await?.status().as_u16(), 204);
    assert_eq!(
        backend.snapshot().active.as_ref().unwrap().key,
        original_node
    );
    assert_eq!(
        store.load()?.rule_subscription,
        Some(config.subscriptions[1].id.clone())
    );
    backend
        .request(Command::ClearLogs)
        .await
        .map_err(anyhow::Error::msg)?;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        backend.snapshot().core_logs.is_empty(),
        "cleared file entries must not reappear"
    );
    backend.shutdown();
    server.abort();
    println!("Evidence: {}", store.root.display());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Runs an official Mihomo executable and a local proxy fixture."]
async fn selecting_a_node_starts_a_stopped_core() -> Result<()> {
    let fixture = proxy().await?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../work")
        .join(format!("manual-start-{}", uuid::Uuid::new_v4()));
    let store = Store::open(Some(root))?;
    let mut config = store.load()?;
    config.core.controller_port = unused_port()?;
    config.core.mixed_port = unused_port()?;
    config.policy.probe_urls = vec!["http://clashfx.test/generate_204".into()];
    store.save(&config)?;
    let path = store.root.join("manual.yaml");
    std::fs::write(
        &path,
        format!(
            "proxies:\n - {{name: Manual node, type: http, server: 127.0.0.1, port: {0}}}\n - {{name: Another node, type: http, server: 127.0.0.1, port: {0}}}",
            fixture.port
        ),
    )?;
    let backend = Backend::launch(store)?;
    backend
        .request(Command::ImportFile {
            notes: String::new(),
            name: "Manual".into(),
            path,
        })
        .await
        .map_err(anyhow::Error::msg)?;
    let key = backend.snapshot().nodes[0].candidate.key.clone();
    let second_key = backend.snapshot().nodes[1].candidate.key.clone();
    let id = backend.snapshot().config.subscriptions[0].id.clone();
    anyhow::ensure!(backend.snapshot().candidates.is_empty());
    backend
        .request(Command::Start)
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(backend.snapshot().running && !backend.snapshot().system_proxy);
    anyhow::ensure!(
        backend
            .snapshot()
            .active
            .as_ref()
            .is_some_and(|node| node.key == key)
    );
    backend
        .request(Command::Stop)
        .await
        .map_err(anyhow::Error::msg)?;
    backend
        .request(Command::SelectNode(key.clone()))
        .await
        .map_err(anyhow::Error::msg)?;
    let state = backend.snapshot();
    anyhow::ensure!(state.running && state.active.as_ref().is_some_and(|node| node.key == key));
    anyhow::ensure!(state.manual_hold_until.is_some());
    anyhow::ensure!(state.config.subscriptions[0].pool_nodes.is_empty());
    backend
        .request(Command::EditSubscription {
            notes: String::new(),
            id: id.clone(),
            name: "Renamed".into(),
            url: None,
        })
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        backend
            .snapshot()
            .active
            .as_ref()
            .is_some_and(|node| node.key == key)
    );
    backend
        .request(Command::SelectNode(second_key.clone()))
        .await
        .map_err(anyhow::Error::msg)?;
    backend
        .request(Command::Stop)
        .await
        .map_err(anyhow::Error::msg)?;
    backend
        .request(Command::Start)
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        backend
            .snapshot()
            .active
            .as_ref()
            .is_some_and(|node| node.key == second_key)
    );
    backend
        .request(Command::SystemProxy(false))
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(backend.snapshot().running && !backend.snapshot().system_proxy);
    backend
        .request(Command::SelectNode(second_key.clone()))
        .await
        .map_err(anyhow::Error::msg)?;
    let hold = backend.snapshot().manual_hold_until;
    backend
        .request(Command::TestNodes(Some(id.clone())))
        .await
        .map_err(anyhow::Error::msg)?;
    let tested = wait(
        &backend,
        "latency testing preserves manual selection and candidate scope",
        8,
        |state| !state.testing_nodes,
    )
    .await?;
    anyhow::ensure!(
        tested
            .active
            .as_ref()
            .is_some_and(|node| node.key == second_key)
    );
    anyhow::ensure!(tested.manual_hold_until == hold && tested.candidates.is_empty());
    anyhow::ensure!(
        tested
            .probes
            .get(&key)
            .is_some_and(|probe| probe.delay_ms.is_some())
    );
    anyhow::ensure!(
        tested
            .probes
            .get(&second_key)
            .is_some_and(|probe| probe.delay_ms.is_some())
    );
    backend
        .request(Command::SetPoolNodes {
            id: id.clone(),
            names: vec!["Manual node".into()],
            in_pool: true,
        })
        .await
        .map_err(anyhow::Error::msg)?;
    backend
        .request(Command::Resume)
        .await
        .map_err(anyhow::Error::msg)?;
    wait(&backend, "resume uses the pool", 8, |s| {
        !s.probing
            && s.manual_hold_until.is_none()
            && s.active.as_ref().is_some_and(|node| node.key == key)
    })
    .await?;
    backend
        .request(Command::SetPoolNodes {
            id,
            names: vec!["Manual node".into()],
            in_pool: false,
        })
        .await
        .map_err(anyhow::Error::msg)?;
    let empty = backend.snapshot();
    anyhow::ensure!(
        empty.candidates.is_empty()
            && empty.running
            && empty.active.as_ref().is_some_and(|node| node.key == key)
    );
    #[cfg(windows)]
    {
        let backup = std::path::PathBuf::from(&empty.data_dir).join("proxy-backup.json");
        std::fs::write(&backup, b"{")?;
        let result = backend.request(Command::Stop).await;
        std::fs::remove_file(&backup)?;
        anyhow::ensure!(result.is_err(), "invalid proxy ownership was ignored");
        anyhow::ensure!(
            backend.snapshot().running,
            "proxy restore failure stopped the core"
        );
        anyhow::ensure!(
            TcpStream::connect(("127.0.0.1", config.core.controller_port))
                .await
                .is_ok(),
            "proxy restore failure left an unavailable core"
        );
    }
    backend.shutdown();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Runs an official Mihomo executable and local proxy fixtures; fetch the core first."]
async fn real_core_log_driven_failover_recovery_and_manual_override() -> Result<()> {
    let a1 = proxy().await?;
    let a2 = proxy().await?;
    let b1 = proxy().await?;
    let excluded = proxy().await?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../work")
        .join(format!("integration-{}", uuid::Uuid::new_v4()));
    let store = Store::open(Some(root))?;
    let mut config = store.load()?;
    config.core.controller_port = unused_port()?;
    config.core.mixed_port = unused_port()?;
    config.policy = AutoPolicy {
        failure_threshold: 3,
        probe_timeout_ms: 500,
        probe_urls: vec!["http://clashfx.test/generate_204".into()],
        cooldown_secs: 5,
        check_interval_secs: 10,
        recovery_successes: 2,
        recovery_enabled: true,
        unavailable_retry_secs: 5,
        manual_hold_secs: 30,
        ..Default::default()
    };
    store.save(&config)?;
    let a_path = store.root.join("a.yaml");
    let b_path = store.root.join("b.yaml");
    std::fs::write(
        &a_path,
        format!(
            "proxies:\n  - {{name: JP 01, type: http, server: 127.0.0.1, port: {}}}\n  - {{name: JP 02, type: http, server: 127.0.0.1, port: {}}}\n",
            a1.port, a2.port
        ),
    )?;
    std::fs::write(
        &b_path,
        format!(
            "proxies:\n  - {{name: JP backup, type: http, server: 127.0.0.1, port: {}}}\n  - {{name: HK excluded, type: http, server: 127.0.0.1, port: {}}}\n",
            b1.port, excluded.port
        ),
    )?;
    use std::io::Write;
    std::fs::OpenOptions::new().append(true).open(&a_path)?.write_all(b"rules:\n  - RULE-SET,offline,OldPolicy\n  - MATCH,OldPolicy\nrule-providers:\n  offline: {type: http, behavior: domain, url: 'http://127.0.0.1:1/unavailable', path: ./offline.yaml}\n")?;
    let backend = Backend::launch(store.clone())?;
    backend.send(Command::ImportFile {
        notes: String::new(),
        name: "Primary A".into(),
        path: a_path,
    });
    backend.send(Command::ImportFile {
        notes: String::new(),
        name: "Backup B".into(),
        path: b_path,
    });
    let snapshot = wait(&backend, "import both subscriptions", 10, |s| {
        s.config.subscriptions.len() == 2
    })
    .await?;
    let backup_id = snapshot.config.subscriptions[1].id.clone();
    backend.send(Command::SetPoolNodes {
        id: snapshot.config.subscriptions[0].id.clone(),
        names: vec!["JP 01".into(), "JP 02".into()],
        in_pool: true,
    });
    backend.send(Command::SetPoolNodes {
        id: backup_id,
        names: vec!["JP backup".into()],
        in_pool: true,
    });
    wait(&backend, "pool excludes healthy HK node", 5, |s| {
        s.candidates.len() == 3
    })
    .await?;
    backend.send(Command::Start);
    wait(&backend, "start native backend and real Mihomo", 30, |s| {
        s.running
            && !s.probing
            && s.log_connected
            && s.active.as_ref().is_some_and(|n| n.name == "JP 01")
    })
    .await?;

    wait(
        &backend,
        "healthy startup leaves spare pool nodes unchecked",
        8,
        |s| {
            s.checking_nodes.is_empty()
                && s.probes.len() == 1
                && s.probes.values().all(|result| result.delay_ms.is_some())
                && s.active.as_ref().is_some_and(|node| node.name == "JP 01")
        },
    )
    .await?;
    a1.healthy.store(false, Ordering::SeqCst);
    fail_requests(config.core.mixed_port).await?;
    let same = wait(
        &backend,
        "three real failure logs select another node in the same subscription",
        20,
        |s| s.active.as_ref().is_some_and(|n| n.name == "JP 02"),
    )
    .await?;
    anyhow::ensure!(
        same.activity
            .iter()
            .any(|line| line.message.contains("连续 3 次")),
        "failure was not triggered by connection logs"
    );
    anyhow::ensure!(
        same.last_switch
            .as_ref()
            .is_some_and(|transition| transition
                .from
                .as_ref()
                .is_some_and(|node| node.name == "JP 01")
                && transition.to.name == "JP 02"),
        "switch flow did not identify both nodes"
    );
    tokio::time::sleep(Duration::from_secs(6)).await;
    a2.healthy.store(false, Ordering::SeqCst);
    fail_requests(config.core.mixed_port).await?;
    wait(
        &backend,
        "all primary nodes fail and backup subscription is selected",
        20,
        |s| s.active.as_ref().is_some_and(|n| n.name == "JP backup"),
    )
    .await?;

    a1.healthy.store(true, Ordering::SeqCst);
    let recovered = wait(
        &backend,
        "primary recovers only after consecutive successful checks",
        40,
        |s| s.active.as_ref().is_some_and(|n| n.name == "JP 01"),
    )
    .await?;
    anyhow::ensure!(
        recovered
            .activity
            .iter()
            .any(|line| line.message.contains("恢复检测 1/2")),
        "recovery confirmation was skipped"
    );
    let backup = recovered
        .candidates
        .iter()
        .find(|node| node.name == "JP backup")
        .context("backup missing")?
        .key
        .clone();
    backend.send(Command::SelectNode(backup));
    wait(&backend, "manual selection pauses automation", 5, |s| {
        s.manual_hold_until.is_some()
    })
    .await?;
    tokio::time::sleep(Duration::from_secs(11)).await;
    anyhow::ensure!(
        backend
            .snapshot()
            .active
            .as_ref()
            .is_some_and(|node| node.name == "JP backup"),
        "manual hold was overridden"
    );
    backend.send(Command::Resume);
    wait(
        &backend,
        "explicit resume returns to the healthy preferred node",
        10,
        |s| s.active.as_ref().is_some_and(|node| node.name == "JP 01") && !s.probing,
    )
    .await?;

    tokio::time::sleep(Duration::from_secs(6)).await;
    a1.healthy.store(false, Ordering::SeqCst);
    b1.healthy.store(false, Ordering::SeqCst);
    fail_requests(config.core.mixed_port).await?;
    let unavailable = wait(
        &backend,
        "all selected nodes fail without falling back to unselected healthy nodes or DIRECT",
        20,
        |s| s.phase == "池内节点全部不可用" && !s.probing,
    )
    .await?;
    anyhow::ensure!(
        unavailable.running
            && unavailable
                .active
                .as_ref()
                .is_some_and(|node| node.name == "JP 01"),
        "all-down handling changed the route unexpectedly"
    );
    anyhow::ensure!(
        unavailable.candidates.iter().all(|node| unavailable
            .probes
            .get(&node.key)
            .is_some_and(|result| result.delay_ms.is_none() && result.checked_at.is_some())),
        "all-down results must cover every pool node"
    );
    anyhow::ensure!(
        unavailable.retry_at.is_some(),
        "all-down state must expose the next retry"
    );
    let mut paused = unavailable.config.policy;
    paused.enabled = false;
    backend.send(Command::SavePolicy(paused));
    b1.healthy.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(6)).await;
    anyhow::ensure!(
        backend
            .snapshot()
            .active
            .as_ref()
            .is_some_and(|node| node.name == "JP 01"),
        "paused automation retried anyway"
    );
    backend.send(Command::Probe);
    wait(
        &backend,
        "manual detection works while automatic switching is paused",
        15,
        |s| {
            s.active
                .as_ref()
                .is_some_and(|node| node.name == "JP backup")
                && !s.probing
        },
    )
    .await?;
    backend.shutdown();
    anyhow::ensure!(
        store.root.join("cache.db").is_file(),
        "Mihomo could not persist its cache using the Windows data path"
    );
    anyhow::ensure!(
        !backend.snapshot().running,
        "shutdown did not stop the core"
    );
    anyhow::ensure!(
        TcpStream::connect(("127.0.0.1", config.core.controller_port))
            .await
            .is_err(),
        "owned core was left running"
    );
    anyhow::ensure!(
        store.load()?.subscriptions[1].pool_nodes == ["JP backup"],
        "pool membership was not persisted"
    );
    println!("Evidence: {}", store.root.display());
    Ok(())
}
