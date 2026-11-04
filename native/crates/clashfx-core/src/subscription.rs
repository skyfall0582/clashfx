use crate::{
    model::{AUTO_GROUP, AppConfig, Candidate, NodeView, Subscription, SubscriptionUsage},
    store::Store,
};
use anyhow::{Context, Result};
use serde_yaml_ng::{Mapping, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct ProxyNode {
    pub name: String,
    pub kind: String,
    pub value: Mapping,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub root: Mapping,
    pub nodes: Vec<ProxyNode>,
}

pub fn parse_profile(body: &str) -> Result<Profile> {
    anyhow::ensure!(body.len() <= 20 * 1024 * 1024, "订阅文件不能超过 20 MB");
    let value: Value =
        serde_yaml_ng::from_str(body).context("请使用 Clash / Mihomo YAML 格式的订阅")?;
    let root = value.as_mapping().context("订阅必须是 YAML 配置")?.clone();
    let proxies = root
        .get("proxies")
        .and_then(Value::as_sequence)
        .context("订阅缺少 proxies 节点列表；请使用包含节点的 Clash / Mihomo YAML 订阅")?;
    anyhow::ensure!(
        !proxies.is_empty() && proxies.len() <= 5000,
        "订阅须包含 1–5000 个节点"
    );
    let mut names = HashSet::new();
    let mut nodes = Vec::new();
    for value in proxies {
        let mapping = value.as_mapping().context("节点格式无效")?;
        let name = mapping
            .get("name")
            .and_then(Value::as_str)
            .context("节点缺少名称")?;
        let kind = mapping
            .get("type")
            .and_then(Value::as_str)
            .context("节点缺少协议类型")?;
        anyhow::ensure!(
            !name.trim().is_empty() && name.len() <= 512,
            "节点名称为空或过长"
        );
        anyhow::ensure!(names.insert(name.to_string()), "订阅内存在重名节点：{name}");
        nodes.push(ProxyNode {
            name: name.into(),
            kind: kind.into(),
            value: mapping.clone(),
        });
    }
    Ok(Profile { root, nodes })
}

pub fn load_profiles(config: &AppConfig, store: &Store) -> Result<HashMap<String, Profile>> {
    let mut result = HashMap::new();
    for subscription in &config.subscriptions {
        let body = std::fs::read_to_string(store.profile_path(&subscription.id)?)
            .with_context(|| format!("无法读取订阅缓存：{}", subscription.name))?;
        result.insert(
            subscription.id.clone(),
            parse_profile(&body).with_context(|| format!("订阅缓存无效：{}", subscription.name))?,
        );
    }
    Ok(result)
}

pub fn wire_name(id: &str, name: &str) -> String {
    format!("[{id}] {name}")
}

pub fn node_views(
    config: &AppConfig,
    profiles: &HashMap<String, Profile>,
) -> Result<Vec<NodeView>> {
    let mut result = Vec::new();
    for subscription in &config.subscriptions {
        let profile = profiles.get(&subscription.id).context("缺少订阅缓存")?;
        for node in &profile.nodes {
            result.push(NodeView {
                candidate: Candidate {
                    key: wire_name(&subscription.id, &node.name),
                    subscription_id: subscription.id.clone(),
                    subscription_name: subscription.name.clone(),
                    name: node.name.clone(),
                },
                kind: node.kind.clone(),
                in_pool: subscription.pool_nodes.contains(&node.name),
            });
        }
    }
    Ok(result)
}

pub fn candidates(
    config: &AppConfig,
    profiles: &HashMap<String, Profile>,
) -> Result<Vec<Candidate>> {
    let views = node_views(config, profiles)?;
    let by_key: HashMap<_, _> = views
        .iter()
        .map(|view| (view.candidate.key.clone(), view))
        .collect();
    let mut result = Vec::new();
    for subscription in config.pool_subscriptions() {
        let mut included = HashSet::new();
        for name in &subscription.pool_nodes {
            let key = wire_name(&subscription.id, name);
            if let Some(view) = by_key.get(&key)
                && included.insert(key)
            {
                result.push(view.candidate.clone());
            }
        }
    }
    Ok(result)
}

pub struct DownloadedSubscription {
    pub body: String,
    pub usage: Option<SubscriptionUsage>,
}

fn parse_usage(header: &str) -> Option<SubscriptionUsage> {
    let mut usage = SubscriptionUsage::default();
    for field in header.split(';') {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        let value = value.trim().parse::<u64>().ok();
        match key.trim().to_ascii_lowercase().as_str() {
            "upload" => usage.upload = value,
            "download" => usage.download = value,
            "total" => usage.total = value,
            _ => {}
        }
    }
    (usage != SubscriptionUsage::default()).then_some(usage)
}

pub async fn download(url: &str) -> Result<DownloadedSubscription> {
    let url = validate_url(url)?;
    let client = reqwest::Client::builder()
        .user_agent("clash-verge/2.5.9 ClashFx/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("订阅下载失败，请检查地址和网络"))?;
    anyhow::ensure!(
        response.status().is_success(),
        "订阅服务器返回 HTTP {}",
        response.status().as_u16()
    );
    anyhow::ensure!(
        response.content_length().unwrap_or(0) <= 20 * 1024 * 1024,
        "订阅文件超过 20 MB"
    );
    let usage = response
        .headers()
        .get("subscription-userinfo")
        .and_then(|header| header.to_str().ok())
        .and_then(parse_usage);
    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| anyhow::anyhow!("订阅下载中断"))?;
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 20 * 1024 * 1024,
            "订阅文件超过 20 MB"
        );
        bytes.extend_from_slice(&chunk);
    }
    let body = String::from_utf8(bytes).context("订阅不是 UTF-8 YAML 文件")?;
    parse_profile(&body)?;
    Ok(DownloadedSubscription { body, usage })
}

