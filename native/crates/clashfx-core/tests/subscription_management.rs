use anyhow::{Result, anyhow};
use clashfx_core::{
    engine::{Backend, Command},
    store::Store,
};
use std::{path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../work")
        .join(format!("subscription-test-{}", uuid::Uuid::new_v4()))
}

fn profile(names: &[&str]) -> String {
    serde_json::json!({"proxies": names.iter().map(|name| serde_json::json!({"name": name, "type": "http", "server": "127.0.0.1", "port": 65501})).collect::<Vec<_>>()}).to_string()
}

async fn command(backend: &Backend, command: Command) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), backend.request(command))
        .await?
        .map_err(|error| anyhow!(error))
}

#[tokio::test]
async fn failed_core_start_never_enables_system_proxy() -> Result<()> {
    let store = Store::open(Some(workspace()))?;
    let mut config = store.load()?;
    config.core.binary_path = store
        .root
        .join("missing-mihomo.exe")
        .to_string_lossy()
        .into();
    store.save(&config)?;
    let backend = Backend::launch(store.clone())?;
    assert!(command(&backend, Command::SystemProxy(true)).await.is_err());
    let file = store.root.join("nodes.yaml");
    std::fs::write(&file, profile(&["Manual node"]))?;
    command(
        &backend,
        Command::ImportFile {
            notes: "Local platform\nManual subscription".into(),
            name: "Fixture".into(),
            path: file,
        },
    )
    .await?;
    let error = command(&backend, Command::SystemProxy(true))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("内核文件不存在"));
    assert!(!backend.snapshot().system_proxy && !backend.snapshot().running);
    assert!(!store.root.join("proxy-backup.json").exists());
    assert_eq!(
        store.load()?.subscriptions[0].notes,
        "Local platform\nManual subscription"
    );
    command(&backend, Command::SystemProxy(false)).await?;
    backend.shutdown();
    Ok(())
}

#[tokio::test]
async fn editing_source_is_transactional_and_survives_restart() -> Result<()> {
    let root = workspace();
    let store = Store::open(Some(root.clone()))?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                let size = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..size]);
                let (status, body) = if request.starts_with("GET /bad") {
                    ("503 Service Unavailable", "unavailable".to_owned())
                } else if request.starts_with("GET /second") {
                    ("200 OK", profile(&["Tokyo A", "Tokyo C", "Tokyo New"]))
                } else {
                    ("200 OK", profile(&["Tokyo A", "Tokyo B", "Tokyo C"]))
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.ok();
            });
        }
    });
    let backend = Backend::launch(store.clone())?;
    let first_url = format!("{base}/first?token=test-only");
    command(
        &backend,
        Command::ImportUrl {
            notes: "Example platform\nMonthly plan".into(),
            name: "Primary".into(),
            url: first_url.clone(),
        },
    )
    .await?;
    let id = backend.snapshot().config.subscriptions[0].id.clone();
    assert_eq!(
        store.load()?.subscriptions[0].notes,
        "Example platform\nMonthly plan"
    );
    assert!(
        command(
            &backend,
            Command::ImportUrl {
                notes: String::new(),
                name: "Duplicate".into(),
                url: first_url
            }
        )
        .await
        .is_err()
    );
    assert_eq!(backend.snapshot().config.subscriptions.len(), 1);
    command(
        &backend,
        Command::SetPoolNodes {
            id: id.clone(),
            names: vec!["Tokyo A".into(), "Tokyo C".into()],
            in_pool: true,
        },
    )
    .await?;
    let url = format!("{base}/second?token=replaced");
    command(
        &backend,
        Command::EditSubscription {
            notes: "Example platform\nAnnual plan\nAccount renewal in November".into(),
            id: id.clone(),
            name: "Renamed".into(),
            url: Some(url.clone()),
        },
    )
    .await?;
    let snapshot = backend.snapshot();
    let sub = &snapshot.config.subscriptions[0];
    assert_eq!(sub.id, id);
    assert_eq!(sub.name, "Renamed");
    assert_eq!(sub.url.as_deref(), Some(url.as_str()));
    assert_eq!(sub.pool_nodes, ["Tokyo A", "Tokyo C"]);
    assert_eq!(
        sub.notes,
        "Example platform\nAnnual plan\nAccount renewal in November"
    );
    assert!(
        snapshot
            .nodes
            .iter()
            .any(|node| node.candidate.name == "Tokyo New" && !node.in_pool)
    );
    command(&backend, Command::UpdateSubscription(id.clone())).await?;
    assert_eq!(backend.snapshot().config.subscriptions[0].notes, sub.notes);
    command(
        &backend,
        Command::EditSubscription {
            id: id.clone(),
            name: sub.name.clone(),
            url: sub.url.clone(),
            notes: "Updated notes only\nNo source change".into(),
        },
    )
    .await?;
    assert_eq!(
        store.load()?.subscriptions[0].notes,
        "Updated notes only\nNo source change"
    );
    let old_config = serde_json::to_value(&backend.snapshot().config)?;
    let old_profile = std::fs::read(store.profile_path(&id)?)?;
    for new_url in [
        format!("{base}/bad"),
        "ftp://invalid/source".into(),
        String::new(),
    ] {
        assert!(
            command(
                &backend,
                Command::EditSubscription {
                    notes: String::new(),
                    id: id.clone(),
                    name: "Must not persist".into(),
                    url: Some(new_url),
                }
            )
            .await
            .is_err()
        );
        assert_eq!(
            serde_json::to_value(&backend.snapshot().config)?,
            old_config
        );
        assert_eq!(std::fs::read(store.profile_path(&id)?)?, old_profile);
    }
    backend.shutdown();
    let reopened = Backend::launch(store)?;
    assert_eq!(
        serde_json::to_value(&reopened.snapshot().config)?,
        old_config
    );
    reopened.shutdown();
    server.abort();
    Ok(())
}

