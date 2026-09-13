//! 托盘常驻：图标 + 菜单（打开 / 重新扫描应用 / 退出）。
//! tray-icon 在 Windows 上要求创建线程自己泵消息，本线程同时把
//! 菜单事件与左键点击转发给 iced runtime（复用 events 桥通道）。

use iced::futures::channel::mpsc::UnboundedSender;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIconBuilder, TrayIconEvent};

use super::{plog, Message};

#[allow(unused_imports)]
use tray_icon::TrayIcon;

pub fn spawn(tx: UnboundedSender<Message>, icon_png: &'static [u8]) {
    std::thread::spawn(move || {
        // Kite 图标 PNG → RGBA（32×32：托盘就是小尺寸，128 给系统缩会糊）
        let img = match image::load_from_memory(icon_png) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                plog(&format!("tray icon decode failed: {e}"));
                return;
            }
        };
        let (w, h) = img.dimensions();
        let icon = match tray_icon::Icon::from_rgba(img.into_raw(), w, h) {
            Ok(icon) => icon,
            Err(e) => {
                plog(&format!("tray icon create failed: {e}"));
                return;
            }
        };

        let menu = Menu::new();
        let open_item = MenuItem::new("打开 (Alt+Space)", true, None);
        let settings_item = MenuItem::new("设置", true, None);
        let rescan_item = MenuItem::new("重新扫描应用", true, None);
        let quit_item = MenuItem::new("退出", true, None);
        if let Err(e) = menu.append_items(&[&open_item, &settings_item, &rescan_item, &quit_item]) {
            plog(&format!("tray menu build failed: {e}"));
            return;
        }

        let _tray = match TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Kite")
            .with_icon(icon)
            .build()
        {
            Ok(tray) => tray,
            Err(e) => {
                plog(&format!("tray build failed: {e}"));
                return;
            }
        };
        plog("tray created");

        // 泵消息；muda 在 DispatchMessage 里把菜单事件推入全局通道
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, TranslateMessage, MSG,
        };
        let mut msg = MSG::default();
        loop {
            unsafe {
                if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                if ev.id == open_item.id() {
                    let _ = tx.unbounded_send(Message::Hotkey(std::time::Instant::now()));
                } else if ev.id == settings_item.id() {
                    let _ = tx.unbounded_send(Message::OpenSettings);
                } else if ev.id == rescan_item.id() {
                    let _ = tx.unbounded_send(Message::Rescan);
                } else if ev.id == quit_item.id() {
                    let _ = tx.unbounded_send(Message::Quit);
                }
            }
            while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                } = ev
                {
                    let _ = tx.unbounded_send(Message::Hotkey(std::time::Instant::now()));
                }
            }
        }
    });
}
