use super::*;
use clashfx_core::model::Subscription as ProfileSubscription;

#[derive(Clone)]
struct PoolDrag {
    subscription: String,
    node: Option<String>,
    label: String,
}

impl Render for PoolDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(250.))
            .p_3()
            .rounded_lg()
            .bg(rgb(ACCENT_SOFT))
            .border_1()
            .border_color(rgb(ACCENT))
            .shadow_md()
            .text_color(rgb(INK))
            .text_size(px(13.))
            .child(self.label.clone())
    }
}

struct PoolTip;
impl Render for PoolTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(INK))
            .text_color(rgb(0xffffff))
            .text_size(px(12.))
            .child("拖动调整优先顺序，也可按 Alt + 上/下方向键")
    }
}

impl ClashFx {
    pub(super) fn pool_editor(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let available = self.pool_column(false, cx);
        let pool = self.pool_column(true, cx);
        div()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex_shrink_0()
                    .child(hint("只有右侧节点参与自动切换；拖动调整优先顺序。")),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap_2()
                    .child(available)
                    .child(
                        div()
                            .w(px(28.))
                            .h_full()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                svg()
                                    .path("icons/transfer.svg")
                                    .size(px(22.))
                                    .text_color(rgb(MUTED)),
                            ),
                    )
                    .child(pool),
            )
            .into_any_element()
    }

    fn pool_column(&mut self, in_pool: bool, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.snapshot.clone();
        let field = if in_pool {
            "pool_member_search"
        } else {
            "pool_available_search"
        };
        let query = self.text(field, cx).trim().to_lowercase();
        let total = if in_pool {
            snapshot
                .config
                .subscriptions
                .iter()
                .map(|sub| sub.pool_nodes.len())
                .sum()
        } else {
            snapshot.nodes.iter().filter(|node| !node.in_pool).count()
        };
        let subscriptions: Vec<_> = if in_pool {
            snapshot
                .config
                .pool_subscriptions()
                .into_iter()
                .cloned()
                .collect()
        } else {
            snapshot.config.subscriptions.clone()
        };
        let mut groups = div()
            .id(if in_pool {
                "pool-groups"
            } else {
                "available-groups"
            })
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        let mut visible_groups = 0;
        for (index, sub) in subscriptions.iter().enumerate() {
            let subscription_matches = sub.name.to_lowercase().contains(&query);
            let names: Vec<String> = if in_pool {
                sub.pool_nodes
                    .iter()
                    .filter(|name| subscription_matches || name.to_lowercase().contains(&query))
                    .cloned()
                    .collect()
            } else {
                snapshot
                    .nodes
                    .iter()
                    .filter(|node| {
                        node.candidate.subscription_id == sub.id
                            && !node.in_pool
                            && (subscription_matches
                                || node.candidate.name.to_lowercase().contains(&query))
                    })
                    .map(|node| node.candidate.name.clone())
                    .collect()
            };
            if !query.is_empty() && names.is_empty() {
                continue;
            }
            visible_groups += 1;
            groups = groups.child(self.pool_group(sub, names, index, in_pool, &query, cx));
        }
        if visible_groups == 0 {
            let (title, detail) = if !query.is_empty() {
                ("没有匹配的节点", "")
            } else if in_pool {
                ("切换池为空", "从左侧添加节点")
            } else {
                ("还没有订阅", "先在订阅页导入")
            };
            groups = groups.child(empty("icons/nodes.svg", title, detail));
        }
        panel()
            .p_2()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .overflow_hidden()
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(section_title(if in_pool {
                        "自动切换池"
                    } else {
                        "可添加的节点"
                    }))
                    .child(div().flex_1())
                    .child(controls::node_count(total)),
            )
            .when(in_pool && !snapshot.config.policy.enabled, |column| {
                column.child(network::status_icon(
                    "pool-paused",
                    "icons/pause.svg",
                    "自动切换已暂停",
                    0xb47c26,
                ))
            })
            .child(div().flex_shrink_0().child(self.inputs[field].clone()))
            .child(groups)
            .into_any_element()
    }

    fn pool_group(
        &mut self,
        sub: &ProfileSubscription,
        names: Vec<String>,
        index: usize,
        in_pool: bool,
        query: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.pending || self.snapshot.busy;
        let expanded = !query.is_empty()
            || if in_pool {
                !self.collapsed_pool.contains(&sub.id)
            } else {
                self.expanded_available.contains(&sub.id)
            };
        let side = if in_pool { "pool" } else { "available" };
        let (heading, _) = self.control(
            format!("{side}-group-toggle-{}", sub.id),
            false,
            Action::TogglePoolGroup {
                id: sub.id.clone(),
                pool: in_pool,
            },
            cx,
        );
        let mut header = div()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_2()
            .bg(rgb(if in_pool { ACCENT_SOFT } else { 0xf8f7f5 }));
        if in_pool {
            header = header.child(self.pool_drag_handle(sub, None, index, cx));
        }
        header = header
            .child(
                heading
                    .flex_1()
                    .min_w_0()
                    .rounded_md()
                    .p_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        svg()
                            .path(if expanded {
                                "icons/chevron-down.svg"
                            } else {
                                "icons/chevron-right.svg"
                            })
                            .size(px(15.))
                            .flex_shrink_0()
                            .text_color(rgb(MUTED)),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(sub.name.clone()),
                        ),
                    )
                    .child(controls::node_count(names.len())),
            )
            .child(self.pool_transfer(
                format!("{side}-group-transfer-{}", sub.id),
                if names.is_empty() {
                    "已全部加入"
                } else if in_pool {
                    if query.is_empty() {
                        "全部移出"
                    } else {
                        "移出匹配"
                    }
                } else if query.is_empty() {
                    "全部添加"
                } else {
                    "添加匹配"
                },
                names.is_empty(),
                Command::SetPoolNodes {
                    id: sub.id.clone(),
                    names: names.clone(),
                    in_pool: !in_pool,
                },
                cx,
            ));
        let mut group = div()
            .id(SharedString::from(format!("{side}-group-{}", sub.id)))
            .flex_shrink_0()
            .w_full()
            .rounded_lg()
            .border_1()
            .border_color(rgb(LINE))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(header);
        if in_pool && !busy {
            group = group
                .drag_over::<PoolDrag>(|style, drag, _, _| {
                    if drag.node.is_none() {
                        style.border_color(rgb(ACCENT))
                    } else {
                        style
                    }
                })
                .on_drop(cx.listener(move |this, drag: &PoolDrag, _, cx| {
                    if drag.node.is_none() {
                        this.submit(
                            Command::MovePoolSubscriptionTo {
                                id: drag.subscription.clone(),
                                index,
                            },
                            "订阅优先顺序已保存",
                            cx,
                        );
                        cx.stop_propagation();
                    }
                }));
        }
        if expanded {
            for name in names {
                let key = clashfx_core::subscription::wire_name(&sub.id, &name);
                let exists = self
                    .snapshot
                    .nodes
                    .iter()
                    .any(|node| node.candidate.key == key);
                let node_index = sub
                    .pool_nodes
                    .iter()
                    .position(|entry| entry == &name)
                    .unwrap_or(0);
                let mut row = div()
                    .id(SharedString::from(format!("{side}-node-{key}")))
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(LINE));
                if in_pool {
                    row = row.child(self.pool_drag_handle(sub, Some(name.clone()), node_index, cx));
                    if !busy {
                        let target = sub.id.clone();
                        let highlight = target.clone();
                        row = row
                            .drag_over::<PoolDrag>(move |style, drag, _, _| {
                                if drag.subscription == highlight && drag.node.is_some() {
                                    style.bg(rgb(ACCENT_SOFT))
                                } else {
                                    style
                                }
                            })
                            .on_drop(cx.listener(move |this, drag: &PoolDrag, _, cx| {
                                if drag.subscription == target
                                    && let Some(name) = &drag.node
                                {
                                    this.submit(
                                        Command::MovePoolNodeTo {
                                            id: target.clone(),
                                            name: name.clone(),
                                            index: node_index,
                                        },
                                        "节点优先顺序已保存",
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }
                            }));
                    }
                }
                row = row
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .pl_1()
                            .child(name.clone()),
                    )
                    .when(!exists, |row| row.child(pill("已失效", 0xb47c26)))
                    .when(in_pool && exists, |row| row.child(self.node_health(&key)))
                    .child(self.pool_transfer(
                        format!("{side}-transfer-{key}"),
                        if in_pool { "移出" } else { "添加" },
                        false,
                        Command::SetPoolNodes {
                            id: sub.id.clone(),
                            names: vec![name],
                            in_pool: !in_pool,
                        },
                        cx,
                    ));
                group = group.child(row);
            }
        }
        group.into_any_element()
    }

    fn pool_transfer(
        &mut self,
        id: String,
        label: &str,
        disabled: bool,
        command: Command,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let include = matches!(command, Command::SetPoolNodes { in_pool: true, .. });
        let bulk = id.contains("group-transfer");
        let color = if include { GREEN } else { 0xb64c4a };
        let tip = label.to_owned();
        let (control, disabled) = self.control(id, disabled, Action::Command(command), cx);
        control
            .h(px(28.))
            .flex_shrink_0()
            .px_2()
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .gap_1()
            .text_size(px(12.))
            .text_color(rgb(color))
            .tooltip(move |_, cx| cx.new(|_| controls::Tip(tip.clone())).into())
            .when(!disabled, |control| {
                control.hover(move |style| style.bg(rgb(if include { 0xe9f5ef } else { 0xffefed })))
            })
            .child(
                svg()
                    .path(if include {
                        "icons/arrow-right.svg"
                    } else {
                        "icons/remove.svg"
                    })
                    .size(px(17.))
                    .text_color(rgb(color)),
            )
            .when(bulk, |control| control.child(label.to_owned()))
            .into_any_element()
    }

    fn pool_drag_handle(
        &mut self,
        sub: &ProfileSubscription,
        node: Option<String>,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = sub.id.clone();
        let key = node.clone();
        let drag = PoolDrag {
            subscription: id.clone(),
            label: node.clone().unwrap_or_else(|| sub.name.clone()),
            node,
        };
        let (handle, disabled) = self.control(
            format!("pool-drag-{}-{}", id, key.as_deref().unwrap_or("group")),
            false,
            Action::Noop,
            cx,
        );
        handle
            .w(px(26.))
            .flex_shrink_0()
            .rounded_md()
            .flex()
            .flex_col()
            .items_center()
            .cursor_move()
            .child(
                svg()
                    .path("icons/grip.svg")
                    .size(px(15.))
                    .text_color(rgb(MUTED)),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(MUTED))
                    .child((index + 1).to_string()),
            )
            .tooltip(|_, cx| cx.new(|_| PoolTip).into())
            .when(!disabled, |handle| {
                handle.on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            })
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.modifiers.alt
                    && matches!(event.keystroke.key.as_str(), "up" | "down")
                {
                    let index = if event.keystroke.key == "up" {
                        index.saturating_sub(1)
                    } else {
                        index + 1
                    };
                    let command = if let Some(name) = &key {
                        Command::MovePoolNodeTo {
                            id: id.clone(),
                            name: name.clone(),
                            index,
                        }
                    } else {
                        Command::MovePoolSubscriptionTo {
                            id: id.clone(),
                            index,
                        }
                    };
                    this.submit(command, "优先顺序已保存", cx);
                    cx.stop_propagation();
                }
            }))
            .into_any_element()
    }
}