#[tokio::test]
async fn pool_transfers_and_ordering_preserve_unrelated_nodes() -> Result<()> {
    let root = workspace();
    let store = Store::open(Some(root.clone()))?;
    let source = root.join("provider.yaml");
    std::fs::write(&source, profile(&["Tokyo A", "Tokyo B", "Singapore"]))?;
    let backend = Backend::launch(store.clone())?;
    for _ in 0..3 {
        command(
            &backend,
            Command::ImportFile {
                notes: String::new(),
                name: "Provider".into(),
                path: source.clone(),
            },
        )
        .await?;
    }
    let subscriptions = backend.snapshot().config.subscriptions;
    assert_eq!(
        subscriptions
            .iter()
            .map(|sub| sub.name.as_str())
            .collect::<Vec<_>>(),
        ["Provider", "Provider (2)", "Provider (3)"]
    );
    assert!(backend.snapshot().candidates.is_empty());
    assert_eq!(backend.snapshot().nodes.len(), 9);
    let ids: Vec<_> = subscriptions.iter().map(|sub| sub.id.clone()).collect();
    for id in &ids {
        command(
            &backend,
            Command::SetPoolNodes {
                id: id.clone(),
                names: vec!["Tokyo A".into(), "Tokyo B".into(), "Singapore".into()],
                in_pool: true,
            },
        )
        .await?;
    }
    command(
        &backend,
        Command::MovePoolSubscriptionTo {
            id: ids[0].clone(),
            index: 2,
        },
    )
    .await?;
    assert_eq!(
        backend.snapshot().config.pool_order,
        [ids[1].clone(), ids[2].clone(), ids[0].clone()]
    );
    assert_eq!(
        backend
            .snapshot()
            .config
            .subscriptions
            .iter()
            .map(|sub| sub.id.clone())
            .collect::<Vec<_>>(),
        ids
    );
    command(
        &backend,
        Command::MovePoolNodeTo {
            id: ids[0].clone(),
            name: "Singapore".into(),
            index: 0,
        },
    )
    .await?;
    command(
        &backend,
        Command::SetPoolNodes {
            id: ids[0].clone(),
            names: vec!["Tokyo A".into()],
            in_pool: false,
        },
    )
    .await?;
    assert_eq!(
        backend.snapshot().config.subscriptions[0].pool_nodes,
        ["Singapore", "Tokyo B"]
    );
    command(
        &backend,
        Command::SetPoolNodes {
            id: ids[0].clone(),
            names: vec!["Tokyo B".into()],
            in_pool: true,
        },
    )
    .await?;
    assert_eq!(
        backend.snapshot().config.subscriptions[0].pool_nodes,
        ["Singapore", "Tokyo B"]
    );
    assert!(
        command(
            &backend,
            Command::SetPoolNodes {
                id: ids[0].clone(),
                names: vec!["Missing".into()],
                in_pool: true
            }
        )
        .await
        .is_err()
    );
    let replacement = root.join("replacement.yaml");
    std::fs::write(&replacement, profile(&["Tokyo B", "Tokyo New"]))?;
    command(
        &backend,
        Command::ReplaceSubscriptionFile {
            id: ids[0].clone(),
            path: replacement,
        },
    )
    .await?;
    assert_eq!(
        backend.snapshot().config.subscriptions[0].pool_nodes,
        ["Singapore", "Tokyo B"]
    );
    assert!(
        backend
            .snapshot()
            .nodes
            .iter()
            .any(|node| node.candidate.subscription_id == ids[0]
                && node.candidate.name == "Tokyo New"
                && !node.in_pool)
    );
    command(
        &backend,
        Command::SetPoolNodes {
            id: ids[0].clone(),
            names: vec!["Singapore".into(), "Tokyo B".into()],
            in_pool: false,
        },
    )
    .await?;
    assert!(
        backend.snapshot().config.subscriptions[0]
            .pool_nodes
            .is_empty()
    );
    assert_eq!(
        backend.snapshot().config.pool_order,
        [ids[1].clone(), ids[2].clone()]
    );
    command(
        &backend,
        Command::SetPoolNodes {
            id: ids[0].clone(),
            names: vec!["Tokyo B".into()],
            in_pool: true,
        },
    )
    .await?;
    assert_eq!(backend.snapshot().config.pool_order.last(), Some(&ids[0]));
    let saved = serde_json::to_value(&backend.snapshot().config)?;
    backend.shutdown();
    assert_eq!(serde_json::to_value(store.load()?)?, saved);
    Ok(())
}

