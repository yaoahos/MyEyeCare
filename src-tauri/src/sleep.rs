//! 息屏控制：空闲多久后自动息屏 + 全局快捷键 Ctrl+Enter 立即息屏。
//!
//! - 空闲检测：GetLastInputInfo 取最后一次键鼠输入距离现在的毫秒数；
//! - 自动息屏：隐藏窗口上挂 5 秒 WM_TIMER，超阈值则发 SC_MONITORPOWER，
//!   关屏后先“解除布防”，通过 GetLastInputInfo 的输入时刻变化检测新键鼠输入才重新计时，
//!   避免每 5 秒重复关屏；
//! - 快捷键：RegisterHotKey 注册全局 Ctrl+Enter，收到 WM_HOTKEY 立即息屏；
//! - 息屏动作：直接对系统外壳窗口调用 DefWindowProcW(WM_SYSCOMMAND,
//!   SC_MONITORPOWER, 2)。HWND_BROADCAST 广播在新版 Windows 上常被应用窗口
//!   拦截而完全无效，这里只在没有可用外壳窗口时才退回广播兜底。
//! - 锁屏选项：开启时先 LockWorkStation()，短暂延迟后再关屏，避免锁屏界面点亮屏幕。
//! - 本模块不读取也不修改 Windows 自带的“关闭屏幕”超时设置；两套机制独立，
//!   任意一套先到时间都会关屏。空闲时长设为 0 即完全沿用系统设置。

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use serde::Serialize;

#[cfg(target_os = "windows")]
mod win {
    use std::ffi::{c_int, c_void};

    pub type Hwnd = *mut c_void;
    pub type Atom = u16;
    pub type WndProc = Option<extern "system" fn(Hwnd, u32, usize, isize) -> isize>;

    #[repr(C)]
    pub struct WndClassExW {
        pub cb_size: u32,
        pub style: u32,
        pub lpfn_wnd_proc: WndProc,
        pub cb_cls_extra: c_int,
        pub cb_wnd_extra: c_int,
        pub h_instance: Hwnd,
        pub h_icon: Hwnd,
        pub h_cursor: Hwnd,
        pub hbr_background: Hwnd,
        pub lpsz_menu_name: *const u16,
        pub lpsz_class_name: *const u16,
        pub h_icon_sm: Hwnd,
    }

    #[repr(C)]
    pub struct Msg {
        pub hwnd: Hwnd,
        pub message: u32,
        pub w_param: usize,
        pub l_param: isize,
        pub time: u32,
        pub pt_x: i32,
        pub pt_y: i32,
        pub l_private: u32,
    }

    #[repr(C)]
    pub struct LastInputInfo {
        pub cb_size: u32,
        pub dw_time: u32,
    }

    pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    pub const WS_POPUP: u32 = 0x8000_0000;
    pub const WM_QUIT: u32 = 0x0012;
    pub const WM_TIMER: u32 = 0x0113;
    pub const WM_HOTKEY: u32 = 0x0312;
    pub const WM_SYSCOMMAND: u32 = 0x0112;
    pub const SC_MONITORPOWER: usize = 0xF170;
    pub const WM_APP: u32 = 0x8000;
    /// 由主线程投递到隐藏窗口线程：重试注册热键 / 触发息屏。
    pub const WM_MEC_RECONFIG: u32 = WM_APP + 6;
    pub const WM_MEC_SLEEP_NOW: u32 = WM_APP + 7;
    pub const TIMER_IDLE: usize = 6;
    /// 勾选锁屏时：先锁定工作站，再延迟关屏，避免锁屏界面把屏幕点亮。
    pub const LOCK_SLEEP_DELAY_MS: u64 = 800;
    pub const HOTKEY_ID: usize = 1;
    pub const IDLE_TIMER_MS: u32 = 5_000;
    pub const SMTO_ABORTIFHUNG: u32 = 0x0002;

    // 固定全局快捷键 Ctrl+Enter（Win32 MOD_CONTROL + VK_RETURN）
    pub const MOD_CONTROL: u32 = 0x0002;
    pub const MOD_NOREPEAT: u32 = 0x4000;
    pub const VK_RETURN: u32 = 0x0D;

