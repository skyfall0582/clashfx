use super::*;

impl ClashFx {
    pub(super) fn subscription_dialog_open(&self) -> bool {
        self.page == Page::Subscriptions && (self.editing_subscription || self.show_import)
    }

    pub(super) fn subscription_dialog(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editing = self.editing_subscription;
        let local_file = editing
            && self
                .snapshot
                .config
                .subscriptions
                .iter()
                .find(|sub| Some(&sub.id) == self.selected_subscription.as_ref())
                .is_some_and(|sub| sub.url.is_none());
        let mut fields = div()
            .id("subscription-dialog-fields")
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.field("订阅名称", if editing { "sub_name" } else { "import_name" }));
        if local_file {
            if let Some(path) = self
                .snapshot
                .config
                .subscriptions
                .iter()
                .find(|sub| Some(&sub.id) == self.selected_subscription.as_ref())
                .and_then(|sub| sub.source_path.as_deref())
            {
                fields = fields.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(hint("本地文件"))
                        .child(
                            div()
                                .text_size(px(13.))
                                .child(subscriptions::wrap_source(path)),
                        ),
                );
            }
        } else {
            fields = fields
                .child(self.field("订阅链接", if editing { "sub_url" } else { "import_url" }));
        }
        fields =
            fields.child(self.field("备注", if editing { "sub_notes" } else { "import_notes" }));
        if let Some(error) = &self.local_error {
            fields = fields.child(
                div()
                    .id("subscription-dialog-error")
                    .max_h(px(96.))
                    .overflow_y_scroll()
                    .rounded_lg()
                    .p_3()
                    .bg(rgb(0xfff5f3))
                    .text_color(rgb(0xa34f48))
                    .text_size(px(13.))
                    .child(error.clone()),
            );
        }
        let mut footer = div().flex_shrink_0().flex().items_center().gap_3();
        if editing {
            if let Some(id) = self.selected_subscription.clone() {
                let (delete, disabled) =
                    self.control("dialog-delete", false, Action::RemoveSubscription(id), cx);
                footer = footer.child(
                    delete
                        .h(px(30.))
                        .px_2()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(13.))
                        .text_color(rgb(0xb64c4a))
                        .when(!disabled, |button| {
                            button.hover(|style| style.bg(rgb(0xffefed)).text_color(rgb(0xa13c3a)))
                        })
                        .child(
                            svg()
                                .path("icons/trash.svg")
                                .size(px(14.))
                                .text_color(rgb(0xb64c4a)),
                        )
                        .child("删除订阅"),
                );
            }
        } else {
            footer = footer.child(self.quiet_button(
                "dialog-file",
                "从文件导入",
                false,
                Action::ImportFile,
                cx,
            ));
        }
        footer = footer.child(div().flex_1()).child(self.quiet_button(
            "dialog-cancel",
            "取消",
            false,
            Action::CloseSubscriptionDialog,
            cx,
        ));
        footer = if editing {
            footer.child(self.button(
                "dialog-save",
                "保存",
                true,
                self.text("sub_name", cx).trim().is_empty(),
                Action::SaveSubscription,
                cx,
            ))
        } else {
            footer.child(self.button(
                "dialog-import",
                "导入订阅",
                true,
                self.text("import_url", cx).trim().is_empty(),
                Action::ImportUrl,
                cx,
            ))
        };
        div()
            .id("subscription-dialog-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(rgba(0x30251b66))
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.keyboard_focus = false;
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .w(px(500.))
                    .max_w_full()
                    .max_h(window.viewport_size().height - px(48.))
                    .p_4()
                    .rounded_2xl()
                    .bg(rgb(0xffffff))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(19.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if editing {
                                        "编辑订阅"
                                    } else {
                                        "导入订阅"
                                    }),
                            )
                            .child(self.icon_button(
                                "dialog-close",
                                "icons/close.svg",
                                "关闭",
                                false,
                                Action::CloseSubscriptionDialog,
                                cx,
                            )),
                    )
                    .child(fields)
                    .child(footer),
            )
            .into_any_element()
    }

    pub(super) fn subscription_dialog_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => {
                if self.inputs.values().any(|input| {
                    let input = input.read(cx);
                    input.focus_handle(cx).is_focused(window) && input.is_composing()
                }) {
                    return;
                }
                self.handle(Action::CloseSubscriptionDialog, window, cx);
            }
            "tab" => {
                self.keyboard_focus = true;
                let editing = self.editing_subscription;
                let local_file = editing
                    && self
                        .snapshot
                        .config
                        .subscriptions
                        .iter()
                        .find(|sub| Some(&sub.id) == self.selected_subscription.as_ref())
                        .is_some_and(|sub| sub.url.is_none());
                let mut order = Vec::new();
                if let Some(focus) = self.button_focus.get("dialog-close") {
                    order.push(focus.clone());
                }
                order.push(
                    self.inputs[if editing { "sub_name" } else { "import_name" }]
                        .read(cx)
                        .focus_handle(cx),
                );
                if !local_file {
                    order.push(
                        self.inputs[if editing { "sub_url" } else { "import_url" }]
                            .read(cx)
                            .focus_handle(cx),
                    );
                }
                order.push(
                    self.inputs[if editing { "sub_notes" } else { "import_notes" }]
                        .read(cx)
                        .focus_handle(cx),
                );
                if let Some(focus) = self.button_focus.get(if editing {
                    "dialog-delete"
                } else {
                    "dialog-file"
                }) {
                    order.push(focus.clone());
                }
                for id in [
                    "dialog-cancel",
                    if editing {
                        "dialog-save"
                    } else {
                        "dialog-import"
                    },
                ] {
                    if let Some(focus) = self.button_focus.get(id) {
                        order.push(focus.clone());
                    }
                }
                let current = order.iter().position(|focus| focus.is_focused(window));
                let next = if event.keystroke.modifiers.shift {
                    current.map_or(order.len() - 1, |index| {
                        (index + order.len() - 1) % order.len()
                    })
                } else {
                    current.map_or(0, |index| (index + 1) % order.len())
                };
                order[next].focus(window);
                cx.notify();
            }
            "s" if !self.pending && !self.snapshot.busy && event.keystroke.modifiers.control => {
                self.handle(
                    if self.editing_subscription {
                        Action::SaveSubscription
                    } else {
                        Action::ImportUrl
                    },
                    window,
                    cx,
                );
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

pub(crate) fn install(cx: &mut App) {
    cx.set_prompt_builder(|_, message, detail, actions, handle, window, cx| {
        let prompt = cx.new(|cx| ConfirmDialog {
            title: message.into(),
            name: detail.unwrap_or_default().into(),
            buttons: actions.to_vec(),
            focus: cx.focus_handle(),
            selected: 0,
        });
        handle.with_view(prompt, window, cx)
    });
}

struct ConfirmDialog {
    title: String,
    name: String,
    buttons: Vec<PromptButton>,
    focus: FocusHandle,
    selected: usize,
}

impl EventEmitter<PromptResponse> for ConfirmDialog {}
impl Focusable for ConfirmDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ConfirmDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut actions = div().flex_shrink_0().flex().justify_end().gap_3().pt_4();
        for (index, button) in self.buttons.iter().enumerate() {
            actions = actions.child(
                div()
                    .id(index)
                    .px_4()
                    .py_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(if self.selected == index { ACCENT } else { LINE }))
                    .bg(rgb(if index == 1 { 0xb64c4a } else { 0xffffff }))
                    .text_color(rgb(if index == 1 { 0xffffff } else { INK }))
                    .cursor_pointer()
                    .child(button.label().clone())
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PromptResponse(index)))),
            );
        }
        div()
            .id("confirmation-overlay")
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .occlude()
            .bg(rgba(0x30251b66))
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => cx.emit(PromptResponse(0)),
                    "enter" => cx.emit(PromptResponse(this.selected)),
                    "tab" | "left" | "right" => {
                        this.selected = (this.selected + 1) % this.buttons.len();
                        cx.notify();
                    }
                    _ => {}
                }
                cx.stop_propagation();
            }))
            .child(
                div()
                    .w(px(440.))
                    .max_w_full()
                    .rounded_2xl()
                    .bg(rgb(0xffffff))
                    .shadow_lg()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(21.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK))
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .id("confirmation-name")
                            .max_h(px(100.))
                            .overflow_y_scroll()
                            .text_size(px(14.))
                            .text_color(rgb(INK))
                            .child(subscriptions::wrap_source(&self.name)),
                    )
                    .child(hint("将从 ClashFx 中移除此订阅及其节点。"))
                    .child(actions),
            )
    }
}
