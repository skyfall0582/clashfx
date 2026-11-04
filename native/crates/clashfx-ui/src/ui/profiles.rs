use super::*;
use subscriptions::source_preview;

fn traffic_size(bytes: u64) -> String {
    let mut value = bytes as f64;
    let units = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
    let mut unit = 0;
    while value >= 1024. && unit < units.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", units[unit])
    }
}

fn subscription_usage(sub: &clashfx_core::model::Subscription) -> AnyElement {
    let row = div()
        .id(SharedString::from(format!("profile-usage-{}", sub.id)))
        .flex()
        .flex_col()
        .gap_1()
        .text_size(px(12.));
    let Some(usage) = &sub.usage else {
        return row
            .text_color(rgb(MUTED))
            .child("暂无流量信息")
            .tooltip(|_, cx| {
                cx.new(|_| controls::Tip("更新订阅后获取，需服务商提供流量信息".into()))
                    .into()
            })
            .into_any_element();
    };
    let remaining = usage.remaining();
    let fraction = remaining.zip(usage.total).and_then(|(remaining, total)| {
        (total > 0).then(|| (remaining as f64 / total as f64) as f32)
    });
    let color = if remaining == Some(0) || fraction.is_some_and(|value| value <= 0.1) {
        0xb64c4a
    } else {
        ACCENT
    };
    let tip = usage.used().map_or_else(
        || "用量未提供 · 随订阅更新刷新".to_owned(),
        |used| format!("已用 {} · 随订阅更新刷新", traffic_size(used)),
    );
    row.child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .text_color(rgb(color))
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!(
                        "剩余 {}",
                        remaining.map(traffic_size).unwrap_or_else(|| "—".into())
                    )),
            )
            .child(div().text_color(rgb(MUTED)).child(format!(
                "总量 {}",
                usage.total.map(traffic_size).unwrap_or_else(|| "—".into())
            ))),
    )
    .when_some(fraction, |row, fraction| {
        row.child(
            div()
                .h(px(4.))
                .w_full()
                .rounded_full()
                .overflow_hidden()
                .bg(rgb(LINE))
                .child(div().h_full().w(relative(fraction)).bg(rgb(color))),
        )
    })
    .tooltip(move |_, cx| cx.new(|_| controls::Tip(tip.clone())).into())
    .into_any_element()
}