    #[link(name = "user32")]
    extern "system" {
        pub fn RegisterClassExW(wc: *const WndClassExW) -> Atom;
        pub fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: Hwnd,
            menu: Hwnd,
            instance: Hwnd,
            param: Hwnd,
        ) -> Hwnd;
        pub fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> c_int;
        pub fn TranslateMessage(msg: *const Msg) -> c_int;
        pub fn DispatchMessageW(msg: *const Msg) -> isize;
        pub fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        pub fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> c_int;
        pub fn SetTimer(hwnd: Hwnd, id: usize, elapse: u32, timer_proc: Hwnd) -> usize;
        pub fn PostQuitMessage(exit_code: c_int);
        pub fn GetModuleHandleW(module_name: *const u16) -> Hwnd;
        pub fn GetLastInputInfo(plii: *mut LastInputInfo) -> c_int;
        pub fn GetTickCount() -> u32;
        pub fn RegisterHotKey(hwnd: Hwnd, id: c_int, fs_modifiers: u32, vk: u32) -> c_int;
        pub fn UnregisterHotKey(hwnd: Hwnd, id: c_int) -> c_int;
        pub fn GetShellWindow() -> Hwnd;
        pub fn GetForegroundWindow() -> Hwnd;
        pub fn LockWorkStation() -> c_int;
        pub fn SendMessageTimeoutW(
            hwnd: Hwnd,
            msg: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }

    pub const HWND_BROADCAST: Hwnd = 0xFFFF_FFFF as Hwnd;
}

#[cfg(target_os = "windows")]
static WATCHER_HWND: AtomicIsize = AtomicIsize::new(0);
// 空闲阈值（分钟），由主线程写入、窗口线程读取。
#[cfg(target_os = "windows")]
static IDLE_MINUTES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// 关屏后解除布防；检测到新的键鼠输入才重新允许自动息屏。
static IDLE_ARMED: AtomicBool = AtomicBool::new(true);
/// 息屏时是否一并锁定 Windows。
static LOCK_ON_SLEEP: AtomicBool = AtomicBool::new(false);
/// 上一轮定时器看到的“最后一次输入时刻”。两次 tick 之间只要这个值变了，
/// 就说明用户动过键鼠；比用“空闲毫秒是否低于固定阈值”更可靠。
static LAST_INPUT_TICK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// 固定热键是否注册成功，由隐藏窗口线程写入。
static HOTKEY_OK: AtomicBool = AtomicBool::new(false);
/// 窗口线程是否已完成第一次注册尝试，避免前端启动时误读初始值 false。
static HOTKEY_CHECKED: AtomicBool = AtomicBool::new(false);

/// 返回（空闲毫秒数，最后一次输入的 tick）。后者用于跨 tick 判断是否有新输入。
#[cfg(target_os = "windows")]
fn input_snapshot() -> (u32, u32) {
    unsafe {
        let mut info = win::LastInputInfo { cb_size: 0, dw_time: 0 };
        info.cb_size = std::mem::size_of::<win::LastInputInfo>() as u32;
        if win::GetLastInputInfo(&mut info) == 0 {
            return (0, 0);
        }
        (
            win::GetTickCount().wrapping_sub(info.dw_time),
            info.dw_time,
        )
    }
}

/// 让显示器进入息屏（SC_MONITORPOWER, 2 = 关闭）。
///
/// 关键：`SendMessage(HWND_BROADCAST, ...)` 发的 WM_SYSCOMMAND 会被应用窗口
/// 的窗口过程吞掉，Win10/11 上经常完全不生效。这里直接调用 DefWindowProcW，
/// 让系统自己处理这条系统命令，不经过目标窗口的自定义过程。
#[cfg(target_os = "windows")]
fn turn_off_display() -> bool {
    unsafe {
        // 外壳窗口（explorer.exe）几乎总是存在，且和普通程序同权限等级。
        let shell = win::GetShellWindow();
        if !shell.is_null() {
            win::DefWindowProcW(shell, win::WM_SYSCOMMAND, win::SC_MONITORPOWER, 2);
            return true;
        }
        // 兜底 1：当前前台窗口（同样走系统默认过程，不经过其自定义过程）。
        let foreground = win::GetForegroundWindow();
        if !foreground.is_null() {
            win::DefWindowProcW(foreground, win::WM_SYSCOMMAND, win::SC_MONITORPOWER, 2);
            return true;
        }
        // 兜底 2：没有可用窗口时才发广播，并加超时避免卡死监听线程。
        let mut result: usize = 0;
        win::SendMessageTimeoutW(
            win::HWND_BROADCAST,
            win::WM_SYSCOMMAND,
            win::SC_MONITORPOWER,
            2,
            win::SMTO_ABORTIFHUNG,
            1_000,
            &mut result,
        );
        true
    }
}

