use anyhow::Result;
use clashfx_core::{engine::Backend, store::Store, subscription};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const ORIGINAL: &str = "proxies:\n  - {name: JP old, type: http, server: 127.0.0.1, port: 12345}\n";
const UPDATED: &str = "proxies:\n  - {name: JP new, type: http, server: 127.0.0.1, port: 12345}\n";

async fn update_fixture(
    status: u16,
) -> Result<(
    Store,
    Backend,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!(
        "http://127.0.0.1:{}/subscription",
        listener.local_addr()?.port()
    );
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut bytes = [0; 4096];
            if !matches!(socket.read(&mut bytes).await, Ok(size) if size > 0) {
                continue;
            }
            observed.fetch_add(1, Ordering::SeqCst);
            let reply = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{UPDATED}",
                UPDATED.len()
            );
            socket.write_all(reply.as_bytes()).await.ok();
        }
    });
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../work")
        .join(format!("updates-{}", uuid::Uuid::new_v4()));
    let store = Store::open(Some(path))?;
    let profile = subscription::parse_profile(ORIGINAL)?;
    let mut sub = subscription::new_subscription("Update fixture".into(), Some(url), &profile);
    sub.pool_nodes = vec!["JP old".into()];
    sub.updated_at = chrono::Utc::now() - chrono::Duration::hours(25);
    store.write_profile(&sub.id, ORIGINAL)?;
    let mut config = store.load()?;
    config.subscriptions.push(sub);
    store.save(&config)?;
    let backend = Backend::launch(store.clone())?;
    Ok((store, backend, requests, task))
}

#[tokio::test]
async fn automatic_update_keeps_an_explicit_allowlist_when_nodes_are_renamed() -> Result<()> {
    let (store, backend, _, task) = update_fixture(200).await?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while backend
        .snapshot()
        .nodes
        .first()
        .is_none_or(|node| node.candidate.name != "JP new")
    {
        anyhow::ensure!(
            Instant::now() < deadline,
            "daily subscription update did not run"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let snapshot = backend.snapshot();
    assert!(snapshot.candidates.is_empty());
    assert_eq!(snapshot.config.subscriptions[0].pool_nodes, ["JP old"]);
    assert_eq!(store.load()?.subscriptions[0].pool_nodes, ["JP old"]);
    backend.shutdown();
    task.abort();
    Ok(())
}

#[tokio::test]
async fn failed_automatic_update_keeps_the_last_profile_and_backs_off() -> Result<()> {
    let (store, backend, requests, task) = update_fixture(500).await?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while backend.snapshot().last_error.is_none() {
        anyhow::ensure!(Instant::now() < deadline, "failed update was not reported");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snapshot = backend.snapshot();
    assert_eq!(snapshot.candidates.len(), 1);
    assert_eq!(
        std::fs::read_to_string(store.profile_path(&snapshot.config.subscriptions[0].id)?)?,
        ORIGINAL
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    backend.shutdown();
    task.abort();
    Ok(())
}
