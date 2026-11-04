use crate::input::TextInput;
use clashfx_core::{
    engine::{Backend, Command},
    model::{AutoPolicy, RoutingMode, Snapshot},
};
use gpui::{prelude::*, *};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
mod controls;
mod diagnostics;
pub(crate) mod dialogs;
mod health;
mod network;
mod nodes;
mod pool;
mod profiles;
mod scrollbar;
mod settings;
mod subscriptions;
use subscriptions::SubscriptionDraft;

pub(crate) const INK: u32 = 0x332d28;
const MUTED: u32 = 0x897f75;
const LINE: u32 = 0xe8e2db;
pub(crate) const ACCENT: u32 = 0xbf5212;
const ACCENT_HOVER: u32 = 0xa74610;
const ACCENT_SOFT: u32 = 0xfff8f2;
const ACCENT_BORDER: u32 = 0xeac5aa;
const GREEN: u32 = 0x238a68;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Subscriptions,
    Nodes,
    Policy,
    Logs,
    Settings,
}

#[derive(Clone)]
enum Action {
    Noop,
    DismissError,
    Command(Command),
    Navigate(Page),
    ImportUrl,
    ImportFile,
    SaveSubscription,
    EditSubscription(String),
    ShowNodes(String),
    SetNodeSubscription(Option<String>),
    ToggleNodePicker,
    SetPolicyTab(bool),
    ToggleImport,
    CloseSubscriptionDialog,
    CopySubscriptionUrl(String),
    ReplaceSubscriptionFile(String),
    TogglePoolGroup { id: String, pool: bool },
    SavePolicy,
    RemoveSubscription(String),
    SetLogLevel(usize),
    CopyLogs,
    OpenDataDir,
    OpenUrl(&'static str),
}

pub struct ClashFx {
    backend: Backend,
    network_view: Entity<network::NetworkView>,
    snapshot: Arc<Snapshot>,
    page: Page,
    node_return_page: Page,
    selected_subscription: Option<String>,
    remove_confirmation: Option<String>,
    local_error: Option<String>,
    error_until: Option<std::time::Instant>,
    inputs: HashMap<&'static str, Entity<TextInput>>,
    button_focus: HashMap<String, FocusHandle>,
    keyboard_focus: bool,
    dialog_return_focus: Option<FocusHandle>,
    log_level: usize,
    show_import: bool,
    editing_subscription: bool,
    expanded_available: HashSet<String>,
    collapsed_pool: HashSet<String>,
    node_subscription: Option<String>,
    node_picker_open: bool,
    node_picker_bounds: Bounds<Pixels>,
    log_scroll: ScrollHandle,
    log_scroll_drag: Option<(Pixels, Pixels)>,
    policy_candidates: bool,
    drafts: HashMap<String, SubscriptionDraft>,
    pending: bool,
    exiting: bool,
    notice: Option<String>,
    notice_until: Option<std::time::Instant>,
    _input_observers: Vec<Subscription>,
}

impl ClashFx {
    pub fn new(backend: Backend, cx: &mut Context<Self>) -> Self {
        let snapshot = backend.snapshot();
        let policy = &snapshot.config.policy;
        let fields = [
            ("import_name", String::new(), "可选，留空自动命名"),
            ("import_url", String::new(), "https://…"),
            (
                "import_notes",
                String::new(),
                "网站、平台、使用说明等（可选）",
            ),
            ("sub_name", String::new(), "订阅名称"),
            ("sub_url", String::new(), "https://…"),
            ("sub_notes", String::new(), "网站、平台、使用说明等（可选）"),
            ("subscription_search", String::new(), "搜索订阅名称或来源"),
            ("node_search", String::new(), "搜索节点"),
            ("node_subscription_search", String::new(), "搜索订阅"),
            ("pool_available_search", String::new(), "搜索订阅或节点"),
            ("pool_member_search", String::new(), "搜索池内节点"),
            (
                "failure_threshold",
                policy.failure_threshold.to_string(),
                "3",
            ),
            (
                "failure_window_secs",
                policy.failure_window_secs.to_string(),
                "30",
            ),
            (
                "probe_timeout_ms",
                policy.probe_timeout_ms.to_string(),
                "4000",
            ),
            ("cooldown_secs", policy.cooldown_secs.to_string(), "60"),
            (
                "check_interval_secs",
                policy.check_interval_secs.to_string(),
                "60",
            ),
            (
                "recovery_successes",
                policy.recovery_successes.to_string(),
                "3",
            ),
            (
                "manual_hold_secs",
                policy.manual_hold_secs.to_string(),
                "300",
            ),
            (
                "unavailable_retry_secs",
                policy.unavailable_retry_secs.to_string(),
                "30",
            ),
            (
                "probe_urls",
                policy.probe_urls.join("; "),
                "返回 HTTP 204 的地址；多个地址用分号分隔",
            ),
            ("log_filter", String::new(), "筛选日志"),
        ];
        let inputs: HashMap<_, _> = fields
            .into_iter()
            .map(|(key, value, placeholder)| {
                (
                    key,
                    cx.new(|cx| {
                        let input = TextInput::new(value, placeholder, cx);
                        if matches!(key, "import_notes" | "sub_notes") {
                            input.multiline()
                        } else {
                            input
                        }
                    }),
                )
            })
            .collect();
        cx.spawn(async move |this, cx| {
            loop {
                gpui::Timer::after(Duration::from_millis(250)).await;
                if this
                    .update(cx, |this, cx| {
                        let snapshot = this.backend.snapshot();
                        if snapshot.revision != this.snapshot.revision {
                            this.sync_snapshot(snapshot, cx);
                        }
                        if this
                            .notice_until
                            .is_some_and(|until| until <= std::time::Instant::now())
                        {
                            this.notice = None;
                            this.notice_until = None;
                            cx.notify();
                        }
                        if !this.subscription_dialog_open()
                            && this
                                .error_until
                                .is_some_and(|until| until <= std::time::Instant::now())
                        {
                            this.local_error = None;
                            this.error_until = None;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let input_observers = inputs
            .values()
            .map(|input| cx.observe(input, |_, _, cx| cx.notify()))
            .collect();
        let expanded_available = snapshot
            .config
            .subscriptions
            .first()
            .map(|sub| sub.id.clone())
            .into_iter()
            .collect();
        let logo = Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../../assets/clashfx-sidebar.png").to_vec(),
        ));
        let network_view = cx.new(|_| {
            network::NetworkView::new(
                network::NetworkState::from_snapshot(&snapshot),
                logo.clone(),
            )
        });
        let mut this = Self {
            backend,
            network_view,
            snapshot: Arc::new(snapshot),
            page: Page::Overview,
            node_return_page: Page::Overview,
            selected_subscription: None,
            remove_confirmation: None,
            local_error: None,
            error_until: None,
            inputs,
            button_focus: HashMap::new(),
            keyboard_focus: false,
            dialog_return_focus: None,
            log_level: 0,
            show_import: false,
            editing_subscription: false,
            expanded_available,
            collapsed_pool: HashSet::new(),
            node_subscription: None,
            node_picker_open: false,
            node_picker_bounds: Bounds::default(),
            log_scroll: ScrollHandle::new(),
            log_scroll_drag: None,
            policy_candidates: true,
            drafts: HashMap::new(),
            pending: false,
            exiting: false,
            notice: None,
            notice_until: None,
            _input_observers: input_observers,
        };
        if let Some(sub) = this.snapshot.config.subscriptions.first() {
            this.select_subscription(sub.id.clone(), cx);
        }
        if let Some(error) = this.snapshot.last_error.clone() {
            this.report_error(error, cx);
        }
        this
    }

    fn text(&self, key: &'static str, cx: &App) -> String {
        self.inputs[key].read(cx).text()
    }

    #[cfg(windows)]
    pub(crate) fn tray_status(&self) -> crate::tray::Status {
        crate::tray::Status::from_snapshot(&self.snapshot)
    }

    #[cfg(windows)]
    pub(crate) fn quit(&mut self, cx: &mut Context<Self>) {
        if self.exiting {
            return;
        }
        self.exiting = true;
        let response = self.backend.request(Command::Stop);
        cx.spawn(async move |this, cx| {
            let result = response.await;
            this.update(cx, |this, cx| {
                this.exiting = false;
                match result {
                    Ok(()) => cx.quit(),
                    Err(error) => {
                        this.report_error(format!("无法关闭代理，已取消退出：{error}"), cx);
                        crate::tray::show_existing(std::path::Path::new(&this.snapshot.data_dir));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn handle(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.local_error = None;
        match action {
            Action::Noop => {}
            Action::DismissError => {
                self.local_error = None;
                self.error_until = None;
            }
            Action::Command(command) => {
                let message = subscriptions::command_feedback(&command);
                self.submit(command, message, cx);
            }
            Action::Navigate(page) => {
                self.node_picker_open = false;
                if page == Page::Nodes {
                    self.node_return_page = self.page;
                    self.node_subscription = None;
                    self.inputs["node_search"].update(cx, |input, cx| input.set_text("", cx));
                }
                self.page = page;
                self.remove_confirmation = None;
            }
            Action::ImportUrl => {
                let name = self.text("import_name", cx);
                let url = self.text("import_url", cx);
                let notes = self.text("import_notes", cx);
                if url.trim().is_empty() {
                    self.local_error = Some("请输入订阅链接".into());
                } else {
                    self.submit(Command::ImportUrl { name, url, notes }, "订阅已导入", cx);
                }
            }
            Action::ImportFile => {
                self.choose_file(None, cx);
            }
            Action::SaveSubscription => {
                if let Some(id) = self.selected_subscription.clone() {
                    self.submit(
                        Command::EditSubscription {
                            id,
                            name: self.text("sub_name", cx),
                            url: Some(self.text("sub_url", cx)),
                            notes: self.text("sub_notes", cx),
                        },
                        "订阅修改已保存",
                        cx,
                    );
                }
            }
            Action::EditSubscription(id) => {
                self.dialog_return_focus = window.focused(cx);
                self.select_subscription(id, cx);
                self.reset_subscription(cx);
                self.editing_subscription = true;
                self.show_import = false;
                self.inputs["sub_name"]
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window);
            }
            Action::ShowNodes(id) => {
                self.node_return_page = self.page;
                self.node_subscription = Some(id);
                self.page = Page::Nodes;
                self.inputs["node_search"].update(cx, |input, cx| input.set_text("", cx));
            }
            Action::SetNodeSubscription(id) => {
                self.node_subscription = id;
                self.node_picker_open = false;
            }
            Action::ToggleNodePicker => {
                self.node_picker_open = !self.node_picker_open;
                if self.node_picker_open {
                    self.inputs["node_subscription_search"]
                        .update(cx, |input, cx| input.set_text("", cx));
                    self.inputs["node_subscription_search"]
                        .read(cx)
                        .focus_handle(cx)
                        .focus(window);
                }
            }
            Action::SetPolicyTab(candidates) => self.policy_candidates = candidates,
            Action::ToggleImport => {
                self.dialog_return_focus = window.focused(cx);
                self.show_import = true;
                self.editing_subscription = false;
                self.inputs["import_url"]
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window);
            }
            Action::CloseSubscriptionDialog => {
                if !self.pending && !self.snapshot.busy {
                    self.reset_subscription(cx);
                    self.show_import = false;
                }
            }
            Action::CopySubscriptionUrl(url) => {
                cx.write_to_clipboard(ClipboardItem::new_string(url));
                self.show_notice("链接已复制");
            }
            Action::ReplaceSubscriptionFile(id) => self.choose_file(Some(id), cx),
            Action::TogglePoolGroup { id, pool } => {
                let set = if pool {
                    &mut self.collapsed_pool
                } else {
                    &mut self.expanded_available
                };
                if !set.remove(&id) {
                    set.insert(id);
                }
            }
            Action::SavePolicy => match self.read_policy(cx) {
                Ok(policy) => self.submit(Command::SavePolicy(policy), "切换设置已保存", cx),
                Err(error) => self.local_error = Some(error.to_string()),
            },
            Action::RemoveSubscription(id) => {
                if self.remove_confirmation.is_some() {
                    return;
                }
                let Some(sub) = self
                    .snapshot
                    .config
                    .subscriptions
                    .iter()
                    .find(|sub| sub.id == id)
                else {
                    return;
                };
                let name = sub.name.clone();
                self.remove_confirmation = Some(id.clone());
                let response = window.prompt(
                    PromptLevel::Warning,
                    "删除订阅？",
                    Some(&name),
                    &[
                        PromptButton::Cancel("取消".into()),
                        PromptButton::Other("删除订阅".into()),
                    ],
                    cx,
                );
                cx.spawn(async move |this, cx| {
                    let answer = response.await;
                    this.update(cx, |this, cx| {
                        this.remove_confirmation = None;
                        if matches!(answer, Ok(1)) {
                            this.submit(Command::RemoveSubscription(id), "订阅已删除", cx);
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            Action::SetLogLevel(level) => {
                self.log_level = level;
                self.log_scroll.set_offset(point(px(0.), px(0.)));
            }
            Action::CopyLogs => {
                let text = self
                    .filtered_logs(cx)
                    .into_iter()
                    .map(|(item, source)| {
                        format!(
                            "{} [{}] {} {}",
                            item.at
                                .with_timezone(&chrono::Local)
                                .format("%Y-%m-%d %H:%M:%S%.3f"),
                            source,
                            item.level.to_uppercase(),
                            item.message
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.show_notice("日志已复制");
            }
            Action::OpenUrl(url) => cx.open_url(url),
            Action::OpenDataDir => {
                #[cfg(windows)]
                std::process::Command::new("explorer.exe")
                    .arg(&self.snapshot.data_dir)
                    .spawn()
                    .ok();
            }
        }
        if self.local_error.is_some() {
            self.error_until = Some(std::time::Instant::now() + Duration::from_secs(8));
        }
        cx.notify();
    }

    pub(crate) fn report_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.local_error = Some(message);
        self.error_until = Some(std::time::Instant::now() + Duration::from_secs(8));
        cx.notify();
    }

    fn choose_file(&self, replace_id: Option<String>, cx: &mut Context<Self>) {
        let name = self.text("import_name", cx);
        let notes = self.text("import_notes", cx);
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 Clash / Mihomo YAML".into()),
        });
        cx.spawn(async move |this, cx| {
            let selection = receiver.await;
            this.update(cx, |this, cx| {
                match selection {
                    Ok(Ok(Some(mut paths))) => {
                        if let Some(path) = paths.pop() {
                            if let Some(id) = replace_id {
                                this.submit(
                                    Command::ReplaceSubscriptionFile { id, path },
                                    "文件已替换，保留原有节点选择",
                                    cx,
                                );
                            } else {
                                this.submit(
                                    Command::ImportFile { name, path, notes },
                                    "订阅已导入",
                                    cx,
                                );
                            }
                        }
                    }
                    Ok(Err(error)) => this.local_error = Some(format!("文件选择失败：{error}")),
                    Err(_) => this.local_error = Some("文件选择窗口未返回结果".into()),
                    _ => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn read_policy(&self, cx: &App) -> anyhow::Result<AutoPolicy> {
        let number = |key| {
            self.text(key, cx)
                .trim()
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("请输入有效的整数"))
        };
        let mut policy = self.snapshot.config.policy.clone();
        policy.failure_threshold = u32::try_from(number("failure_threshold")?)?;
        policy.failure_window_secs = number("failure_window_secs")?;
        policy.probe_timeout_ms = number("probe_timeout_ms")?;
        policy.cooldown_secs = number("cooldown_secs")?;
        policy.check_interval_secs = number("check_interval_secs")?;
        policy.recovery_successes = u32::try_from(number("recovery_successes")?)?;
        policy.manual_hold_secs = number("manual_hold_secs")?;
        policy.unavailable_retry_secs = number("unavailable_retry_secs")?;
        policy.probe_urls = self
            .text("probe_urls", cx)
            .split(';')
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(String::from)
            .collect();
        policy.validate()?;
        Ok(policy)
    }

    fn control(
        &mut self,
        id: impl Into<String>,
        disabled: bool,
        action: Action,
        cx: &mut Context<Self>,
    ) -> (Stateful<Div>, bool) {
        let disabled = disabled
            || self.exiting
            || ((self.snapshot.busy || self.pending)
                && !matches!(
                    action,
                    Action::DismissError
                        | Action::Navigate(_)
                        | Action::OpenDataDir
                        | Action::OpenUrl(_)
                        | Action::CopySubscriptionUrl(_)
                        | Action::ToggleImport
                        | Action::TogglePoolGroup { .. }
                        | Action::SetNodeSubscription(_)
                        | Action::ToggleNodePicker
                        | Action::SetPolicyTab(_)
                        | Action::SetLogLevel(_)
                        | Action::CopyLogs
                ));
        let id = id.into();
        let focus = self
            .button_focus
            .entry(id.clone())
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let focus_mouse = focus.clone();
        let key_action = action.clone();
        let control = div()
            .id(SharedString::from(id))
            .track_focus(&focus)
            .border_1()
            .border_color(gpui::transparent_black())
            .when(disabled, |button| button.opacity(0.4))
            .when(!disabled, |button| button.cursor_pointer())
            .when(self.keyboard_focus, |control| {
                control.focus(|style| style.border_color(rgb(ACCENT)))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.keyboard_focus = false;
                    if !disabled {
                        focus_mouse.focus(window);
                    }
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if !disabled {
                    this.handle(action.clone(), window, cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if !disabled
                    && !event.keystroke.modifiers.alt
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                    && matches!(event.keystroke.key.as_str(), "enter" | "space")
                {
                    this.handle(key_action.clone(), window, cx);
                    cx.stop_propagation();
                }
            }));
        (control, disabled)
    }

    fn button(
        &mut self,
        id: impl Into<String>,
        label: impl Into<String>,
        primary: bool,
        disabled: bool,
        action: Action,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (control, disabled) = self.control(id, disabled, action, cx);
        control
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .h(px(30.))
            .px_3()
            .rounded_lg()
            .border_color(rgb(if primary { ACCENT } else { 0xe8e2db }))
            .bg(rgb(if primary { ACCENT } else { 0xffffff }))
            .text_color(rgb(if primary { 0xffffff } else { 0x675b50 }))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .when(!disabled, |button| {
                button.hover(move |style| {
                    style.bg(rgb(if primary { ACCENT_HOVER } else { 0xf4efea }))
                })
            })
            .child(label.into())
            .into_any_element()
    }

    fn toggle(
        &mut self,
        id: &str,
        enabled: bool,
        disabled: bool,
        action: Action,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (control, disabled) = self.control(id, disabled, action, cx);
        control
            .w(px(36.))
            .h(px(22.))
            .flex_shrink_0()
            .rounded_full()
            .flex()
            .items_center()
            .px(px(3.))
            .bg(rgb(if enabled { ACCENT } else { 0xdcd5ce }))
            .when(enabled, |switch| switch.justify_end())
            .when(!disabled, |switch| {
                switch.hover(move |style| {
                    style.bg(rgb(if enabled { ACCENT_HOVER } else { 0xcfc6bc }))
                })
            })
            .child(
                div()
                    .size(px(14.))
                    .rounded_full()
                    .bg(rgb(0xffffff))
                    .shadow_sm(),
            )
            .into_any_element()
    }

    fn nav_item(
        &mut self,
        page: Page,
        label: &str,
        icon: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected =
            self.page == page || (self.page == Page::Nodes && self.node_return_page == page);
        let (control, _) = self.control(format!("nav-{label}"), false, Action::Navigate(page), cx);
        control
            .w_full()
            .h(px(36.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_lg()
            .when(selected, |row| row.bg(rgb(ACCENT_SOFT)))
            .text_color(rgb(if selected { ACCENT } else { 0x766f68 }))
            .font_weight(if selected {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .hover(move |style| style.bg(rgb(if selected { ACCENT_SOFT } else { 0xf4f1ed })))
            .focus(|style| style.bg(rgb(ACCENT_SOFT)).text_color(rgb(ACCENT)))
            .child(
                svg()
                    .path(icon)
                    .size(px(16.))
                    .flex_shrink_0()
                    .text_color(rgb(if selected { ACCENT } else { 0x92877d })),
            )
            .child(div().flex_1().child(label.to_owned()))
            .when(selected, |row| {
                row.child(div().size(px(5.)).rounded_full().bg(rgb(ACCENT)))
            })
            .into_any_element()
    }

    fn sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut nav = div().flex().flex_col().gap(px(4.));
        for (page, label, mark) in [
            (Page::Overview, "连接", "icons/overview.svg"),
            (Page::Policy, "自动切换", "icons/smart.svg"),
            (Page::Subscriptions, "订阅池", "icons/subscriptions.svg"),
            (Page::Logs, "日志", "icons/logs.svg"),
        ] {
            nav = nav.child(self.nav_item(page, label, mark, cx));
        }
        div()
            .w(px(144.))
            .flex_shrink_0()
            .h_full()
            .px(px(4.))
            .py(px(8.))
            .bg(rgb(0xfdfcfb))
            .border_r_1()
            .border_color(rgb(LINE))
            .flex()
            .flex_col()
            .child(nav)
            .child(div().flex_1())
            .child(self.nav_item(Page::Settings, "设置", "icons/settings.svg", cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(8.))
                    .pt(px(8.))
                    .mt(px(8.))
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
            )
            .into_any_element()
    }

    fn overview(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.snapshot.clone();
        let current = snapshot
            .active
            .as_ref()
            .or_else(|| {
                snapshot.config.last_active.as_ref().and_then(|key| {
                    snapshot
                        .nodes
                        .iter()
                        .find(|node| &node.candidate.key == key)
                        .map(|node| &node.candidate)
                })
            })
            .or_else(|| snapshot.candidates.first())
            .or_else(|| snapshot.nodes.first().map(|node| &node.candidate));
        let mut controls = div().flex().flex_wrap().items_center().gap_2();
        if snapshot.nodes.is_empty() {
            controls = controls.child(self.button(
                "overview-import",
                "导入订阅",
                true,
                false,
                Action::Navigate(Page::Subscriptions),
                cx,
            ));
        } else {
            controls = controls.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_2()
                    .child(section_title("系统代理"))
                    .child(self.toggle(
                        "proxy-overview",
                        snapshot.system_proxy,
                        false,
                        Action::Command(Command::SystemProxy(!snapshot.system_proxy)),
                        cx,
                    )),
            );
        }
        if !snapshot.candidates.is_empty() {
            controls = controls.child(self.button(
                "probe-overview",
                "检测并切换",
                false,
                !snapshot.running || snapshot.probing,
                Action::Command(Command::Probe),
                cx,
            ));
        }
        if !snapshot.candidates.is_empty()
            && (snapshot.manual_hold_until.is_some() || !snapshot.config.policy.enabled)
        {
            controls = controls.child(self.button(
                "resume-overview",
                "恢复自动",
                false,
                false,
                Action::Command(Command::Resume),
                cx,
            ));
        }
        let node_selector = current.map(|node| {
            let (control, _) =
                self.control("current-node", false, Action::Navigate(Page::Nodes), cx);
            control
                .rounded_md()
                .px_2()
                .py_1()
                .ml_neg_2()
                .flex()
                .items_center()
                .gap_3()
                .hover(|style| style.bg(rgb(ACCENT_SOFT)))
                .child(
                    div()
                        .min_w_0()
                        .text_size(px(21.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(node.name.clone()),
                )
                .child(
                    svg()
                        .path("icons/chevron-down.svg")
                        .size(px(19.))
                        .flex_shrink_0()
                        .text_color(rgb(MUTED)),
                )
                .tooltip(|_, cx| cx.new(|_| controls::Tip("选择代理节点".into())).into())
                .into_any_element()
        });
        let connection = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .p(px(12.))
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .when_some(node_selector, |body, selector| body.child(selector))
                            .when_some(current, |body, node| {
                                body.child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(rgb(MUTED))
                                        .child(node.subscription_name.clone()),
                                )
                            })
                            .child(div().mt_2().child(controls)),
                    )
                    .child(self.network_view.clone()),
            )
            .child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .gap_5()
                            .child(metric("池内节点", snapshot.candidates.len().to_string()))
                            .child(metric(
                                "连续失败",
                                format!(
                                    "{} / {}",
                                    snapshot.failure_streak,
                                    snapshot.config.policy.failure_threshold
                                ),
                            ))
                            .child(metric("本次切换", snapshot.switch_count.to_string())),
                    ),
            );
        let mut activity = panel().min_w_0().flex().flex_col().gap_3().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(section_title("最近动态"))
                .child(self.button(
                    "open-logs",
                    "查看日志",
                    false,
                    false,
                    Action::Navigate(Page::Logs),
                    cx,
                )),
        );
        if snapshot.activity.is_empty() {
            activity = activity.child(empty("icons/logs.svg", "暂无切换记录", ""));
        }
        for item in snapshot.activity.iter().take(5) {
            activity = activity.child(
                div()
                    .flex()
                    .gap_3()
                    .text_size(px(12.))
                    .child(
                        div().w(px(60.)).text_color(rgb(MUTED)).child(
                            item.at
                                .with_timezone(&chrono::Local)
                                .format("%H:%M:%S")
                                .to_string(),
                        ),
                    )
                    .child(div().flex_1().min_w_0().child(item.message.clone())),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(connection)
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .items_start()
                    .gap_2()
                    .child(self.pool_status(cx))
                    .child(activity),
            )
            .into_any_element()
    }

    fn policy_rules(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let policy = self.snapshot.config.policy.clone();
        let mut next = policy.clone();
        next.enabled = !policy.enabled;
        let mut recovery = policy.clone();
        recovery.recovery_enabled = !policy.recovery_enabled;
        let toggles = panel()
            .flex()
            .items_center()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .child(section_title("自动故障转移"))
                    .child(self.toggle(
                        "auto-toggle",
                        policy.enabled,
                        false,
                        Action::Command(Command::SavePolicy(next)),
                        cx,
                    )),
            )
            .child(div().w(px(1.)).h(px(28.)).bg(rgb(LINE)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .child(section_title("自动切回优先节点"))
                    .child(self.toggle(
                        "recovery-toggle",
                        policy.recovery_enabled,
                        false,
                        Action::Command(Command::SavePolicy(recovery)),
                        cx,
                    )),
            );
        let mut fields = panel().flex().flex_col().gap_3();
        for pair in [
            [
                ("连续失败次数", "failure_threshold"),
                ("失败统计窗口 · 秒", "failure_window_secs"),
            ],
            [
                ("节点检测总超时 · 毫秒", "probe_timeout_ms"),
                ("自动切回冷却 · 秒", "cooldown_secs"),
            ],
            [
                ("当前节点检查间隔 · 秒", "check_interval_secs"),
                ("恢复确认次数", "recovery_successes"),
            ],
            [
                ("手动选择保持 · 秒", "manual_hold_secs"),
                ("全部不可用重试 · 秒", "unavailable_retry_secs"),
            ],
        ] {
            fields = fields.child(
                div()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().child(self.field(pair[0].0, pair[0].1)))
                    .child(div().flex_1().child(self.field(pair[1].0, pair[1].1))),
            );
        }
        fields = fields
            .child(self.field("连通性检测地址", "probe_urls"))
            .child(hint(
                "HTTP 204 地址；多个地址用分号分隔。检测地址与复检共用总超时，备用节点按需检测。",
            ))
            .child(div().flex().justify_end().child(self.button(
                "save-policy",
                "保存设置",
                true,
                false,
                Action::SavePolicy,
                cx,
            )));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(toggles)
            .child(fields)
            .into_any_element()
    }

    fn field(&self, label: &str, key: &'static str) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(label.to_owned()),
            )
            .child(self.inputs[key].clone())
    }
}

impl Render for ClashFx {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog_open = self.subscription_dialog_open();
        if !dialog_open && let Some(focus) = self.dialog_return_focus.take() {
            focus.focus(window);
        }
        let content = match self.page {
            Page::Overview => self.overview(cx),
            Page::Subscriptions => self.subscription_management(cx),
            Page::Nodes => self.nodes(window, cx),
            Page::Policy => self.policy(cx),
            Page::Logs => self.logs(cx),
            Page::Settings => self.settings(cx),
        };
        let sidebar = self.sidebar(cx);
        let mut main = div()
            .id(match self.page {
                Page::Overview => "scroll-overview",
                Page::Subscriptions => "scroll-subscriptions",
                Page::Nodes => "scroll-nodes",
                Page::Policy => "scroll-policy",
                Page::Logs => "scroll-logs",
                Page::Settings => "scroll-settings",
            })
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .p(px(4.))
            .bg(rgb(0xf8f7f5));
        let feedback = self
            .local_error
            .clone()
            .map(|message| (message, 0xa34f48, 0xfff1ef))
            .or_else(|| {
                self.notice
                    .clone()
                    .map(|message| (message, GREEN, 0xeaf6f0))
            })
            .or_else(|| {
                (self.pending || self.snapshot.busy).then(|| {
                    (
                        if self.snapshot.busy {
                            self.snapshot.phase.clone()
                        } else {
                            "正在处理…".into()
                        },
                        ACCENT,
                        ACCENT_SOFT,
                    )
                })
            });
        main = if matches!(
            self.page,
            Page::Subscriptions | Page::Nodes | Page::Policy | Page::Logs
        ) {
            main.child(div().flex_1().min_h_0().child(content))
        } else {
            main.child(
                div()
                    .id("main-page-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(content),
            )
        };
        let dialog = dialog_open.then(|| self.subscription_dialog(window, cx));
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .on_mouse_move(cx.listener(Self::drag_log_scrollbar))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.log_scroll_drag = None;
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.node_picker_open
                    && event.keystroke.key == "escape"
                    && !this.inputs["node_subscription_search"]
                        .read(cx)
                        .is_composing()
                {
                    this.node_picker_open = false;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if this.subscription_dialog_open() {
                    this.subscription_dialog_key(event, window, cx);
                    return;
                }
                if event.keystroke.modifiers.control && event.keystroke.key == "s" && !this.pending
                {
                    match this.page {
                        Page::Subscriptions if this.editing_subscription => {
                            this.handle(Action::SaveSubscription, window, cx)
                        }

                        Page::Policy if !this.policy_candidates => {
                            this.handle(Action::SavePolicy, window, cx)
                        }
                        _ => {}
                    }
                    cx.stop_propagation();
                }
                if event.keystroke.key == "tab" {
                    this.keyboard_focus = true;
                    if event.keystroke.modifiers.shift {
                        window.focus_prev();
                    } else {
                        window.focus_next();
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .font_family("Microsoft YaHei UI")
            .text_size(px(13.))
            .text_color(rgb(INK))
            .child(sidebar)
            .child(main)
            .relative()
            .when_some(
                feedback.filter(|_| !dialog_open),
                |root, (message, color, background)| {
                    root.child(
                        div()
                            .absolute()
                            .right_6()
                            .bottom_6()
                            .max_w(px(460.))
                            .px_4()
                            .py_3()
                            .rounded_lg()
                            .shadow_md()
                            .border_1()
                            .border_color(rgb(LINE))
                            .bg(rgb(background))
                            .text_color(rgb(color))
                            .text_size(px(13.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(message))
                            .when(self.local_error.is_some(), |toast| {
                                toast.child(self.icon_button(
                                    "dismiss-toast",
                                    "icons/close.svg",
                                    "关闭提示",
                                    false,
                                    Action::DismissError,
                                    cx,
                                ))
                            }),
                    )
                },
            )
            .when_some(dialog, |root, dialog| root.child(dialog))
    }
}

fn panel() -> Div {
    div()
        .bg(rgb(0xffffff))
        .border_1()
        .border_color(rgb(LINE))
        .rounded_xl()
        .p(px(12.))
}
fn section_title(text: &str) -> Div {
    div()
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(14.))
        .child(text.to_owned())
}
fn hint(text: &str) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(text.to_owned())
}
fn pill(text: &str, color: u32) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(12.))
        .text_color(rgb(color))
        .child(div().size(px(6.)).rounded_full().bg(rgb(color)))
        .child(text.to_owned())
}
fn metric(label: &str, value: String) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child(label.to_owned()),
        )
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value),
        )
}
fn empty(icon: &'static str, title: &str, description: &str) -> Div {
    div()
        .py_6()
        .flex()
        .flex_col()
        .items_center()
        .text_center()
        .gap_2()
        .child(
            div()
                .size(px(36.))
                .mb_2()
                .rounded_xl()
                .bg(rgb(0xf5ede6))
                .flex()
                .items_center()
                .justify_center()
                .child(svg().path(icon).size(px(20.)).text_color(rgb(0xc5a68c))),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(rgb(MUTED))
                .child(title.to_owned()),
        )
        .when(!description.is_empty(), |empty| {
            empty.child(hint(description))
        })
}
