//! Windows 系统托盘边界。

use crate::PlatformError;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const TRAY_OPEN_ID: &str = "floatile.tray.open";
const TRAY_EXIT_ID: &str = "floatile.tray.exit";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsTrayEvent {
    Open,
    Exit,
}

/// 托盘图标必须在 Win32 消息循环所在线程创建和轮询。
pub struct WindowsTrayIcon {
    _tray: TrayIcon,
    open_id: MenuId,
    exit_id: MenuId,
}

impl WindowsTrayIcon {
    pub fn install(tooltip: &str) -> Result<Self, PlatformError> {
        let menu = Menu::new();
        let open = MenuItem::with_id(TRAY_OPEN_ID, "打开 Floatile", true, None);
        let exit = MenuItem::with_id(TRAY_EXIT_ID, "退出 Floatile", true, None);
        menu.append_items(&[&open, &exit])
            .map_err(|error| PlatformError::Platform(format!("tray menu failed: {error}")))?;

        let tray = TrayIconBuilder::new()
            .with_tooltip(tooltip)
            .with_icon(floatile_icon()?)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(true)
            .build()
            .map_err(|error| PlatformError::Platform(format!("tray icon failed: {error}")))?;

        Ok(Self {
            _tray: tray,
            open_id: open.id().clone(),
            exit_id: exit.id().clone(),
        })
    }

    /// 非阻塞排空库的全局托盘事件队列。
    pub fn poll_event(&self) -> Option<WindowsTrayEvent> {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.open_id {
                return Some(WindowsTrayEvent::Open);
            }
            if event.id == self.exit_id {
                return Some(WindowsTrayEvent::Exit);
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                return Some(WindowsTrayEvent::Open);
            }
        }
        None
    }
}

fn floatile_icon() -> Result<Icon, PlatformError> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let offset = ((y * SIZE + x) * 4) as usize;
            let inside = (3..29).contains(&x) && (3..29).contains(&y);
            let letter_f = (8..12).contains(&x) && (7..25).contains(&y)
                || (8..23).contains(&x) && (7..11).contains(&y)
                || (8..20).contains(&x) && (14..18).contains(&y);
            let color = if letter_f {
                [255, 255, 255, 255]
            } else if inside {
                [48, 112, 232, 255]
            } else {
                [0, 0, 0, 0]
            };
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE)
        .map_err(|error| PlatformError::Platform(format!("tray icon pixels invalid: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tray_icon_is_valid() {
        assert!(floatile_icon().is_ok());
    }
}
