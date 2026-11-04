#![cfg(windows)]

use anyhow::{Context, Result};
use gpui::{App, Timer, WeakEntity, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::Cell, path::Path, ptr::null_mut, sync::mpsc, thread::JoinHandle, time::Duration};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

const CALLBACK: u32 = WM_APP + 1;
const OPEN: u32 = WM_APP + 2;
const STATUS_CHANGED: u32 = WM_APP + 3;
const KEY_SELECT: u32 = NIN_SELECT | NINF_KEY;
const SHOW_ITEM: usize = 1;
const QUIT_ITEM: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Off,
    On,
    Busy,
}

impl Status {
    pub(crate) fn from_snapshot(snapshot: &clashfx_core::model::Snapshot) -> Self {
        if !snapshot.running {
            Self::Off
        } else if snapshot.testing_nodes
            || snapshot.switching
            || snapshot.probing
            || !snapshot.checking_nodes.is_empty()
            || snapshot.retry_at.is_some()
        {
            Self::Busy
        } else if snapshot.system_proxy {
            Self::On
        } else {
            Self::Off
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn title(path: &Path) -> Vec<u16> {
    wide(&format!("ClashFx.Tray:{}", path.display()).to_lowercase())
}

pub(crate) fn show_existing(path: &Path) -> bool {
    unsafe {
        let host = FindWindowW(wide("ClashFx.TrayHost").as_ptr(), title(path).as_ptr());
        !host.is_null() && PostMessageW(host, OPEN, 0, 0) != 0
    }
}

pub(crate) fn installation_guard() -> Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name = wide("Local\\ClashFx.Installer.9981d37b-7783-4723-ae53-477b3371b5ef");
    let handle = unsafe { CreateMutexW(null_mut(), 0, name.as_ptr()) };
    anyhow::ensure!(!handle.is_null(), "无法建立安装程序运行保护");
    Ok(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) })
}

#[derive(Clone, Copy)]
enum Event {
    Show,
    Quit,
}
struct Host {
    hwnd: isize,
    status: Status,
    events: mpsc::Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}
struct State {
    sender: mpsc::Sender<Event>,
    icon: Cell<NOTIFYICONDATAW>,
    icons: [HICON; 3],
    taskbar_created: u32,
}

