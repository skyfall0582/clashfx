use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

pub struct Assets;

const ICONS: &[(&str, &[u8])] = &[
    (
        "icons/arrow-right.svg",
        include_bytes!("../../../assets/icons/arrow-right.svg"),
    ),
    (
        "icons/transfer.svg",
        include_bytes!("../../../assets/icons/transfer.svg"),
    ),
    (
        "icons/clock.svg",
        include_bytes!("../../../assets/icons/clock.svg"),
    ),
    (
        "icons/warning.svg",
        include_bytes!("../../../assets/icons/warning.svg"),
    ),
    (
        "icons/smart.svg",
        include_bytes!("../../../assets/icons/smart.svg"),
    ),
    (
        "icons/back.svg",
        include_bytes!("../../../assets/icons/back.svg"),
    ),
    (
        "icons/copy.svg",
        include_bytes!("../../../assets/icons/copy.svg"),
    ),
    (
        "icons/edit.svg",
        include_bytes!("../../../assets/icons/edit.svg"),
    ),
    (
        "icons/refresh.svg",
        include_bytes!("../../../assets/icons/refresh.svg"),
    ),
    (
        "icons/trash.svg",
        include_bytes!("../../../assets/icons/trash.svg"),
    ),
    (
        "icons/import.svg",
        include_bytes!("../../../assets/icons/import.svg"),
    ),
    (
        "icons/file.svg",
        include_bytes!("../../../assets/icons/file.svg"),
    ),
    (
        "icons/close.svg",
        include_bytes!("../../../assets/icons/close.svg"),
    ),
    (
        "icons/add.svg",
        include_bytes!("../../../assets/icons/add.svg"),
    ),
    (
        "icons/remove.svg",
        include_bytes!("../../../assets/icons/remove.svg"),
    ),
    (
        "icons/pause.svg",
        include_bytes!("../../../assets/icons/pause.svg"),
    ),
    (
        "icons/chevron-right.svg",
        include_bytes!("../../../assets/icons/chevron-right.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../../../assets/icons/chevron-down.svg"),
    ),
    (
        "icons/nodes.svg",
        include_bytes!("../../../assets/icons/nodes.svg"),
    ),
    (
        "icons/grip.svg",
        include_bytes!("../../../assets/icons/grip.svg"),
    ),
    (
        "icons/check.svg",
        include_bytes!("../../../assets/icons/check.svg"),
    ),
    (
        "icons/overview.svg",
        include_bytes!("../../../assets/icons/overview.svg"),
    ),
    (
        "icons/subscriptions.svg",
        include_bytes!("../../../assets/icons/subscriptions.svg"),
    ),
    (
        "icons/switch.svg",
        include_bytes!("../../../assets/icons/switch.svg"),
    ),
    (
        "icons/logs.svg",
        include_bytes!("../../../assets/icons/logs.svg"),
    ),
    (
        "icons/settings.svg",
        include_bytes!("../../../assets/icons/settings.svg"),
    ),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| (*name).into())
            .collect())
    }
}
