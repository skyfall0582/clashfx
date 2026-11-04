use crate::{
    mihomo::{self, Controller},
    model::{AutoPolicy, CoreSettings, EntryRepairSettings},
    store::{Store, atomic_write},
};
use anyhow::{Context, Result};
use futures_util::{FutureExt, StreamExt, stream};
use serde::Deserialize;
use serde_yaml_ng::{Mapping, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    path::PathBuf,
    time::Duration,
};

#[derive(Clone)]
pub(crate) struct Target {
    pub key: String,
    pub host: String,
    pub port: u16,
    pub proxy: Mapping,
}

impl Target {
    pub fn new(key: String, proxy: &Mapping) -> Option<Self> {
        let host = proxy.get("server")?.as_str()?;
        let port = proxy.get("port")?;
        let port = port
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .or_else(|| port.as_str()?.parse::<u16>().ok())?;
        // A chained node resolves its entry through another proxy, not this machine.
        if host.parse::<IpAddr>().is_ok()
            || proxy
                .get("dialer-proxy")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty())
            || port == 0
        {
            return None;
        }
        reqwest::Url::parse(&format!("https://{host}/")).ok()?;
        let mut proxy = proxy.clone();
        proxy.insert("name".into(), key.clone().into());
        Some(Self {
            key,
            host: host.trim_end_matches('.').to_ascii_lowercase(),
            port,
            proxy,
        })
    }
}

pub(crate) struct Repair {
    pub host: String,
    pub ip: IpAddr,
    pub delays: BTreeMap<String, u64>,
}

pub(crate) fn overlay(body: &str, pins: &BTreeMap<String, IpAddr>) -> Result<String> {
    if pins.is_empty() {
        return Ok(body.to_owned());
    }
    let mut root: Mapping = serde_yaml_ng::from_str(body)?;
    let domains: BTreeSet<_> = root
        .get("proxies")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|proxy| proxy.get("server").and_then(Value::as_str))
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
        .collect();
    let pins: Vec<_> = pins
        .iter()
        .filter(|(host, _)| domains.contains(*host))
        .collect();
    if pins.is_empty() {
        return Ok(body.to_owned());
    }
    let hosts = root
        .entry(Value::from("hosts"))
        .or_insert_with(|| Value::Mapping(Mapping::new()))
        .as_mapping_mut()
        .context("Invalid hosts configuration")?;
    for (host, ip) in pins {
        hosts.insert(host.clone().into(), ip.to_string().into());
    }
    if let Some(dns) = root.get_mut("dns").and_then(Value::as_mapping_mut) {
        dns.insert("use-hosts".into(), true.into());
    }
    Ok(serde_yaml_ng::to_string(&root)?)
}

#[derive(Deserialize)]
struct DnsAnswer {
    name: String,
    #[serde(rename = "type")]
    kind: u16,
    data: String,
}

#[derive(Deserialize)]
struct DnsResponse {
    #[serde(rename = "Status")]
    status: u16,
    #[serde(rename = "Answer", default)]
    answers: Vec<DnsAnswer>,
}