impl ClashFx {
    pub(super) fn subscription_management(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.text("subscription_search", cx).to_lowercase();
        let subscriptions = self.snapshot.config.subscriptions.clone();
        let mut cards = div().grid().grid_cols(2).gap_2();
        let mut count = 0;
        for sub in subscriptions {
            let source = sub
                .url
                .as_deref()
                .or(sub.source_path.as_deref())
                .unwrap_or("本地 YAML 文件");
            if !query.is_empty()
                && !format!("{} {source} {}", sub.name, sub.notes)
                    .to_lowercase()
                    .contains(&query)
            {
                continue;
            }
            count += 1;
            let total = self
                .snapshot
                .nodes
                .iter()
                .filter(|node| node.candidate.subscription_id == sub.id)
                .count();
            let (nodes_link, nodes_disabled) = self.control(
                format!("profile-nodes-{}", sub.id),
                false,
                Action::ShowNodes(sub.id.clone()),
                cx,
            );
            let card = panel()
                .p_3()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_2()
                                .rounded_md()
                                .child(
                                    svg()
                                        .path("icons/file.svg")
                                        .size(px(18.))
                                        .flex_shrink_0()
                                        .text_color(rgb(ACCENT)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(sub.name.clone()),
                                ),
                        )
                        .child(self.icon_button(
                            format!("profile-edit-{}", sub.id),
                            "icons/edit.svg",
                            "编辑订阅",
                            false,
                            Action::EditSubscription(sub.id.clone()),
                            cx,
                        )),
                )
                .when(!sub.notes.trim().is_empty(), |card| {
                    card.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .line_height(px(18.))
                            .line_clamp(1)
                            .child(sub.notes.clone()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(px(36.))
                        .px_2()
                        .py_1()
                        .rounded_lg()
                        .bg(rgb(0xf8f7f5))
                        .child(
                            div()
                                .id(SharedString::from(format!("profile-source-{}", sub.id)))
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(rgb(0x807369))
                                .child(source_preview(source)),
                        )
                        .when_some(sub.url.clone(), |row, url| {
                            row.child(self.icon_button(
                                format!("profile-copy-{}", sub.id),
                                "icons/copy.svg",
                                "复制订阅链接",
                                false,
                                Action::CopySubscriptionUrl(url),
                                cx,
                            ))
                        }),
                )
                .when(sub.url.is_some(), |card| {
                    card.child(subscription_usage(&sub))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            hint(
                                &sub.updated_at
                                    .with_timezone(&chrono::Local)
                                    .format("%m-%d %H:%M")
                                    .to_string(),
                            )
                            .flex_shrink_0()
                            .whitespace_nowrap(),
                        )
                        .child(self.icon_button(
                            format!("profile-update-{}", sub.id),
                            "icons/refresh.svg",
                            if sub.url.is_some() {
                                "更新订阅"
                            } else {
                                "替换订阅文件"
                            },
                            false,
                            if sub.url.is_some() {
                                Action::Command(Command::UpdateSubscription(sub.id.clone()))
                            } else {
                                Action::ReplaceSubscriptionFile(sub.id.clone())
                            },
                            cx,
                        ))
                        .child(div().flex_1())
                        .child(
                            nodes_link
                                .h(px(30.))
                                .px_2()
                                .rounded_md()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(ACCENT))
                                .tooltip(|_, cx| {
                                    cx.new(|_| controls::Tip("查看节点".into())).into()
                                })
                                .when(!nodes_disabled, |link| {
                                    link.hover(|style| style.text_color(rgb(ACCENT_HOVER)))
                                })
                                .child(
                                    svg()
                                        .path("icons/nodes.svg")
                                        .size(px(16.))
                                        .flex_shrink_0()
                                        .text_color(rgb(ACCENT)),
                                )
                                .child(total.to_string())
                                .child(
                                    svg()
                                        .path("icons/arrow-right.svg")
                                        .size(px(14.))
                                        .flex_shrink_0()
                                        .text_color(rgb(ACCENT)),
                                ),
                        ),
                );
            cards = cards.child(card);
        }
        div()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(360.))
                            .child(self.inputs["subscription_search"].clone()),
                    )
                    .child(div().flex_1())
                    .child(self.button(
                        "add-profile",
                        "导入订阅",
                        true,
                        false,
                        Action::ToggleImport,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("profile-cards")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(count == 0, |body| {
                        body.child(empty(
                            "icons/subscriptions.svg",
                            if self.snapshot.config.subscriptions.is_empty() {
                                "还没有订阅"
                            } else {
                                "没有匹配的订阅"
                            },
                            "",
                        ))
                    })
                    .child(cards),
            )
            .into_any_element()
    }

    pub(super) fn policy(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut tabs = div().flex().gap_4().border_b_1().border_color(rgb(LINE));
        for (index, (label, candidates)) in [("切换池", true), ("检测规则", false)]
            .into_iter()
            .enumerate()
        {
            let selected = self.policy_candidates == candidates;
            let (tab, _) = self.control(
                format!("policy-tab-{index}"),
                false,
                Action::SetPolicyTab(candidates),
                cx,
            );
            tabs = tabs.child(
                tab.h(px(32.))
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_0()
                    .border_b_2()
                    .border_color(if selected {
                        rgb(ACCENT).into()
                    } else {
                        transparent_black()
                    })
                    .text_size(px(13.))
                    .text_color(rgb(if selected { ACCENT } else { MUTED }))
                    .when(selected, |tab| tab.font_weight(FontWeight::SEMIBOLD))
                    .hover(|style| style.text_color(rgb(ACCENT)))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(
                            event.keystroke.key.as_str(),
                            "left" | "right" | "home" | "end"
                        ) {
                            let next = match event.keystroke.key.as_str() {
                                "home" => true,
                                "end" => false,
                                _ => !candidates,
                            };
                            this.keyboard_focus = true;
                            this.handle(Action::SetPolicyTab(next), window, cx);
                            if let Some(focus) = this
                                .button_focus
                                .get(&format!("policy-tab-{}", usize::from(!next)))
                            {
                                focus.focus(window);
                            }
                            cx.stop_propagation();
                        }
                    }))
                    .child(label),
            );
        }
        let content = if self.policy_candidates {
            self.pool_editor(cx)
        } else {
            div()
                .id("policy-rules-body")
                .h_full()
                .overflow_y_scroll()
                .child(self.policy_rules(cx))
                .into_any_element()
        };
        div()
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().flex_shrink_0().child(tabs))
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}