#[cfg(not(target_os = "windows"))]
fn turn_off_display() -> bool {
    false
}

/// 所有息屏入口的统一动作：按选项决定是否先锁定工作站，再关屏。
#[cfg(target_os = "windows")]
fn perform_sleep() {
    IDLE_ARMED.store(false, Ordering::SeqCst);
    let locked = LOCK_ON_SLEEP.load(Ordering::SeqCst) && unsafe { win::LockWorkStation() } != 0;
    if locked {
        // 锁屏界面出现会点亮屏幕，等它稳定后再发关屏指令。用独立线程而不是
        // 窗口定时器：锁定后窗口定时器可能要等解锁才触发，反而会误关屏。
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(win::LOCK_SLEEP_DELAY_MS));
            turn_off_display();
        });
    } else {
        turn_off_display();
    }
}

/// 注册/重注册固定全局热键 Ctrl+Enter（先注销旧的）。
#[cfg(target_os = "windows")]
unsafe fn register_hotkey(hwnd: win::Hwnd) -> bool {
    win::UnregisterHotKey(hwnd, win::HOTKEY_ID as i32);
    let ok = win::RegisterHotKey(
        hwnd,
        win::HOTKEY_ID as i32,
        win::MOD_CONTROL | win::MOD_NOREPEAT,
        win::VK_RETURN,
    ) != 0;
    HOTKEY_OK.store(ok, Ordering::SeqCst);
    HOTKEY_CHECKED.store(true, Ordering::SeqCst);
    ok
}

#[cfg(target_os = "windows")]
extern "system" fn sleep_proc(
    hwnd: win::Hwnd,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    unsafe {
        match msg {
            win::WM_HOTKEY => {
                if wparam == win::HOTKEY_ID {
                    perform_sleep();
                }
                0
            }
            win::WM_TIMER => {
                if wparam == win::TIMER_IDLE {
                    let (idle, tick) = input_snapshot();
                    let prev_tick = LAST_INPUT_TICK.swap(tick, Ordering::SeqCst);
                    if !IDLE_ARMED.load(Ordering::SeqCst) {
                        // 屏幕已关：只有“最后输入时刻”发生变化才重新布防。
                        if prev_tick != 0 && tick != 0 && tick != prev_tick {
                            IDLE_ARMED.store(true, Ordering::SeqCst);
                        }
                        return 0;
                    }
                    let minutes = IDLE_MINUTES.load(Ordering::SeqCst);
                    if minutes > 0 && idle >= minutes.saturating_mul(60_000) {
                        perform_sleep();
                    }
                }
                0
            }
            // RegisterHotKey 必须由拥有隐藏窗口的线程调用；主线程只投递消息。
            win::WM_MEC_RECONFIG => {
                register_hotkey(hwnd);
                0
            }
            win::WM_MEC_SLEEP_NOW => {
                perform_sleep();
                0
            }
            win::WM_QUIT => {
                win::PostQuitMessage(0);
                0
            }
            _ => win::DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(target_os = "windows")]
fn sleep_thread() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    unsafe {
        let instance = win::GetModuleHandleW(std::ptr::null());
        let class_name: Vec<u16> = OsStr::new("MyEyeCareSleepClass\0").encode_wide().collect();
        let wc = win::WndClassExW {
            cb_size: std::mem::size_of::<win::WndClassExW>() as u32,
            style: 0,
            lpfn_wnd_proc: Some(sleep_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: instance,
            h_icon: std::ptr::null_mut(),
            h_cursor: std::ptr::null_mut(),
            hbr_background: std::ptr::null_mut(),
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: class_name.as_ptr(),
            h_icon_sm: std::ptr::null_mut(),
        };
        win::RegisterClassExW(&wc);
        let hwnd = win::CreateWindowExW(
            win::WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            std::ptr::null(),
            win::WS_POPUP,
            0, 0, 0, 0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        WATCHER_HWND.store(hwnd as isize, Ordering::SeqCst);
        let _ = register_hotkey(hwnd);
        win::SetTimer(hwnd, win::TIMER_IDLE, win::IDLE_TIMER_MS, std::ptr::null_mut());

        let mut msg: win::Msg = std::mem::zeroed();
        while win::GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.message == win::WM_QUIT {
                break;
            }
            win::TranslateMessage(&msg);
            win::DispatchMessageW(&msg);
        }
        win::UnregisterHotKey(hwnd, win::HOTKEY_ID as i32);
        WATCHER_HWND.store(0, Ordering::SeqCst);
    }
}

/// 应用启动时调用一次。快捷键固定 Ctrl+Enter，无需传入。
pub fn init(idle_minutes: u32, lock_on_sleep: bool) {
    #[cfg(target_os = "windows")]
    {
        IDLE_MINUTES.store(idle_minutes.min(1440), Ordering::SeqCst);
        LOCK_ON_SLEEP.store(lock_on_sleep, Ordering::SeqCst);
        IDLE_ARMED.store(true, Ordering::SeqCst);
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("mec-sleep".to_string())
                .spawn(sleep_thread);
        });
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (idle_minutes, lock_on_sleep);
    }
}