#[test]
fn previous_defaults_migrate_to_stable_connections_once() -> Result<()> {
    let store = Store::open(Some(workspace()))?;
    let mut old = serde_json::to_value(store.load()?)?;
    old["schema_version"] = 2.into();
    old["policy"]["check_interval_secs"] = 60.into();
    old["policy"]["probe_timeout_ms"] = 4000.into();
    old["policy"]["recovery_enabled"] = true.into();
    let original = serde_json::to_vec(&old)?;
    std::fs::write(store.root.join("settings.json"), &original)?;
    let mut config = store.load()?;
    assert_eq!(config.policy.check_interval_secs, 300);
    assert_eq!(config.policy.probe_timeout_ms, 3000);
    assert!(!config.policy.recovery_enabled);
    assert_eq!(
        std::fs::read(store.root.join("settings.v2.backup.json"))?,
        original
    );
    config.policy.recovery_enabled = true;
    config.policy.check_interval_secs = 60;
    store.save(&config)?;
    assert!(store.load()?.policy.recovery_enabled);
    assert_eq!(store.load()?.policy.check_interval_secs, 60);
    old["policy"]["check_interval_secs"] = 120.into();
    old["policy"]["probe_timeout_ms"] = 12000.into();
    std::fs::write(store.root.join("settings.json"), serde_json::to_vec(&old)?)?;
    let custom = store.load()?;
    assert_eq!(custom.policy.check_interval_secs, 120);
    assert_eq!(custom.policy.probe_timeout_ms, 12000);
    old["schema_version"] = 3.into();
    old["policy"]["probe_timeout_ms"] = 8000.into();
    std::fs::write(store.root.join("settings.json"), serde_json::to_vec(&old)?)?;
    let upgraded = store.load()?;
    assert_eq!(upgraded.schema_version, 4);
    assert_eq!(upgraded.policy.probe_timeout_ms, 3000);
    assert!(upgraded.policy.recovery_enabled);
    assert_eq!(upgraded.policy.check_interval_secs, 120);
    assert!(store.root.join("settings.v3.backup.json").exists());
    Ok(())
}

#[test]
fn legacy_filters_migrate_to_an_explicit_pool_without_expanding_it() -> Result<()> {
    let store = Store::open(Some(workspace()))?;
    let ids: Vec<_> = (0..3).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let mut legacy = serde_json::to_value(store.load()?)?;
    legacy["schema_version"] = 1.into();
    legacy.as_object_mut().unwrap().remove("pool_order");
    legacy["subscriptions"] = serde_json::json!([
        {"id": ids[0], "name": "Primary", "url": null, "enabled": true, "region_filter": "JP", "selected_nodes": ["JP B", "HK A", "JP A"], "updated_at": chrono::Utc::now()},
        {"id": ids[1], "name": "Previously disabled", "url": null, "enabled": false, "region_filter": "", "selected_nodes": ["JP A"], "updated_at": chrono::Utc::now()},
        {"id": ids[2], "name": "Empty", "url": null, "enabled": true, "region_filter": "", "selected_nodes": [], "updated_at": chrono::Utc::now()}
    ]);
    for id in &ids {
        store.write_profile(id, &profile(&["JP A", "JP B", "HK A"]))?;
    }
    let original = serde_json::to_vec_pretty(&legacy)?;
    std::fs::write(store.root.join("settings.json"), &original)?;
    let migrated = store.load()?;
    assert_eq!(migrated.schema_version, 4);
    assert!(
        migrated
            .subscriptions
            .iter()
            .all(|sub| sub.notes.is_empty())
    );
    assert_eq!(migrated.subscriptions[0].pool_nodes, ["JP B", "JP A"]);
    assert!(migrated.subscriptions[1].pool_nodes.is_empty());
    assert!(migrated.subscriptions[2].pool_nodes.is_empty());
    assert_eq!(migrated.pool_order, [ids[0].clone()]);
    let profiles = clashfx_core::subscription::load_profiles(&migrated, &store)?;
    assert_eq!(
        clashfx_core::subscription::node_views(&migrated, &profiles)?.len(),
        9
    );
    assert_eq!(
        clashfx_core::subscription::candidates(&migrated, &profiles)?
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>(),
        ["JP B", "JP A"]
    );
    let serialized = serde_json::to_value(&migrated)?;
    assert!(serialized["subscriptions"][0].get("enabled").is_none());
    assert!(
        serialized["subscriptions"][0]
            .get("region_filter")
            .is_none()
    );
    assert_eq!(
        std::fs::read(store.root.join("settings.v1.backup.json"))?,
        original
    );
    assert_eq!(serde_json::to_value(store.load()?)?, serialized);
    Ok(())
}
