use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Health {
    Unknown,
    Checking,
    Available,
    Failed,
}

impl ClashFx {
    fn health(&self, key: &str) -> Health {
        if self.snapshot.checking_nodes.contains(key) {
            return Health::Checking;
        }
        let ttl = self
            .snapshot
            .config
            .policy
            .check_interval_secs
            .saturating_mul(2)
            .max(30) as i64;
        match self.snapshot.probes.get(key) {
            Some(probe)
                if self.snapshot.running
                    && probe
                        .checked_at
                        .is_some_and(|at| (chrono::Utc::now() - at).num_seconds() <= ttl) =>
            {
                if probe.delay_ms.is_some() {
                    Health::Available
                } else {
                    Health::Failed
                }
            }
            _ => Health::Unknown,
        }
    }

    pub(super) fn node_health(&self, key: &str) -> AnyElement {
        let probe = self.snapshot.probes.get(key);
        let (icon, label, color) = match self.health(key) {
            Health::Checking => ("icons/refresh.svg", "检测中".into(), ACCENT),
            Health::Available => (
                "icons/check.svg",
                format!("{} ms", probe.and_then(|probe| probe.delay_ms).unwrap_or(0)),
                GREEN,
            ),
            Health::Failed => ("icons/close.svg", "不可用".into(), 0xb64c4a),
            Health::Unknown => (
                "icons/clock.svg",
                if probe.is_some() {
                    "待复测"
                } else {
                    "待检测"
                }
                .into(),
                MUTED,
            ),
        };
        let detail = probe
            .and_then(|probe| {
                probe.checked_at.map(|at| {
                    format!(
                        "{}{}",
                        at.with_timezone(&chrono::Local).format("%H:%M:%S 检测"),
                        probe
                            .error
                            .as_ref()
                            .map(|error| format!(" · {error}"))
                            .unwrap_or_default()
                    )
                })
            })
            .unwrap_or_else(|| "尚未完成连通性检测".into());
        div()
            .id(SharedString::from(format!("health-{key}")))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(11.))
            .text_color(rgb(color))
            .tooltip(move |_, cx| cx.new(|_| controls::Tip(detail.clone())).into())
            .child(svg().path(icon).size(px(14.)).text_color(rgb(color)))
            .child(label)
            .into_any_element()
    }

    pub(super) fn pool_status(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.snapshot.clone();
        let total = snapshot.candidates.len();
        let available = snapshot
            .candidates
            .iter()
            .filter(|node| self.health(&node.key) == Health::Available)
            .count();
        let failed = snapshot
            .candidates
            .iter()
            .filter(|node| self.health(&node.key) == Health::Failed)
            .count();
        let checking = snapshot
            .candidates
            .iter()
            .filter(|node| self.health(&node.key) == Health::Checking)
            .count();
        let unknown = total - available - failed - checking;
        let all_failed = total > 0 && failed == total;
        let mut body = panel()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .when(all_failed, |body| {
                body.border_color(rgb(0xedc7c4)).bg(rgb(0xfffafa))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(section_title("切换池"))
                    .child(div().flex_1())
                    .child(self.icon_button(
                        "manage-pool",
                        "icons/settings.svg",
                        "管理切换池",
                        false,
                        Action::Navigate(Page::Policy),
                        cx,
                    )),
            );
        if total == 0 {
            return body.child(hint("从自动切换页添加节点")).into_any_element();
        }
        let mut summary = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_4()
            .child(pill(&format!("{available} 可用"), GREEN))
            .child(pill(&format!("{failed} 不可用"), 0xb64c4a));
        if unknown > 0 {
            summary = summary.child(pill(&format!("{unknown} 待检测"), MUTED));
        }
        if checking > 0 {
            summary = summary.child(pill(&format!("{checking} 检测中"), ACCENT));
        }
        body = body.child(summary);
        if all_failed {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.))
                    .text_color(rgb(0xb64c4a))
                    .child(
                        svg()
                            .path("icons/warning.svg")
                            .size(px(16.))
                            .text_color(rgb(0xb64c4a)),
                    )
                    .child(format!(
                        "全部 {total} 个节点不可用{}",
                        snapshot
                            .retry_at
                            .map(|at| format!(
                                " · {} 重试",
                                at.with_timezone(&chrono::Local).format("%H:%M:%S")
                            ))
                            .unwrap_or_default()
                    )),
            );
        }
        if let Some(switch) = &snapshot.last_switch {
            let from = switch
                .from
                .as_ref()
                .map(|node| format!("{} · {}", node.subscription_name, node.name))
                .unwrap_or_else(|| "—".into());
            let detail = format!(
                "{} · {}",
                switch.at.with_timezone(&chrono::Local).format("%H:%M:%S"),
                switch.reason
            );
            body = body.child(
                div()
                    .id("last-switch")
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .pt_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(px(12.))
                    .tooltip(move |_, cx| cx.new(|_| controls::Tip(detail.clone())).into())
                    .child(div().flex_1().min_w_0().text_color(rgb(MUTED)).child(from))
                    .child(
                        svg()
                            .path("icons/arrow-right.svg")
                            .size(px(18.))
                            .flex_shrink_0()
                            .text_color(rgb(ACCENT)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(rgb(ACCENT))
                            .child(format!(
                                "{} · {}",
                                switch.to.subscription_name, switch.to.name
                            )),
                    ),
            );
        }
        for sub in snapshot.config.pool_subscriptions() {
            let nodes: Vec<_> = snapshot
                .candidates
                .iter()
                .filter(|node| node.subscription_id == sub.id)
                .collect();
            if nodes.is_empty() {
                continue;
            }
            let mut group = div().flex().flex_col().gap_2().child(hint(&sub.name));
            for node in nodes {
                let active = snapshot
                    .active
                    .as_ref()
                    .is_some_and(|active| active.key == node.key);
                group = group.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .py_1()
                        .child(
                            svg()
                                .path(if active {
                                    "icons/arrow-right.svg"
                                } else {
                                    "icons/nodes.svg"
                                })
                                .size(px(15.))
                                .text_color(rgb(if active { ACCENT } else { MUTED })),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(13.))
                                .text_color(rgb(if active { ACCENT } else { INK }))
                                .child(node.name.clone()),
                        )
                        .when(active, |row| row.child(hint("当前")))
                        .child(self.node_health(&node.key)),
                );
            }
            body = body.child(group);
        }
        body.into_any_element()
    }
}
