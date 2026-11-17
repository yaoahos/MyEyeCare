mod autostart;
mod display;
mod gamma;
mod overlay;
mod settings;
mod tray;

use tauri::{AppHandle, Manager};

/// 关闭窗口强制只隐藏到托盘，应用继续驻留在后台。
#[tauri::command]
fn close_main_window(app: AppHandle) -> Result<(), String> {
    app.get_webview_window("main")
        .ok_or_else(|| "找不到主窗口".to_string())?
        .hide()
        .map_err(|e| e.to_string())
}

/// 唤起并显示主窗口：用于启动检查到更新时，让托盘状态下的提醒弹窗可见。
#[tauri::command]
fn show_main_window(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到主窗口".to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

/// 绿色单文件版标识：前端据此隐藏开机自启，避免注册表写入。
#[tauri::command]
fn is_portable() -> bool {
    cfg!(feature = "portable")
}

/// 只允许打开本项目 GitHub 仓库的 https 链接，防止 IPC 被诱导执行任意程序。
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    const ALLOWED_PREFIX: &str = "https://github.com/yaoahos/MyEyeCare";
    if !url.starts_with(ALLOWED_PREFIX) {
        return Err("仅支持打开项目官方页面".to_string());
    }
    // 前缀之后只能是结束、斜杠路径或查询串，避免 `...MyEyeCare.evil.com` 绕过
    match url.as_bytes().get(ALLOWED_PREFIX.len()) {
        None | Some(b'/') | Some(b'?') | Some(b'#') => {}
        _ => return Err("仅支持打开项目官方页面".to_string()),
    }

    #[cfg(target_os = "windows")]
    {
        use std::ffi::{c_void, OsStr};
        use std::os::windows::ffi::OsStrExt;

        fn wide(value: &str) -> Vec<u16> {
            let mut buf: Vec<u16> = OsStr::new(value).encode_wide().collect();
            buf.push(0);
            buf
        }

        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(
                hwnd: *mut c_void,
                verb: *const u16,
                file: *const u16,
                params: *const u16,
                dir: *const u16,
                show: i32,
            ) -> *mut c_void;
        }

        let verb = wide("open");
        let target = wide(&url);
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result as isize > 32 {
            Ok(())
        } else {
            Err("无法打开发布页面".to_string())
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = url;
        Err("当前平台不支持打开外部链接".to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            settings::load_settings,
            settings::save_settings,
            gamma::apply_effect,
            gamma::reset_effect,
            gamma::gamma_probe,
            autostart::is_autostart_enabled,
            autostart::set_autostart,
            close_main_window,
            show_main_window,
            is_portable,
            open_external,
        ])
        .setup(|app| {
            let settings = settings::load_from(&app.handle());

            // 安装版按配置同步开机自启；首次运行时配置默认为开启。
            // 绿色版走 imp 的空实现，不写注册表。
            if let Err(e) = autostart::apply_saved_state(settings.autostart_enabled) {
                eprintln!("同步开机自启失败: {}", e);
            }

            // 先写入期望状态，再启动色罩窗口线程，避免启动时序竞态
            overlay::init();
            // 恢复上次的护眼效果（gamma + 色罩），并保存进程内快照
            gamma::apply(&settings);
            // 监听息屏唤醒与显示变化，自动重应用上面的配置
            display::init();

            tray::setup(app)?;

            if let Some(window) = app.get_webview_window("main") {
                if !settings.start_minimized {
                    let _ = window.show();
                    let _ = window.set_focus();
                }

                let window_for_event = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        // 强制驻留托盘：阻止窗口销毁，仅隐藏
                        api.prevent_close();
                        let _ = window_for_event.hide();
                    }
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build MyEyeCare");

    app.run(|_app, event| {
        // 无论从托盘还是窗口退出，都恢复原始 gamma 并关闭色罩
        if let tauri::RunEvent::Exit = event {
            display::shutdown();
            gamma::reset();
            overlay::shutdown();
        }
    });
}