impl Host {
    fn set_status(&mut self, status: Status) {
        if self.status != status
            && unsafe { PostMessageW(self.hwnd as HWND, STATUS_CHANGED, status as usize, 0) } != 0
        {
            self.status = status;
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

pub(crate) fn install(
    window: &mut Window,
    cx: &mut App,
    path: &Path,
    view: WeakEntity<crate::ui::ClashFx>,
) -> Result<()> {
    let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("窗口句柄不可用：{error}"))?
        .as_raw()
    else {
        anyhow::bail!("Windows 窗口不可用");
    };
    let hwnd = handle.hwnd.get();
    let (sender, events) = mpsc::channel();
    let (ready, initialized) = mpsc::sync_channel(1);
    let title = title(path);
    let thread = std::thread::Builder::new()
        .name("clashfx-tray".into())
        .spawn(move || unsafe {
            run_host(sender, ready, title);
        })?;
    let host = match initialized.recv().context("系统托盘未响应")? {
        Ok(hwnd) => Host {
            hwnd,
            status: Status::Off,
            events,
            thread: Some(thread),
        },
        Err(message) => {
            thread.join().ok();
            anyhow::bail!(message);
        }
    };
    window.on_window_should_close(cx, move |_, _| {
        unsafe {
            ShowWindow(hwnd as HWND, SW_HIDE);
        }
        false
    });
    cx.spawn(async move |cx| {
        let mut host = host;
        loop {
            Timer::after(Duration::from_millis(100)).await;
            let Ok(status) = view.read_with(cx, |view, _| view.tray_status()) else {
                return;
            };
            host.set_status(status);
            while let Ok(event) = host.events.try_recv() {
                if cx
                    .update(|cx| match event {
                        Event::Show => unsafe {
                            ShowWindow(hwnd as HWND, SW_RESTORE);
                            SetForegroundWindow(hwnd as HWND);
                            cx.activate(true);
                        },
                        Event::Quit => {
                            view.update(cx, |view, cx| view.quit(cx)).ok();
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
        }
    })
    .detach();
    Ok(())
}

unsafe fn run_host(
    sender: mpsc::Sender<Event>,
    ready: mpsc::SyncSender<Result<isize, String>>,
    title: Vec<u16>,
) {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let icons = [2usize, 3, 4].map(|id| {
            LoadImageW(
                instance,
                id as _,
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_SHARED,
            ) as HICON
        });
        if icons.iter().any(|icon| icon.is_null()) {
            ready.send(Err("无法加载系统托盘状态图标".into())).ok();
            return;
        }
        let class_name = wide("ClashFx.TrayHost");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&class) == 0 {
            ready.send(Err("无法注册系统托盘窗口".into())).ok();
            return;
        }
        let mut state = Box::new(State {
            sender,
            icon: Cell::new(std::mem::zeroed()),
            icons,
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            (&mut *state as *mut State).cast(),
        );
        if hwnd.is_null() {
            ready.send(Err("无法创建系统托盘窗口".into())).ok();
            return;
        }
        let mut icon = state.icon.get();
        icon.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        icon.hWnd = hwnd;
        icon.uID = 1;
        icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        icon.uCallbackMessage = CALLBACK;
        icon.hIcon = state.icons[Status::Off as usize];
        let tip = wide("ClashFx · 点击打开，右键退出");
        icon.szTip[..tip.len()].copy_from_slice(&tip);
        state.icon.set(icon);
        if icon.hIcon.is_null() || Shell_NotifyIconW(NIM_ADD, &icon) == 0 {
            ready
                .send(Err("无法显示系统托盘图标；关闭窗口将退出应用".into()))
                .ok();
            DestroyWindow(hwnd);
            return;
        }
        icon.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        state.icon.set(icon);
        Shell_NotifyIconW(NIM_SETVERSION, &icon);
        ready.send(Ok(hwnd as isize)).ok();
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnregisterClassW(class_name.as_ptr(), instance);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if !pointer.is_null() {
            let state = &*pointer;
            if message == STATUS_CHANGED {
                if let Some(handle) = state.icons.get(wparam) {
                    let mut icon = state.icon.get();
                    icon.hIcon = *handle;
                    state.icon.set(icon);
                    Shell_NotifyIconW(NIM_MODIFY, &icon);
                }
                return 0;
            }
            if state.taskbar_created != 0 && message == state.taskbar_created {
                let icon = state.icon.get();
                Shell_NotifyIconW(NIM_ADD, &icon);
                Shell_NotifyIconW(NIM_SETVERSION, &icon);
                return 0;
            }
            match message {
                OPEN => {
                    state.sender.send(Event::Show).ok();
                    return 0;
                }
                CALLBACK => {
                    match lparam as u32 & 0xffff {
                        NIN_SELECT | KEY_SELECT | WM_LBUTTONDBLCLK => {
                            state.sender.send(Event::Show).ok();
                        }
                        WM_CONTEXTMENU | WM_RBUTTONUP => {
                            let menu = CreatePopupMenu();
                            if !menu.is_null() {
                                AppendMenuW(
                                    menu,
                                    MF_STRING,
                                    SHOW_ITEM,
                                    wide("显示 ClashFx").as_ptr(),
                                );
                                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                                AppendMenuW(menu, MF_STRING, QUIT_ITEM, wide("退出").as_ptr());
                                let mut point: POINT = std::mem::zeroed();
                                GetCursorPos(&mut point);
                                SetForegroundWindow(hwnd);
                                let selected = TrackPopupMenu(
                                    menu,
                                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                                    point.x,
                                    point.y,
                                    0,
                                    hwnd,
                                    std::ptr::null(),
                                );
                                DestroyMenu(menu);
                                PostMessageW(hwnd, WM_NULL, 0, 0);
                                match selected as usize {
                                    SHOW_ITEM => {
                                        state.sender.send(Event::Show).ok();
                                    }
                                    QUIT_ITEM => {
                                        state.sender.send(Event::Quit).ok();
                                    }
                                    _ => {
                                        Shell_NotifyIconW(NIM_SETFOCUS, &state.icon.get());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    return 0;
                }
                WM_DESTROY => {
                    Shell_NotifyIconW(NIM_DELETE, &state.icon.get());
                    PostQuitMessage(0);
                    return 0;
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}
