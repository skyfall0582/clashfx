#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod assets;
mod input;
mod tray;
mod ui;

use clashfx_core::{
    engine::{Backend, Command},
    store::Store,
};
use gpui::*;
use std::path::PathBuf;

fn launch() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut data_dir = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => {
                data_dir =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--data-dir requires a path")
                    })?));
            }
            "--version" => {
                return Ok(());
            }
            _ => anyhow::bail!("Unknown argument: {arg}"),
        }
    }
    let store = Store::open(data_dir)?;
    #[cfg(windows)]
    if tray::show_existing(&store.root) {
        return Ok(());
    }
    #[cfg(windows)]
    let _installation_guard = tray::installation_guard()?;
    let tray_path = store.root.clone();
    let backend = Backend::launch(store)?;
    if !backend.snapshot().nodes.is_empty() {
        backend.send(Command::Start);
    }
    let ui_backend = backend.clone();
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx| {
            input::init(cx);
            ui::dialogs::install(cx);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1000.), px(680.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(860.), px(600.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("ClashFx".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |window, cx| {
                    window.set_rem_size(px(14.));
                    let view = cx.new(|cx| ui::ClashFx::new(ui_backend, cx));
                    #[cfg(windows)]
                    if let Err(error) = tray::install(window, cx, &tray_path, view.downgrade()) {
                        view.update(cx, |view, cx| view.report_error(error.to_string(), cx));
                    }
                    view
                },
            )
            .expect("open ClashFx window");
            cx.activate(true);
        });
    backend.shutdown();
    Ok(())
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--proxy-guard") {
        std::process::exit(i32::from(
            clashfx_core::sysproxy::run_guard_process().is_err(),
        ));
    }
    if let Err(error) = launch() {
        rfd::MessageDialog::new()
            .set_title("ClashFx")
            .set_description(format!("无法启动：{error:#}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}