pub fn validate_url(url: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(url.trim()).map_err(|_| anyhow::anyhow!("订阅地址格式无效"))?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https"),
        "订阅地址须使用 HTTP(S)"
    );
    anyhow::ensure!(url.host_str().is_some(), "订阅地址缺少服务器名称");
    Ok(url)
}

pub fn new_subscription(name: String, url: Option<String>, _profile: &Profile) -> Subscription {
    Subscription {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        notes: String::new(),
        url,
        source_path: None,
        pool_nodes: Vec::new(),
        usage: None,
        updated_at: chrono::Utc::now(),
    }
}

pub fn build_runtime(config: &AppConfig, profiles: &HashMap<String, Profile>) -> Result<String> {
    build_runtime_for(config, profiles, None)
}

pub fn rule_sources(
    config: &AppConfig,
    profiles: &HashMap<String, Profile>,
) -> Vec<(String, usize)> {
    config
        .subscriptions
        .iter()
        .filter_map(|sub| {
            let count = profiles
                .get(&sub.id)?
                .root
                .get("rules")?
                .as_sequence()?
                .len();
            (count > 0).then(|| (sub.id.clone(), count))
        })
        .collect()
}

pub fn rule_source(config: &AppConfig, profiles: &HashMap<String, Profile>) -> Option<String> {
    let sources = rule_sources(config, profiles);
    config
        .rule_subscription
        .as_ref()
        .filter(|id| sources.iter().any(|(source, _)| source == *id))
        .cloned()
        .or_else(|| sources.first().map(|(id, _)| id.clone()))
}

