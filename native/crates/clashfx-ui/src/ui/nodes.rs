use super::*;

impl ClashFx {
    fn node_subscription_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.snapshot.clone();
        let selected = snapshot
            .config
            .subscriptions
            .iter()
            .find(|sub| Some(&sub.id) == self.node_subscription.as_ref());
        let name = selected
            .map(|sub| sub.name.as_str())
            .unwrap_or("全部订阅")
            .to_owned();
        let tip = name.clone();
        let (trigger, _) = self.control(
            "node-subscription-picker",
            false,
            Action::ToggleNodePicker,
            cx,
        );
        let trigger = trigger
            .h(px(34.))
            .px_3()
            .rounded_lg()
            .border_color(rgb(0xe8e2db))
            .bg(rgb(0xffffff))
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(13.))
            .tooltip(move |_, cx| cx.new(|_| controls::Tip(tip.clone())).into())
            .child(div().flex_1().min_w_0().truncate().child(name))
            .child(
                svg()
                    .path("icons/chevron-down.svg")
                    .size(px(15.))
                    .text_color(rgb(MUTED)),
            );
        let view = cx.entity().downgrade();
        let mut root = div()
            .w(px(200.))
            .flex_shrink_0()
            .relative()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        view.update(cx, |this, _| this.node_picker_bounds = bounds)
                            .ok();
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(trigger);
        if self.node_picker_open {
            let query = self.text("node_subscription_search", cx).to_lowercase();
            let mut choices = vec![(None, "全部订阅".to_owned())];
            choices.extend(
                snapshot
                    .config
                    .subscriptions
                    .iter()
                    .filter(|sub| sub.name.to_lowercase().contains(&query))
                    .map(|sub| (Some(sub.id.clone()), sub.name.clone())),
            );
            let mut list = div()
                .id("node-subscription-options")
                .max_h(px(270.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_1();
            for (id, name) in choices {
                let selected = id == self.node_subscription;
                let (row, _) = self.control(
                    format!(
                        "node-subscription-option-{}",
                        id.as_deref().unwrap_or("all")
                    ),
                    false,
                    Action::SetNodeSubscription(id),
                    cx,
                );
                list = list.child(
                    row.px_3()
                        .py_2()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_2()
                        .hover(|style| style.bg(rgb(ACCENT_SOFT)))
                        .child(div().flex_1().min_w_0().text_size(px(13.)).child(name))
                        .when(selected, |row| {
                            row.child(
                                svg()
                                    .path("icons/check.svg")
                                    .size(px(16.))
                                    .text_color(rgb(ACCENT)),
                            )
                        }),
                );
            }
            root = root.child(
                deferred(
                    div()
                        .id("node-subscription-popup")
                        .absolute()
                        .top(px(39.))
                        .left_0()
                        .w(px(360.))
                        .p_2()
                        .rounded_lg()
                        .shadow_lg()
                        .border_1()
                        .border_color(rgb(LINE))
                        .bg(rgb(0xffffff))
                        .occlude()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            if !this.node_picker_bounds.contains(&event.position) {
                                this.node_picker_open = false;
                                cx.notify();
                            }
                        }))
                        .child(self.inputs["node_subscription_search"].clone())
                        .child(list),
                )
                .with_priority(1),
            );
        }
        root.into_any_element()
    }

    pub(super) fn nodes(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.snapshot.clone();
        if self.node_subscription.as_ref().is_some_and(|id| {
            !snapshot
                .config
                .subscriptions
                .iter()
                .any(|sub| &sub.id == id)
        }) {
            self.node_subscription = None;
        }
        let filter = self.node_subscription_picker(cx);
        let mut grid = div()
            .grid()
            .gap_3()
            .when(window.viewport_size().width >= px(1250.), |grid| {
                grid.grid_cols(3)
            })
            .when(window.viewport_size().width < px(1250.), |grid| {
                grid.grid_cols(2)
            });
        let query = self.text("node_search", cx).to_lowercase();
        let mut count = 0;
        for node in &snapshot.nodes {
            if self
                .node_subscription
                .as_ref()
                .is_some_and(|id| id != &node.candidate.subscription_id)
                || (!query.is_empty()
                    && !format!(
                        "{} {} {}",
                        node.candidate.name, node.candidate.subscription_name, node.kind
                    )
                    .to_lowercase()
                    .contains(&query))
            {
                continue;
            }
            count += 1;
            let active = snapshot
                .active
                .as_ref()
                .is_some_and(|active| active.key == node.candidate.key);
            let (tile, _) = self.control(
                format!("proxy-{}", node.candidate.key),
                active,
                Action::Command(Command::SelectNode(node.candidate.key.clone())),
                cx,
            );
            grid = grid.child(
                tile.min_w_0()
                    .p_3()
                    .rounded_lg()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .bg(rgb(if active { ACCENT_SOFT } else { 0xffffff }))
                    .border_color(rgb(if active { ACCENT } else { LINE }))
                    .when(active, |tile| tile.opacity(1.))
                    .when(!active, |tile| {
                        tile.hover(|style| {
                            style.bg(rgb(ACCENT_SOFT)).border_color(rgb(ACCENT_BORDER))
                        })
                    })
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(node.candidate.name.clone()),
                            )
                            .when(active, |row| {
                                row.child(
                                    svg()
                                        .path("icons/check.svg")
                                        .size(px(18.))
                                        .flex_shrink_0()
                                        .text_color(rgb(ACCENT)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(hint(&format!(
                                "{} · {}",
                                node.candidate.subscription_name, node.kind
                            ))))
                            .child(self.node_health(&node.candidate.key)),
                    ),
            );
        }
        let body = div()
            .id("proxy-grid")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .when(count == 0, |body| {
                body.child(empty(
                    "icons/subscriptions.svg",
                    if snapshot.nodes.is_empty() {
                        "导入订阅后即可选择节点"
                    } else {
                        "没有匹配的节点"
                    },
                    "",
                ))
            })
            .child(grid);
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
                    .child(self.icon_button(
                        "nodes-back",
                        "icons/back.svg",
                        "返回",
                        false,
                        Action::Navigate(self.node_return_page),
                        cx,
                    ))
                    .child(filter)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.inputs["node_search"].clone()),
                    )
                    .child(self.icon_button(
                        "nodes-test",
                        "icons/overview.svg",
                        if snapshot.testing_nodes {
                            "测速中…"
                        } else {
                            "测速"
                        },
                        !snapshot.running || snapshot.nodes.is_empty() || snapshot.testing_nodes,
                        Action::Command(Command::TestNodes(self.node_subscription.clone())),
                        cx,
                    )),
            )
            .child(body)
            .into_any_element()
    }
}
