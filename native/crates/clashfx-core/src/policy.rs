use crate::model::{AUTO_GROUP, AutoPolicy, Candidate};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionEvent {
    Failure,
    Success,
    Unrelated,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CoreLog {
    #[serde(rename = "type", alias = "level", default)]
    pub level: String,
    #[serde(alias = "message", alias = "msg", default)]
    pub payload: String,
}

pub fn classify(log: &CoreLog) -> ConnectionEvent {
    let message = &log.payload;
    if !(message.contains("[TCP]") || message.contains("[UDP]")) {
        return ConnectionEvent::Unrelated;
    }
    if message.contains(&format!("dial {AUTO_GROUP} "))
        && message.to_ascii_lowercase().contains("error:")
        && matches!(log.level.as_str(), "warning" | "warn" | "error")
    {
        ConnectionEvent::Failure
    } else if message.contains(&format!("using {AUTO_GROUP}["))
        && matches!(log.level.as_str(), "info" | "debug")
    {
        ConnectionEvent::Success
    } else {
        ConnectionEvent::Unrelated
    }
}

#[derive(Default, Debug)]
pub struct FailureMonitor {
    count: u32,
    first_failure: Option<DateTime<Utc>>,
}

impl FailureMonitor {
    pub fn record(
        &mut self,
        event: ConnectionEvent,
        now: DateTime<Utc>,
        policy: &AutoPolicy,
    ) -> bool {
        match event {
            ConnectionEvent::Failure => {
                if self.first_failure.is_none_or(|first| {
                    (now - first).num_seconds() > policy.failure_window_secs as i64
                }) {
                    self.count = 0;
                    self.first_failure = Some(now);
                }
                self.count = self.count.saturating_add(1);
            }
            ConnectionEvent::Success => self.reset(),
            ConnectionEvent::Unrelated => return false,
        }
        self.count >= policy.failure_threshold
    }

    pub fn count(&self) -> u32 {
        self.count
    }
    pub fn reset(&mut self) {
        self.count = 0;
        self.first_failure = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeReason {
    Startup,
    Failures,
    Scheduled,
    Manual,
    Retry,
}

pub fn probe_order(
    candidates: &[Candidate],
    active: Option<&Candidate>,
    reason: ProbeReason,
) -> Vec<Candidate> {
    let Some(active) = active else {
        return candidates.to_vec();
    };
    if matches!(reason, ProbeReason::Startup | ProbeReason::Manual) {
        return candidates.to_vec();
    }
    if reason == ProbeReason::Scheduled {
        return candidates.to_vec();
    }
    let mut ordered = Vec::with_capacity(candidates.len());
    ordered.extend(
        candidates
            .iter()
            .filter(|node| node.key == active.key)
            .cloned(),
    );
    ordered.extend(
        candidates
            .iter()
            .filter(|node| node.subscription_id == active.subscription_id && node.key != active.key)
            .cloned(),
    );
    ordered.extend(
        candidates
            .iter()
            .filter(|node| node.subscription_id != active.subscription_id)
            .cloned(),
    );
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(level: &str, text: &str) -> CoreLog {
        CoreLog {
            level: level.into(),
            payload: text.into(),
        }
    }

    #[test]
    fn only_managed_connection_failures_trigger_failover() {
        assert_eq!(
            classify(&log(
                "warning",
                "[TCP] dial CLASHFX_AUTO (match Match/) 127.0.0.1:123 --> example.com:443 error: i/o timeout"
            )),
            ConnectionEvent::Failure
        );
        for message in [
            "[DNS] error: timeout",
            "[TCP] dial DIRECT (match GeoIP/CN) x error: refused",
            "subscription update error: timeout",
            "[UDP] DoSniff error: unsupported",
            "[TCP] dial CLASHFX_AUTO_OTHER x error: timeout",
        ] {
            assert_eq!(
                classify(&log("warning", message)),
                ConnectionEvent::Unrelated
            );
        }
        assert_eq!(
            classify(&log(
                "info",
                "[TCP] 127.0.0.1 --> example.com:443 using CLASHFX_AUTO[JP 01]"
            )),
            ConnectionEvent::Success
        );
    }

    #[test]
    fn consecutive_failures_reset_on_success_and_expire_across_idle_gaps() {
        let policy = AutoPolicy::default();
        let now = Utc::now();
        let mut monitor = FailureMonitor::default();
        assert!(!monitor.record(ConnectionEvent::Failure, now, &policy));
        assert!(!monitor.record(ConnectionEvent::Failure, now, &policy));
        monitor.record(ConnectionEvent::Unrelated, now, &policy);
        assert_eq!(monitor.count(), 2);
        monitor.record(ConnectionEvent::Success, now, &policy);
        assert_eq!(monitor.count(), 0);
        for _ in 0..2 {
            assert!(!monitor.record(ConnectionEvent::Failure, now, &policy));
        }
        assert!(monitor.record(ConnectionEvent::Failure, now, &policy));
        assert!(!monitor.record(
            ConnectionEvent::Failure,
            now + chrono::Duration::seconds(31),
            &policy
        ));
        assert_eq!(monitor.count(), 1);
    }

    #[test]
    fn failed_backup_tries_its_own_nodes_before_other_subscriptions() {
        let nodes: Vec<_> = [("a1", "a"), ("a2", "a"), ("b1", "b"), ("b2", "b")]
            .into_iter()
            .map(|(key, sub)| Candidate {
                key: key.into(),
                subscription_id: sub.into(),
                subscription_name: sub.into(),
                name: key.into(),
            })
            .collect();
        let ordered = probe_order(&nodes, Some(&nodes[2]), ProbeReason::Failures);
        assert_eq!(
            ordered
                .iter()
                .map(|node| node.key.as_str())
                .collect::<Vec<_>>(),
            ["b1", "b2", "a1", "a2"]
        );
    }
}
