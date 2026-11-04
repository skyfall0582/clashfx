use super::*;
use clashfx_core::model::Activity;

impl ClashFx {
    pub(super) fn filtered_logs(&self, cx: &App) -> Vec<(&Activity, &'static str)> {
        let filter = self.text("log_filter", cx).to_lowercase();
        let mut entries: Vec<_> = self
            .snapshot
            .activity
            .iter()
            .map(|entry| (entry, "ClashFx"))
            .chain(
                self.snapshot
                    .core_logs
                    .iter()
                    .map(|entry| (entry, "Mihomo")),
            )
            .filter(|(entry, source)| {
                let level_matches = match self.log_level {
                    1 => matches!(entry.level.as_str(), "warn" | "warning"),
                    2 => matches!(entry.level.as_str(), "error" | "fatal"),
                    _ => true,
                };
                level_matches
                    && (filter.is_empty()
                        || entry.message.to_lowercase().contains(&filter)
                        || entry.level.to_lowercase().contains(&filter)
                        || source.to_lowercase().contains(&filter))
            })
            .collect();
        entries.sort_by_key(|(entry, _)| std::cmp::Reverse(entry.at));
        entries
    }

    pub(super) fn logs(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let matching = self.filtered_logs(cx);
        let has_entries = !matching.is_empty();
        let mut content = div()
            .id("log-entries")
            .h_full()
            .min_h_0()
            .p_3()
            .pr_5()
            .overflow_y_scroll()
            .track_scroll(&self.log_scroll)
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .flex()
            .flex_col();
        if !has_entries {
            content = content.child(empty(
                "icons/logs.svg",
                if self.log_level == 0 && self.text("log_filter", cx).is_empty() {
                    "暂无日志"
                } else {
                    "没有匹配的日志"
                },
                "",
            ));
        }
        for (item, source) in matching {
            let (label, color) = match item.level.as_str() {
                "error" | "fatal" => ("ERROR", 0xb64c4a),
                "warn" | "warning" => ("WARNING", 0xa16c19),
                "debug" => ("DEBUG", MUTED),
                _ => ("INFO", MUTED),
            };
            content = content.child(
                div()
                    .flex_shrink_0()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_size(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_size(px(11.))
                            .child(
                                div().w(px(83.)).text_color(rgb(MUTED)).child(
                                    item.at
                                        .with_timezone(&chrono::Local)
                                        .format("%H:%M:%S%.3f")
                                        .to_string(),
                                ),
                            )
                            .child(
                                div()
                                    .w(px(62.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(color))
                                    .child(label),
                            )
                            .child(div().text_color(rgb(MUTED)).child(source)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_color(rgb(if label == "INFO" || label == "DEBUG" {
                                INK
                            } else {
                                color
                            }))
                            .child(item.message.clone()),
                    ),
            );
        }
        let choices = ["全部", "警告", "错误"]
            .into_iter()
            .enumerate()
            .map(|(index, label)| (label.to_owned(), Action::SetLogLevel(index)))
            .collect();
        let toolbar = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .child(div().w(px(268.)).flex_shrink_0().child(self.segmented(
                "log-level",
                choices,
                self.log_level,
                cx,
            )))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.inputs["log_filter"].clone()),
            )
            .child(self.icon_button(
                "copy-logs",
                "icons/copy.svg",
                "复制当前筛选的日志",
                !has_entries,
                Action::CopyLogs,
                cx,
            ))
            .child(self.icon_button(
                "clear-logs",
                "icons/trash.svg",
                "清空日志",
                self.snapshot.activity.is_empty() && self.snapshot.core_logs.is_empty(),
                Action::Command(Command::ClearLogs),
                cx,
            ));
        div()
            .h_full()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap_2()
            .child(toolbar)
            .child(
                panel()
                    .p_0()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .relative()
                    .child(content)
                    .child(self.log_scrollbar(cx)),
            )
            .into_any_element()
    }
}