async fn query(
    client: &reqwest::Client,
    resolver: &str,
    host: &str,
    kind: &str,
) -> Result<Vec<IpAddr>> {
    let response: DnsResponse = client
        .get(resolver)
        .header("Accept", "application/dns-json")
        .query(&[("name", host), ("type", kind)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    anyhow::ensure!(response.status == 0, "DNS returned a failure response");
    let normalize = |name: &str| name.trim_end_matches('.').to_ascii_lowercase();
    let mut names = BTreeSet::from([normalize(host)]);
    for _ in 0..response.answers.len() {
        let previous = names.len();
        for answer in &response.answers {
            if answer.kind == 5 && names.contains(&normalize(&answer.name)) {
                names.insert(normalize(&answer.data));
            }
        }
        if names.len() == previous {
            break;
        }
    }
    Ok(response
        .answers
        .iter()
        .filter(|answer| matches!(answer.kind, 1 | 28) && names.contains(&normalize(&answer.name)))
        .filter_map(|answer| answer.data.parse().ok())
        .collect())
}

async fn resolve(target: &Target, settings: &EntryRepairSettings) -> Result<Vec<IpAddr>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?;
    let system = async {
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((target.host.as_str(), target.port)),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .map(|addresses| addresses.map(|address| address.ip()).collect::<Vec<_>>())
        .unwrap_or_default()
    };
    let mut requests = Vec::new();
    for url in &settings.resolvers {
        for kind in ["A", "AAAA"] {
            let client = client.clone();
            let url = url.clone();
            let host = target.host.clone();
            requests.push(async move { query(&client, &url, &host, kind).await }.boxed());
        }
    }
    let public = async {
        stream::iter(requests)
            .buffer_unordered(8)
            .collect::<Vec<_>>()
            .await
    };
    let (system, public) = tokio::join!(system, public);
    let mut seen = BTreeSet::new();
    Ok(system
        .into_iter()
        .chain(public.into_iter().filter_map(Result::ok).flatten())
        .filter(|ip| !ip.is_unspecified() && !ip.is_multicast() && seen.insert(*ip))
        .take(16)
        .collect())
}

pub(crate) async fn repair(
    root: PathBuf,
    binary: PathBuf,
    settings: EntryRepairSettings,
    policy: AutoPolicy,
    target: Target,
    runtime: Mapping,
) -> Result<Option<Repair>> {
    let ips = resolve(&target, &settings).await?;
    if ips.is_empty() {
        return Ok(None);
    }
    let store = Store::open(Some(root))?;
    let core = CoreSettings {
        binary_path: binary.to_string_lossy().into_owned(),
        controller_port: std::net::TcpListener::bind(("127.0.0.1", 0))?
            .local_addr()?
            .port(),
        mixed_port: 0,
        ..Default::default()
    };
    let controller = Controller::new(&core)?;
    let mut config = Mapping::new();
    config.insert(
        "external-controller".into(),
        format!("127.0.0.1:{}", core.controller_port).into(),
    );
    config.insert("secret".into(), core.secret.clone().into());
    config.insert("ipv6".into(), true.into());
    config.insert("log-level".into(), "silent".into());
    config.insert(
        "dns".into(),
        serde_yaml_ng::from_str("enable: false\nuse-hosts: true")?,
    );
    for key in ["hosts", "global-client-fingerprint", "global-ua"] {
        if let Some(value) = runtime.get(key) {
            config.insert(key.into(), value.clone());
        }
    }
    let nodes = [&target];
    config.insert(
        "proxies".into(),
        Value::Sequence(
            nodes
                .iter()
                .map(|node| Value::Mapping(node.proxy.clone()))
                .collect(),
        ),
    );
    let base = serde_yaml_ng::to_string(&config)?;
    let mut child = None;
    for ip in ips {
        let body = overlay(&base, &BTreeMap::from([(target.host.clone(), ip)]))?;
        atomic_write(&store.runtime_path(), body.as_bytes())?;
        if child.is_none() {
            child = Some(mihomo::start(&core, &store).await?);
        } else {
            controller.reload(&store.runtime_path()).await?;
        }
        let mut delays = BTreeMap::new();
        for node in &nodes {
            match controller.probe(&node.key, &policy).await {
                Ok(delay) => {
                    delays.insert(node.key.clone(), delay);
                }
                Err(_) => break,
            }
        }
        if delays.len() == nodes.len() {
            if let Some(child) = &mut child {
                child.kill().await.ok();
                child.wait().await.ok();
            }
            return Ok(Some(Repair {
                host: target.host,
                ip,
                delays,
            }));
        }
    }
    if let Some(child) = &mut child {
        child.kill().await.ok();
        child.wait().await.ok();
    }
    Ok(None)
}