pub fn build_runtime_for(
    config: &AppConfig,
    profiles: &HashMap<String, Profile>,
    manual: Option<&Candidate>,
) -> Result<String> {
    config.validate()?;
    let mut selectable = candidates(config, profiles)?;
    anyhow::ensure!(
        !selectable.is_empty() || manual.is_some(),
        "请在代理节点中选择节点，或向自动切换池添加节点"
    );
    let available: Vec<_> = node_views(config, profiles)?
        .into_iter()
        .map(|node| node.candidate)
        .collect();
    if let Some(manual) = manual {
        anyhow::ensure!(
            available.iter().any(|node| node.key == manual.key),
            "节点已失效，请更新订阅"
        );
        selectable.retain(|node| node.key != manual.key);
        selectable.insert(0, manual.clone());
    }
    for node in available {
        if !selectable.iter().any(|existing| existing.key == node.key) {
            selectable.push(node);
        }
    }
    let mut root = Mapping::new();
    let source = rule_source(config, profiles);
    let primary = source.as_ref().and_then(|id| profiles.get(id));
    if let Some(profile) = primary {
        for key in [
            "dns",
            "hosts",
            "sniffer",
            "geodata-mode",
            "geodata-loader",
            "geo-auto-update",
            "geo-update-interval",
            "geox-url",
            "global-client-fingerprint",
            "global-ua",
            "find-process-mode",
        ] {
            if let Some(value) = profile.root.get(key) {
                root.insert(key.into(), value.clone());
            }
        }
        if let Some(value) = profile.root.get("rule-providers") {
            let mut providers = value.as_mapping().context("规则集格式无效")?.clone();
            for (index, value) in providers.values_mut().enumerate() {
                if let Some(provider) = value.as_mapping_mut() {
                    provider.insert(
                        "path".into(),
                        format!("./rule-providers/{index}.yaml").into(),
                    );
                    if provider.contains_key("proxy") {
                        provider.insert("proxy".into(), AUTO_GROUP.into());
                    }
                }
            }
            root.insert("rule-providers".into(), Value::Mapping(providers));
        }
        if let Some(value) = profile.root.get("sub-rules") {
            let mut groups = value.as_mapping().context("子规则格式无效")?.clone();
            for value in groups.values_mut() {
                *value = rewrite_rules(value)?;
            }
            root.insert("sub-rules".into(), Value::Mapping(groups));
        }
    }
    root.insert("mixed-port".into(), config.core.mixed_port.into());
    root.insert(
        "external-controller".into(),
        format!("127.0.0.1:{}", config.core.controller_port).into(),
    );
    root.insert("secret".into(), config.core.secret.clone().into());
    root.insert("allow-lan".into(), false.into());
    root.insert("bind-address".into(), "127.0.0.1".into());
    root.insert("mode".into(), config.routing_mode.as_str().into());
    root.insert("log-level".into(), "info".into());
    root.insert("ipv6".into(), true.into());
    root.insert(
        "profile".into(),
        serde_yaml_ng::from_str::<Value>("store-selected: true")?,
    );
    let mut proxies = Vec::new();
    let mut groups = Vec::new();
    for subscription in &config.subscriptions {
        let profile = profiles.get(&subscription.id).context("缺少订阅缓存")?;
        let mut aliases: HashMap<String, String> = profile
            .nodes
            .iter()
            .map(|node| (node.name.clone(), wire_name(&subscription.id, &node.name)))
            .collect();
        if let Some(source_groups) = profile
            .root
            .get("proxy-groups")
            .and_then(Value::as_sequence)
        {
            for group in source_groups {
                if let Some(name) = group.get("name").and_then(Value::as_str) {
                    aliases.insert(name.to_string(), wire_name(&subscription.id, name));
                }
            }
            for source_group in source_groups {
                let Some(group) = source_group.as_mapping() else {
                    continue;
                };
                if group.contains_key("use")
                    || group.get("include-all-providers").and_then(Value::as_bool) == Some(true)
                {
                    continue;
                }
                let mut group = group.clone();
                if let Some(name) = group.get("name").and_then(Value::as_str) {
                    group.insert("name".into(), wire_name(&subscription.id, name).into());
                }
                if let Some(nodes) = group.get_mut("proxies").and_then(Value::as_sequence_mut) {
                    for name in nodes {
                        if let Some(alias) = name.as_str().and_then(|name| aliases.get(name)) {
                            *name = alias.clone().into();
                        }
                    }
                }
                groups.push(Value::Mapping(group));
            }
        }
        for node in &profile.nodes {
            let mut value = node.value.clone();
            value.insert(
                "name".into(),
                wire_name(&subscription.id, &node.name).into(),
            );
            if let Some(alias) = value
                .get("dialer-proxy")
                .and_then(Value::as_str)
                .and_then(|name| aliases.get(name))
            {
                value.insert("dialer-proxy".into(), alias.clone().into());
            }
            proxies.push(Value::Mapping(value));
        }
    }
    // Rules use our selector; imported groups are only needed by proxy chains.
    let mut required: HashSet<String> = proxies
        .iter()
        .filter_map(|node| {
            node.get("dialer-proxy")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    loop {
        let previous = required.len();
        for group in &groups {
            if group
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| required.contains(name))
                && let Some(nodes) = group.get("proxies").and_then(Value::as_sequence)
            {
                required.extend(nodes.iter().filter_map(Value::as_str).map(str::to_owned));
            }
        }
        if required.len() == previous {
            break;
        }
    }
    groups.retain(|group| {
        group
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| required.contains(name))
    });
    let mut selector = Mapping::new();
    selector.insert("name".into(), AUTO_GROUP.into());
    selector.insert("type".into(), "select".into());
    selector.insert(
        "proxies".into(),
        Value::Sequence(
            selectable
                .iter()
                .map(|node| node.key.clone().into())
                .collect(),
        ),
    );
    groups.insert(0, Value::Mapping(selector));
    root.insert("proxies".into(), Value::Sequence(proxies));
    root.insert("proxy-groups".into(), Value::Sequence(groups));
    let rules = match primary.and_then(|profile| profile.root.get("rules")) {
        Some(value) => rewrite_rules(value)?,
        None => Value::Sequence(
            [
                "DOMAIN,localhost,DIRECT",
                "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve",
                "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
                "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve",
                "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
                "IP-CIDR6,::1/128,DIRECT,no-resolve",
            ]
            .into_iter()
            .map(Value::from)
            .chain(std::iter::once(format!("MATCH,{AUTO_GROUP}").into()))
            .collect(),
        ),
    };
    root.insert("rules".into(), rules);
    Ok(serde_yaml_ng::to_string(&root)?)
}

