use super::*;

pub(super) struct Tip(pub String);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(320.))
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(INK))
            .text_color(rgb(0xffffff))
            .text_size(px(12.))
            .child(self.0.clone())
    }
}

pub(super) fn node_count(count: usize) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(
            svg()
                .path("icons/subscriptions.svg")
                .size(px(14.))
                .text_color(rgb(MUTED)),
        )
        .child(count.to_string())
}

impl ClashFx {
    pub(super) fn quiet_button(
        &mut self,
        id: impl Into<String>,
        label: &str,
        disabled: bool,
        action: Action,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (control, disabled) = self.control(id, disabled, action, cx);
        control
            .h(px(30.))
            .px_3()
            .border_0()
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(13.))
            .text_color(rgb(MUTED))
            .focus(|style| style.bg(rgb(ACCENT_SOFT)).text_color(rgb(ACCENT)))
            .when(!disabled, |control| {
                control.hover(|style| style.bg(rgb(0xf4efea)).text_color(rgb(INK)))
            })
            .child(label.to_owned())
            .into_any_element()
    }

    pub(super) fn icon_button(
        &mut self,
        id: impl Into<String>,
        icon: &'static str,
        label: &str,
        disabled: bool,
        action: Action,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let destructive = matches!(action, Action::RemoveSubscription(_));
        let testing =
            matches!(action, Action::Command(Command::TestNodes(_))) && self.snapshot.testing_nodes;
        let label = label.to_owned();
        let (control, disabled) = self.control(id, disabled, action, cx);
        let glyph = svg()
            .path(icon)
            .size(px(16.))
            .text_color(rgb(if destructive { 0xb64c4a } else { 0x827369 }));
        let glyph = if testing {
            glyph
                .with_animation(
                    "testing-spin",
                    Animation::new(Duration::from_millis(1200)).repeat(),
                    |glyph, phase| {
                        glyph.with_transformation(Transformation::rotate(percentage(phase)))
                    },
                )
                .into_any_element()
        } else {
            glyph.into_any_element()
        };
        control
            .size(px(28.))
            .flex_shrink_0()
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(if destructive { 0xb64c4a } else { 0x827369 }))
            .tooltip(move |_, cx| cx.new(|_| Tip(label.clone())).into())
            .when(!disabled, |control| {
                control.hover(move |style| {
                    style
                        .bg(rgb(if destructive { 0xffefed } else { 0xf1ebe5 }))
                        .text_color(rgb(if destructive { 0xa13c3a } else { ACCENT }))
                })
            })
            .child(glyph)
            .into_any_element()
    }

    pub(super) fn segmented(
        &mut self,
        id: &'static str,
        choices: Vec<(String, Action)>,
        selected: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let choices = Arc::new(choices);
        let mut segments = div()
            .flex()
            .min_w_full()
            .p_1()
            .gap_1()
            .rounded_lg()
            .bg(rgb(0xeee9e3));
        for (index, (label, action)) in choices.iter().enumerate() {
            let (segment, _) = self.control(format!("{id}-{index}"), false, action.clone(), cx);
            let choices = choices.clone();
            let tip = label.clone();
            segments = segments.child(
                segment
                    .flex_1()
                    .min_w(px(80.))
                    .h(px(28.))
                    .px_3()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(rgb(if index == selected { INK } else { MUTED }))
                    .when(index == selected, |segment| {
                        segment
                            .bg(rgb(0xffffff))
                            .shadow_sm()
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(index != selected, |segment| {
                        segment.hover(|style| style.bg(rgb(0xf4efea)).text_color(rgb(INK)))
                    })
                    .tooltip(move |_, cx| cx.new(|_| Tip(tip.clone())).into())
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        let next = match event.keystroke.key.as_str() {
                            "left" => (index + choices.len() - 1) % choices.len(),
                            "right" => (index + 1) % choices.len(),
                            "home" => 0,
                            "end" => choices.len() - 1,
                            _ => return,
                        };
                        this.keyboard_focus = true;
                        this.handle(choices[next].1.clone(), window, cx);
                        if let Some(focus) = this.button_focus.get(&format!("{id}-{next}")) {
                            focus.focus(window);
                        }
                        cx.stop_propagation();
                    }))
                    .child(div().truncate().child(label.clone())),
            );
        }
        div()
            .id(id)
            .min_w_0()
            .overflow_x_scroll()
            .child(segments)
            .into_any_element()
    }
}
