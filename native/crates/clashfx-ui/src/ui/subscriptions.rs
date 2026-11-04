use super::*;
use clashfx_core::model::Subscription as ProfileSubscription;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct SubscriptionDraft {
    name: String,
    url: String,
    notes: String,
}

impl From<&ProfileSubscription> for SubscriptionDraft {
    fn from(sub: &ProfileSubscription) -> Self {
        Self {
            name: sub.name.clone(),
            url: sub.url.clone().unwrap_or_default(),
            notes: sub.notes.clone(),
        }
    }
}

pub(super) fn command_feedback(command: &Command) -> &'static str {
    match command {
        Command::Start | Command::Stop | Command::SelectNode(_) | Command::SystemProxy(_) => "",
        Command::Probe => "正在检测节点",
        Command::TestNodes(_) => "已开始测速",
        Command::UpdateSubscription(_) => "订阅已更新，保留切换池",
        Command::MovePoolSubscriptionTo { .. } | Command::MovePoolNodeTo { .. } => "优先顺序已保存",
        Command::SetPoolNodes { in_pool: true, .. } => "已加入切换池",
        Command::SetPoolNodes { in_pool: false, .. } => "已移出切换池",
        Command::ClearLogs | Command::ClearLogSource { .. } => "日志已清空",
        _ => "设置已保存",
    }
}

impl ClashFx {
    pub(super) fn show_notice(&mut self, message: &str) {
        if message.is_empty() {
            self.notice = None;
            self.notice_until = None;
            return;
        }
        self.notice = Some(message.to_owned());
        self.notice_until = Some(std::time::Instant::now() + Duration::from_secs(5));
    }

    pub(super) fn submit(&mut self, command: Command, message: &str, cx: &mut Context<Self>) {
        if self.pending || self.exiting {
            return;
        }
        self.pending = true;
        for (key, input) in &self.inputs {
            if !matches!(
                *key,
                "subscription_search"
                    | "node_search"
                    | "node_subscription_search"
                    | "log_filter"
                    | "pool_available_search"
                    | "pool_member_search"
            ) {
                input.update(cx, |input, cx| input.set_read_only(true, cx));
            }
        }
        self.local_error = None;
        self.notice = None;
        let response = self.backend.request(command);
        let message = message.to_owned();
        cx.spawn(async move |this, cx| {
            let result = response.await;
            this.update(cx, |this, cx| {
                this.pending = false;
                for input in this.inputs.values() {
                    input.update(cx, |input, cx| input.set_read_only(false, cx));
                }
                this.sync_snapshot(this.backend.snapshot(), cx);
                match result {
                    Ok(()) => {
                        this.show_notice(&message);
                        if this
                            .snapshot
                            .config
                            .subscriptions
                            .iter()
                            .find(|sub| Some(&sub.id) == this.selected_subscription.as_ref())
                            .is_some_and(|sub| {
                                this.current_draft(cx) == SubscriptionDraft::from(sub)
                            })
                        {
                            this.editing_subscription = false;
                        }
                    }
                    Err(error) => this.report_error(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn sync_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        if snapshot.last_error != self.snapshot.last_error
            && let Some(error) = &snapshot.last_error
        {
            self.report_error(error.clone(), cx);
        }
        let state = network::NetworkState::from_snapshot(&snapshot);
        self.network_view.update(cx, |view, cx| {
            view.set_state(state, cx);
        });
        let added = snapshot
            .config
            .subscriptions
            .iter()
            .find(|sub| {
                !self
                    .snapshot
                    .config
                    .subscriptions
                    .iter()
                    .any(|old| old.id == sub.id)
            })
            .map(|sub| sub.id.clone());
        self.snapshot = Arc::new(snapshot);
        self.drafts.retain(|id, _| {
            self.snapshot
                .config
                .subscriptions
                .iter()
                .any(|sub| &sub.id == id)
        });
        if let Some(id) = added {
            if !self.snapshot.running && !self.snapshot.nodes.is_empty() {
                self.backend.send(Command::Start);
            }
            self.expanded_available.insert(id.clone());
            self.select_subscription(id, cx);
            self.show_import = false;
            for key in [
                "import_name",
                "import_url",
                "import_notes",
                "subscription_search",
            ] {
                self.inputs[key].update(cx, |input, cx| input.set_text("", cx));
            }
        }
        if self.selected_subscription.as_ref().is_none_or(|id| {
            !self
                .snapshot
                .config
                .subscriptions
                .iter()
                .any(|sub| &sub.id == id)
        }) {
            if let Some(id) = self
                .snapshot
                .config
                .subscriptions
                .first()
                .map(|sub| sub.id.clone())
            {
                self.select_subscription(id, cx);
            } else {
                self.selected_subscription = None;
                self.editing_subscription = false;
            }
        }
        cx.notify();
    }

    fn current_draft(&self, cx: &App) -> SubscriptionDraft {
        SubscriptionDraft {
            name: self.text("sub_name", cx).trim().to_owned(),
            url: self.text("sub_url", cx).trim().to_owned(),
            notes: self.text("sub_notes", cx),
        }
    }

    fn load_draft(&mut self, draft: SubscriptionDraft, cx: &mut Context<Self>) {
        for (key, text) in [
            ("sub_name", draft.name),
            ("sub_url", draft.url),
            ("sub_notes", draft.notes),
        ] {
            self.inputs[key].update(cx, |input, cx| input.set_text(text, cx));
        }
    }

    pub(super) fn select_subscription(&mut self, id: String, cx: &mut Context<Self>) {
        if self.selected_subscription.as_ref() == Some(&id) {
            return;
        }
        if let Some(old) = self.selected_subscription.clone()
            && self
                .snapshot
                .config
                .subscriptions
                .iter()
                .any(|sub| sub.id == old)
        {
            self.drafts.insert(old, self.current_draft(cx));
        }
        if let Some(sub) = self
            .snapshot
            .config
            .subscriptions
            .iter()
            .find(|sub| sub.id == id)
        {
            let draft = self.drafts.get(&id).cloned().unwrap_or_else(|| sub.into());
            self.editing_subscription =
                self.page == Page::Subscriptions && draft != SubscriptionDraft::from(sub);
            self.load_draft(draft, cx);
            self.selected_subscription = Some(id);
            self.remove_confirmation = None;
            self.inputs["node_search"].update(cx, |input, cx| input.set_text("", cx));
        }
        cx.notify();
    }

    pub(super) fn reset_subscription(&mut self, cx: &mut Context<Self>) {
        self.editing_subscription = false;
        if let Some(sub) = self
            .snapshot
            .config
            .subscriptions
            .iter()
            .find(|sub| Some(&sub.id) == self.selected_subscription.as_ref())
        {
            let draft = sub.into();
            self.drafts.remove(&sub.id);
            self.load_draft(draft, cx);
        }
    }
}

pub(super) fn source_preview(source: &str) -> String {
    if let Some((_, rest)) = source.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = authority.rsplit('@').next().unwrap_or_default();
        format!("{host}/…")
    } else {
        source
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(source)
            .to_owned()
    }
}

pub(super) fn wrap_source(value: &str) -> String {
    let mut result = String::new();
    for (index, ch) in value.chars().enumerate() {
        result.push(ch);
        if matches!(ch, '/' | '?' | '&' | '=' | '\\') || index % 40 == 39 {
            result.push('\u{200b}');
        }
    }
    result
}