fn rewrite_rules(value: &Value) -> Result<Value> {
    let rules = value.as_sequence().context("rules 必须是列表")?;
    let mut result = Vec::new();
    for rule in rules {
        let raw = rule.as_str().context("规则必须是字符串")?;
        let mut depth = 0i32;
        let mut start = 0;
        let mut parts = Vec::new();
        for (index, character) in raw.char_indices() {
            match character {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(raw[start..index].trim().to_string());
                    start = index + 1;
                }
                _ => {}
            }
        }
        parts.push(raw[start..].trim().to_string());
        if parts.first().is_some_and(|kind| kind != "SUB-RULE") && parts.len() >= 2 {
            let target = parts.len()
                - if parts.last().is_some_and(|part| part == "no-resolve") {
                    2
                } else {
                    1
                };
            if !matches!(
                parts[target].as_str(),
                "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS"
            ) {
                parts[target] = AUTO_GROUP.into();
            }
        }
        result.push(parts.join(",").into());
    }
    Ok(Value::Sequence(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_header_keeps_missing_values_unknown_and_clamps_overage() {
        let usage =
            parse_usage(" download = 75 ; total=100; Upload=5; expire=1800000000;").unwrap();
        assert_eq!(usage.used(), Some(80));
        assert_eq!(usage.remaining(), Some(20));
        for header in [
            "",
            "expire=1800000000",
            "upload=-1; download=NaN; total=invalid",
        ] {
            assert!(parse_usage(header).is_none());
        }
        let partial = parse_usage("download=75; total=100; upload=invalid").unwrap();
        assert_eq!(partial.total, Some(100));
        assert_eq!(partial.used(), None);
        assert_eq!(partial.remaining(), None);
        assert_eq!(
            parse_usage("upload=0; download=0").unwrap().remaining(),
            None
        );
        for header in [
            "upload=5; download=100; total=100",
            "upload=0; download=0; total=0",
            "upload=18446744073709551615; download=1; total=100",
        ] {
            assert_eq!(parse_usage(header).unwrap().remaining(), Some(0));
        }
    }

    #[test]
    fn runtime_omits_unused_health_groups_and_keeps_nested_proxy_chains() {
        let profile = parse_profile("proxies:\n - {name: Entry, type: http, server: localhost, port: 8080}\n - {name: Exit, type: http, server: localhost, port: 8081, dialer-proxy: Chain}\nproxy-groups:\n - {name: Unused, type: url-test, proxies: [Entry], url: 'http://example.test/check', interval: 10}\n - {name: Chain, type: select, proxies: [Nested]}\n - {name: Nested, type: select, proxies: [Entry]}\nrules:\n - MATCH,Unused\n").unwrap();
        let mut sub = new_subscription("Fixture".into(), None, &profile);
        sub.pool_nodes.push("Exit".into());
        let id = sub.id.clone();
        let config = AppConfig {
            subscriptions: vec![sub],
            ..Default::default()
        };
        let runtime: Value = serde_yaml_ng::from_str(
            &build_runtime_for(&config, &HashMap::from([(id.clone(), profile)]), None).unwrap(),
        )
        .unwrap();
        let names: Vec<_> = runtime["proxy-groups"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|group| group["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                AUTO_GROUP,
                &wire_name(&id, "Chain"),
                &wire_name(&id, "Nested")
            ]
        );
        assert_eq!(
            runtime["proxies"][1]["dialer-proxy"],
            wire_name(&id, "Chain")
        );
        assert_eq!(
            runtime["proxy-groups"][1]["proxies"][0],
            wire_name(&id, "Nested")
        );
    }

    #[test]
    fn only_explicit_pool_nodes_are_candidates() {
        let profile = parse_profile("proxies:\n  - {name: Japan 01, type: http, server: localhost, port: 8080}\n  - {name: HK 01, type: http, server: localhost, port: 8081}").unwrap();
        let mut sub = new_subscription("A".into(), None, &profile);
        assert!(sub.pool_nodes.is_empty());
        sub.pool_nodes = vec!["Japan 01".into()];
        let mut config = AppConfig {
            subscriptions: vec![sub.clone()],
            ..Default::default()
        };
        let profiles = HashMap::from([(sub.id.clone(), profile)]);
        assert_eq!(candidates(&config, &profiles).unwrap().len(), 1);
        config.subscriptions[0].pool_nodes.clear();
        assert!(candidates(&config, &profiles).unwrap().is_empty());
        assert!(build_runtime(&config, &profiles).is_err());
    }

    #[test]
    fn rules_preserve_direct_and_reject_and_route_proxy_policies_to_auto() {
        let rules: Value = serde_yaml_ng::from_str("- DOMAIN-SUFFIX,cn,DIRECT\n- IP-CIDR,8.8.8.8/32,Other,no-resolve\n- AND,((NETWORK,TCP),(DST-PORT,443)),Old\n- SUB-RULE,(NETWORK,TCP),sub\n- MATCH,REJECT").unwrap();
        let result = rewrite_rules(&rules).unwrap();
        let lines = result.as_sequence().unwrap();
        assert_eq!(lines[0].as_str().unwrap(), "DOMAIN-SUFFIX,cn,DIRECT");
        assert_eq!(
            lines[1].as_str().unwrap(),
            format!("IP-CIDR,8.8.8.8/32,{AUTO_GROUP},no-resolve")
        );
        assert_eq!(
            lines[2].as_str().unwrap(),
            format!("AND,((NETWORK,TCP),(DST-PORT,443)),{AUTO_GROUP}")
        );
        assert_eq!(lines[3].as_str().unwrap(), "SUB-RULE,(NETWORK,TCP),sub");
        assert_eq!(lines[4].as_str().unwrap(), "MATCH,REJECT");
    }

    #[test]
    fn default_rules_skip_node_only_profiles_and_keep_the_selected_source() {
        let nodes = "proxies:\n - {name: JP, type: http, server: localhost, port: 8080}\n";
        let plain = parse_profile(nodes).unwrap();
        let ruled = parse_profile(&format!("{nodes}hosts: {{local.test: 127.0.0.1}}\nrules:\n - DOMAIN,local.test,DIRECT\n - MATCH,Proxy\n")).unwrap();
        let alternate = parse_profile(&format!("{nodes}rules:\n - MATCH,REJECT\n")).unwrap();
        let subs = vec![
            new_subscription("Nodes".into(), None, &plain),
            new_subscription("Rules".into(), None, &ruled),
            new_subscription("Alternate".into(), None, &alternate),
        ];
        let profiles = HashMap::from([
            (subs[0].id.clone(), plain),
            (subs[1].id.clone(), ruled),
            (subs[2].id.clone(), alternate),
        ]);
        let mut config = AppConfig {
            subscriptions: subs,
            ..Default::default()
        };
        let manual = node_views(&config, &profiles).unwrap()[0].candidate.clone();
        let runtime: Value =
            serde_yaml_ng::from_str(&build_runtime_for(&config, &profiles, Some(&manual)).unwrap())
                .unwrap();
        assert_eq!(runtime["mode"].as_str(), Some("rule"));
        assert_eq!(runtime["hosts"]["local.test"].as_str(), Some("127.0.0.1"));
        assert_eq!(
            runtime["rules"][0].as_str(),
            Some("DOMAIN,local.test,DIRECT")
        );
        assert_eq!(
            rule_source(&config, &profiles),
            Some(config.subscriptions[1].id.clone())
        );
        config.rule_subscription = Some(config.subscriptions[2].id.clone());
        config.subscriptions.swap(0, 2);
        let runtime: Value =
            serde_yaml_ng::from_str(&build_runtime_for(&config, &profiles, Some(&manual)).unwrap())
                .unwrap();
        assert_eq!(runtime["rules"][0].as_str(), Some("MATCH,REJECT"));
    }
}