/// 前端修改息屏设置后立即生效，并在窗口线程上重试一次热键注册。
fn reconfigure(idle_minutes: u32, lock_on_sleep: bool) {
    IDLE_MINUTES.store(idle_minutes.min(1440), Ordering::SeqCst);
    LOCK_ON_SLEEP.store(lock_on_sleep, Ordering::SeqCst);
    // 改设置时重新布防，让用户能立刻用新的时长测试。
    IDLE_ARMED.store(true, Ordering::SeqCst);
    #[cfg(target_os = "windows")]
    {
        let hwnd = WATCHER_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                win::PostMessageW(hwnd as win::Hwnd, win::WM_MEC_RECONFIG, 0, 0);
            }
        }
    }
}

/// 立即息屏命令（供前端“手动息屏”按钮调用）。
#[tauri::command]
pub fn sleep_now() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let hwnd = WATCHER_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                win::PostMessageW(hwnd as win::Hwnd, win::WM_MEC_SLEEP_NOW, 0, 0);
            }
            return Ok(());
        }
    }
    if turn_off_display() {
        Ok(())
    } else {
        Err("未找到可用于息屏的系统窗口".to_string())
    }
}

/// 设置息屏选项并立即生效，同时同步持久化。快捷键固定为 Ctrl+Enter。
#[tauri::command]
pub fn set_sleep_settings(
    app: tauri::AppHandle,
    idle_minutes: u32,
    lock_on_sleep: bool,
) -> Result<(), String> {
    let minutes = idle_minutes.min(1440);
    let mut settings = crate::settings::load_from(&app);
    settings.idle_sleep_minutes = minutes;
    settings.lock_on_sleep = lock_on_sleep;
    crate::settings::save_settings(app, settings)?;
    reconfigure(minutes, lock_on_sleep);
    Ok(())
}

/// 当前息屏状态，供前端提示快捷键是否被占用。
#[derive(Serialize)]
pub struct SleepStatus {
    pub hotkey_registered: bool,
}

#[tauri::command]
pub fn get_sleep_status() -> SleepStatus {
    #[cfg(target_os = "windows")]
    {
        // 启动时前端可能在隐藏窗口线程完成注册之前就查询；最多等 500ms。
        for _ in 0..20 {
            if HOTKEY_CHECKED.load(Ordering::SeqCst) { break; }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        // 不在主线程注册热键：RegisterHotKey 必须由拥有目标窗口的线程调用。
    }
    SleepStatus {
        hotkey_registered: HOTKEY_OK.load(Ordering::SeqCst),
    }
}

pub fn shutdown() {
    #[cfg(target_os = "windows")]
    {
        let hwnd = WATCHER_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                win::PostMessageW(hwnd as win::Hwnd, win::WM_QUIT, 0, 0);
            }
        }
    }
}
