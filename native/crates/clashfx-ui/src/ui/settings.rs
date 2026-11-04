use super::*;

impl ClashFx {
    pub(super) fn settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.snapshot.clone();
        let modes = [RoutingMode::Rule, RoutingMode::Global, RoutingMode::Direct];
        let selected = modes
            .iter()
            .position(|mode| *mode == snapshot.config.routing_mode)
            .unwrap_or(0);
        let choices = modes
            .into_iter()
            .map(|mode| {
                (
                    mode.label().into(),
                    Action::Command(Command::SetRoutingMode(mode)),
                )
            })
            .collect();
        let mut routing = panel()
            .flex()
            .flex_col()
            .gap_3()
            .child(section_title("分流方式"))
            .child(div().max_w(px(480.)).child(self.segmented(
                "routing-mode",
                choices,
                selected,
                cx,
            )));
        if snapshot.config.routing_mode == RoutingMode::Rule {
            routing = routing.child(div().pt_2().child(hint("规则来源")));
            if snapshot.rule_sources.is_empty() {
                routing = routing.child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(0xa16c19))
                        .child("订阅未提供规则：本地网络直连，其余流量使用代理。"),
                );
            }
            for (id, count) in &snapshot.rule_sources {
                let Some(sub) = snapshot
                    .config
                    .subscriptions
                    .iter()
                    .find(|sub| &sub.id == id)
                else {
                    continue;
                };
                let selected = snapshot.effective_rule_subscription.as_ref() == Some(id);
                let (row, _) = self.control(
                    format!("rule-source-{id}"),
                    false,
                    Action::Command(Command::SetRuleSubscription(id.clone())),
                    cx,
                );
                routing = routing.child(
                    row.rounded_md()
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_3()
                        .hover(|style| style.bg(rgb(0xf4efea)))
                        .child(
                            div()
                                .size(px(16.))
                                .flex_shrink_0()
                                .rounded_full()
                                .border_1()
                                .border_color(rgb(if selected { ACCENT } else { 0xbeb3a8 }))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(selected, |radio| {
                                    radio.child(div().size(px(8.)).rounded_full().bg(rgb(ACCENT)))
                                }),
                        )
                        .child(div().flex_1().min_w_0().child(sub.name.clone()))
                        .child(hint(&format!("{count} 条规则"))),
                );
            }
        } else {
            routing = routing.child(hint(
                if snapshot.config.routing_mode == RoutingMode::Global {
                    "所有流量使用当前代理节点。"
                } else {
                    "所有流量直接连接，不使用代理节点。"
                },
            ));
        }
        let startup = panel()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(section_title("开机自启"))
                    .child(hint("登录 Windows 后自动启动 ClashFx")),
            )
            .child(self.toggle(
                "launch-on-startup",
                snapshot.launch_on_startup,
                false,
                Action::Command(Command::SetLaunchOnStartup(!snapshot.launch_on_startup)),
                cx,
            ));
        let data = panel()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(section_title("配置与缓存"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(snapshot.data_dir.clone()),
                    ),
            )
            .child(self.button(
                "open-data",
                "打开目录",
                false,
                false,
                Action::OpenDataDir,
                cx,
            ));
        let (github, _) = self.control(
            "settings-github",
            false,
            Action::OpenUrl("https://github.com/skyfall0582"),
            cx,
        );
        let footer = div()
            .py_2()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(hint(&format!(
                "ClashFx {} · GPL-3.0-only · 基于 Clash Verge Rev",
                env!("CARGO_PKG_VERSION")
            )))
            .child(
                github
                    .h(px(24.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(rgb(ACCENT))
                    .underline()
                    .hover(|style| style.text_color(rgb(ACCENT_HOVER)))
                    .child("https://github.com/skyfall0582"),
            )
            .child(hint("QQ：2834463359"));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(routing)
            .child(startup)
            .child(data)
            .child(footer)
            .into_any_element()
    }
}
