use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};
use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpStream},
    task::JoinSet,
};

const VARIABLES: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

pub struct ProxyCompatibility {
    tasks: JoinSet<()>,
}

impl ProxyCompatibility {
    pub fn start(port: u16) -> Result<Self> {
        let mut values: Vec<_> = VARIABLES
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .collect();
        #[cfg(windows)]
        {
            use winreg::{RegKey, enums::*};
            for (root, path) in [
                (HKEY_CURRENT_USER, "Environment"),
                (
                    HKEY_LOCAL_MACHINE,
                    "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
                ),
            ] {
                if let Ok(key) = RegKey::predef(root).open_subkey(path) {
                    values.extend(
                        VARIABLES
                            .iter()
                            .filter_map(|name| key.get_value::<String, _>(name).ok()),
                    );
                }
            }
        }
        Self::bind(addresses(values), port)
    }

    fn bind(addresses: impl IntoIterator<Item = SocketAddr>, port: u16) -> Result<Self> {
        let target = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let listeners: Vec<_> = addresses
            .into_iter()
            .filter(|address| *address != target)
            .map(|address| {
                let listener = std::net::TcpListener::bind(address).with_context(|| {
                    format!("本地代理 {address} 无法接管，请退出其他代理软件后重试")
                })?;
                listener.set_nonblocking(true)?;
                Ok(TcpListener::from_std(listener)?)
            })
            .collect::<Result<_>>()?;
        let mut tasks = JoinSet::new();
        for listener in listeners {
            tasks.spawn(async move {
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        connection = listener.accept() => {
                            let Ok((mut incoming, _)) = connection else { break };
                            connections.spawn(async move {
                                if let Ok(mut upstream) = TcpStream::connect(target).await {
                                    copy_bidirectional(&mut incoming, &mut upstream).await.ok();
                                }
                            });
                        }
                        _ = connections.join_next(), if !connections.is_empty() => {}
                    }
                }
            });
        }
        Ok(Self { tasks })
    }

    pub async fn stop(mut self) {
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
    }
}

fn addresses(values: impl IntoIterator<Item = String>) -> BTreeSet<SocketAddr> {
    let mut addresses = BTreeSet::new();
    for value in values {
        let value = value.trim();
        let value = if value.contains("://") {
            value.to_owned()
        } else {
            format!("http://{value}")
        };
        let Ok(url) = reqwest::Url::parse(&value) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "socks5" | "socks5h") {
            continue;
        }
        let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
            continue;
        };
        if port == 0 {
            continue;
        }
        if host.eq_ignore_ascii_case("localhost") {
            addresses.insert(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
            addresses.insert(SocketAddr::from((Ipv6Addr::LOCALHOST, port)));
        } else if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>()
            && ip.is_loopback()
        {
            addresses.insert(SocketAddr::new(ip, port));
        }
    }
    addresses
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::{accept_async, client_async, tungstenite::Message};

    #[test]
    fn only_local_http_and_socks_proxy_endpoints_are_claimed() {
        let actual = addresses(
            [
                "http://127.0.0.1:7890",
                "127.0.0.1:7890",
                "socks5://localhost:1080",
                "socks5h://[::1]:1080",
                "https://127.0.0.1:7891",
                "http://192.168.1.1:8080",
                "http://proxy.example.com:8080",
            ]
            .map(str::to_owned),
        );
        assert_eq!(
            actual,
            BTreeSet::from([
                "127.0.0.1:7890".parse().unwrap(),
                "127.0.0.1:1080".parse().unwrap(),
                "[::1]:1080".parse().unwrap(),
            ])
        );
    }

    #[tokio::test]
    async fn inherited_proxy_port_carries_connect_and_websocket_streams() -> Result<()> {
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = upstream.local_addr()?.port();
        let available = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = available.local_addr()?;
        drop(available);
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await?;
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(socket.read_u8().await?);
            }
            assert!(header.starts_with(b"CONNECT codex.test:443 HTTP/1.1"));
            socket
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
            let mut websocket = accept_async(socket).await?;
            while let Some(message) = websocket.next().await {
                let message = message?;
                if message.is_close() {
                    break;
                }
                websocket.send(message).await?;
            }
            Ok::<(), anyhow::Error>(())
        });
        let compatibility = ProxyCompatibility::bind([address], port)?;
        let mut socket = TcpStream::connect(address).await?;
        socket
            .write_all(b"CONNECT codex.test:443 HTTP/1.1\r\nHost: codex.test:443\r\n\r\n")
            .await?;
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(socket.read_u8().await?);
        }
        assert!(header.starts_with(b"HTTP/1.1 200"));
        let (mut websocket, _) = client_async("ws://codex.test/responses", socket).await?;
        for text in [
            "response.created",
            "response.output_text.delta",
            "response.completed",
        ] {
            websocket.send(Message::Text(text.into())).await?;
            assert_eq!(websocket.next().await.unwrap()?, Message::Text(text.into()));
        }
        websocket.close(None).await?;
        server.await??;
        compatibility.stop().await;
        let _released = std::net::TcpListener::bind(address)?;
        Ok(())
    }

    #[tokio::test]
    async fn occupied_proxy_port_rolls_back_other_listeners() -> Result<()> {
        let first = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let second = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let free = first.local_addr()?;
        let occupied = second.local_addr()?;
        drop(first);
        let error = ProxyCompatibility::bind([free, occupied], 17897)
            .err()
            .expect("an occupied inherited proxy port must prevent activation");
        assert!(error.to_string().contains(&occupied.to_string()));
        let _released = std::net::TcpListener::bind(free)?;
        assert!(TcpStream::connect(occupied).await.is_ok());
        Ok(())
    }
}
