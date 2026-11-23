use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::App;

pub fn setup(app: &mut App) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出 MyEyeCare", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

    let icon = app.default_window_icon().unwrap().clone();

    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .tooltip("MyEyeCare - 护眼助手")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().0.clone();
            match id.as_str() {
                "show" => {
                    let _ = crate::show_and_clear_focus(app);
                }
                "quit" => {
                    crate::gamma::reset();
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击、双击一律只展开主界面，不再切换隐藏。
            // Windows 双击会先产生两次 Click(Up)，再产生 DoubleClick；show 本身幂等，重复调用安全。
            let should_show = match &event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => true,
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => true,
                _ => false,
            };
            if should_show {
                let app = tray.app_handle();
                let _ = crate::show_and_clear_focus(app);
            }
        })
        .build(app)?;

    Ok(())
}
