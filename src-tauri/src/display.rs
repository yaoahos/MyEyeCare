//! 显示状态监听。
//!
//! Windows 在息屏唤醒、屏幕重启、分辨率/DPI 变化后会重置显卡 gamma；
//! 遮罩窗口也可能失去置顶层级或仍停留在旧几何。这里用一个隐藏窗口接收
//! 系统广播（含显示器电源通知），分级延迟重应用当前配置；部分驱动完全不广播，
//! 因此再加低频定时兜底。

use std::sync::atomic::{AtomicIsize, Ordering};

#[cfg(target_os = "windows")]
mod win {
    use std::ffi::{c_int, c_void};

    pub type Hwnd = *mut c_void;
    pub type HpowerNotify = *mut c_void;
    pub type Atom = u16;
    pub type WndProc = Option<extern "system" fn(Hwnd, u32, usize, isize) -> isize>;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct Guid {
        pub data1: u32,
        pub data2: u16,
        pub data3: u16,
        pub data4: [u8; 8],
    }

    /// WM_POWERBROADCAST / PBT_POWERSETTINGCHANGE 的参数。Data 为变长，
    /// 这里只取首字节即可判断显示器开关（1 = 开，0 = 关）。
    #[repr(C)]
    pub struct PowerBroadcastSetting {
        pub power_setting: Guid,
        pub data_length: u32,
        pub data: [u8; 1],
    }

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

    pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    pub const WS_POPUP: u32 = 0x8000_0000;
    pub const WM_QUIT: u32 = 0x0012;
    pub const WM_DISPLAYCHANGE: u32 = 0x007E;
    pub const WM_DPICHANGED: u32 = 0x02E0;
    pub const WM_POWERBROADCAST: u32 = 0x0218;
    pub const WM_TIMER: u32 = 0x0113;
    pub const WM_APP: u32 = 0x8000;
    pub const WM_MEC_QUIT: u32 = WM_APP + 5;
    /// DPI 变化时只同步遮罩几何，不重写 gamma（gamma 与 DPI 无关，重复写会闪）
    pub const TIMER_GEOMETRY_ONLY: usize = 5;
    pub const GEOMETRY_DELAY_MS: u32 = 250;

    pub const PBT_APMRESUMECRITICAL: usize = 0x0006;
    pub const PBT_APMRESUMESUSPEND: usize = 0x0007;
    pub const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
    pub const PBT_POWERSETTINGCHANGE: usize = 0x8013;
    pub const DEVICE_NOTIFY_WINDOW_HANDLE: u32 = 0;

    /// 显示器电源：息屏再唤醒走这个通知，而不是系统睡眠的 PBT_APMRESUME*。
    pub const GUID_MONITOR_POWER_ON: Guid = Guid {
        data1: 0x0273_1015,
        data2: 0x4510,
        data3: 0x4526,
        data4: [0x99, 0xE6, 0xE5, 0xA1, 0x7E, 0xBD, 0x1A, 0xEA],
    };
    /// 会话显示状态：部分驱动只发这个，两个都注册以覆盖差异。
    pub const GUID_SESSION_DISPLAY_STATUS: Guid = Guid {
        data1: 0x2B84_C20E,
        data2: 0xAD23,
        data3: 0x4DDF,
        data4: [0x93, 0xDB, 0x05, 0xFF, 0xBD, 0x7E, 0xFC, 0xA5],
    };
    /// 控制台显示状态（开 / 关 / 变暗）。
    pub const GUID_CONSOLE_DISPLAY_STATE: Guid = Guid {
        data1: 0x6FE6_9556,
        data2: 0x704A,
        data3: 0x47A0,
        data4: [0x8F, 0x24, 0xC2, 0x8D, 0x93, 0x6F, 0xDA, 0x47],
    };

    pub const TIMER_REAPPLY_1: usize = 1;
    pub const TIMER_PERIODIC: usize = 2;
    pub const TIMER_REAPPLY_2: usize = 3;
    pub const TIMER_REAPPLY_3: usize = 4;

    // 唤醒后驱动可能在数秒内二次重置 gamma，做三次阶梯重应用覆盖。
    pub const FAST_DELAY_MS: u32 = 500;
    pub const MID_DELAY_MS: u32 = 1_500;
    pub const SLOW_DELAY_MS: u32 = 4_000;
    pub const PERIODIC_MS: u32 = 6_000;

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
        pub fn KillTimer(hwnd: Hwnd, id: usize) -> c_int;
        pub fn PostQuitMessage(exit_code: c_int);
        pub fn GetModuleHandleW(module_name: *const u16) -> Hwnd;
        pub fn RegisterPowerSettingNotification(
            h_recipient: Hwnd,
            power_setting_guid: *const Guid,
            flags: u32,
        ) -> HpowerNotify;
        pub fn UnregisterPowerSettingNotification(h_handle: HpowerNotify) -> c_int;
    }
}

#[cfg(target_os = "windows")]
static WATCHER_HWND: AtomicIsize = AtomicIsize::new(0);
#[cfg(target_os = "windows")]
static POWER_NOTIFY_1: AtomicIsize = AtomicIsize::new(0);
#[cfg(target_os = "windows")]
static POWER_NOTIFY_2: AtomicIsize = AtomicIsize::new(0);
#[cfg(target_os = "windows")]
static POWER_NOTIFY_3: AtomicIsize = AtomicIsize::new(0);

/// 安排三次阶梯重应用：先快速恢复，再覆盖驱动的延迟重置。
#[cfg(target_os = "windows")]
unsafe fn schedule_reapply(hwnd: win::Hwnd) {
    win::KillTimer(hwnd, win::TIMER_REAPPLY_1);
    win::KillTimer(hwnd, win::TIMER_REAPPLY_2);
    win::KillTimer(hwnd, win::TIMER_REAPPLY_3);
    win::SetTimer(hwnd, win::TIMER_REAPPLY_1, win::FAST_DELAY_MS, std::ptr::null_mut());
    win::SetTimer(hwnd, win::TIMER_REAPPLY_2, win::MID_DELAY_MS, std::ptr::null_mut());
    win::SetTimer(hwnd, win::TIMER_REAPPLY_3, win::SLOW_DELAY_MS, std::ptr::null_mut());
}

/// 完整重应用：gamma（已幂等）+ 遮罩刷新。用于息屏/电源/分辨率变化。
fn reapply() {
    let Some(settings) = crate::settings::current() else {
        return;
    };
    if !settings.enabled {
        return;
    }
    // 息屏唤醒/分辨率变化后按“此刻应有的值”重应用：若日程自动控制中，
    // effective() 会用当前时间插值出的色温/亮度覆盖快照里的手动值。
    let eff = crate::schedule::effective(&settings);
    crate::gamma::apply_effective(&eff);
    crate::overlay::refresh();
}

/// 仅同步遮罩几何：DPI 变化/窗口跨屏拖动走这里，不碰 gamma，避免闪烁。
fn refresh_geometry_only() {
    crate::overlay::refresh();
}

#[cfg(target_os = "windows")]
extern "system" fn watcher_proc(
    hwnd: win::Hwnd,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    unsafe {
        match msg {
            win::WM_DISPLAYCHANGE => {
                // 分辨率变化会重置驱动 gamma，阶梯延迟后重写
                schedule_reapply(hwnd);
                0
            }
            win::WM_DPICHANGED => {
                // DPI 变化（常见于拖动窗口跨屏）不改 gamma，只去抖同步遮罩几何。
                // 若也重写 gamma，驱动会每次跳变一次亮度，表现为“闪来闪去”。
                win::KillTimer(hwnd, win::TIMER_GEOMETRY_ONLY);
                win::SetTimer(
                    hwnd,
                    win::TIMER_GEOMETRY_ONLY,
                    win::GEOMETRY_DELAY_MS,
                    std::ptr::null_mut(),
                );
                0
            }
            win::WM_POWERBROADCAST => {
                match wparam {
                    // 系统睡眠 / 休眠唤醒
                    win::PBT_APMRESUMECRITICAL
                    | win::PBT_APMRESUMESUSPEND
                    | win::PBT_APMRESUMEAUTOMATIC => {
                        schedule_reapply(hwnd);
                    }
                    // 息屏 / 唤醒（含软件息屏）最常见的通知路径
                    win::PBT_POWERSETTINGCHANGE => {
                        let setting = lparam as *const win::PowerBroadcastSetting;
                        if !setting.is_null() {
                            let s = &*setting;
                            let is_display = s.power_setting == win::GUID_MONITOR_POWER_ON
                                || s.power_setting == win::GUID_SESSION_DISPLAY_STATUS
                                || s.power_setting == win::GUID_CONSOLE_DISPLAY_STATE;
                            // Data[0] = 1 表示显示器点亮；息屏（0）时不重应用
                            if is_display && s.data[0] != 0 {
                                schedule_reapply(hwnd);
                            }
                        }
                    }
                    _ => {}
                }
                0
            }
            win::WM_TIMER => {
                match wparam {
                    win::TIMER_REAPPLY_1 => {
                        win::KillTimer(hwnd, win::TIMER_REAPPLY_1);
                        reapply();
                    }
                    win::TIMER_REAPPLY_2 => {
                        win::KillTimer(hwnd, win::TIMER_REAPPLY_2);
                        reapply();
                    }
                    win::TIMER_REAPPLY_3 => {
                        win::KillTimer(hwnd, win::TIMER_REAPPLY_3);
                        reapply();
                    }
                    win::TIMER_GEOMETRY_ONLY => {
                        win::KillTimer(hwnd, win::TIMER_GEOMETRY_ONLY);
                        refresh_geometry_only();
                    }
                    win::TIMER_PERIODIC => reapply(),
                    _ => {}
                }
                0
            }
            win::WM_MEC_QUIT => {
                win::PostQuitMessage(0);
                0
            }
            _ => win::DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(target_os = "windows")]
fn watcher_thread() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    unsafe {
        let instance = win::GetModuleHandleW(std::ptr::null());
        let class_name: Vec<u16> = OsStr::new("MyEyeCareDisplayWatcherClass\0")
            .encode_wide()
            .collect();
        let wc = win::WndClassExW {
            cb_size: std::mem::size_of::<win::WndClassExW>() as u32,
            style: 0,
            lpfn_wnd_proc: Some(watcher_proc),
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

        // 不调用 ShowWindow：隐藏的普通窗口仍能收到 HWND_BROADCAST 事件。
        let hwnd = win::CreateWindowExW(
            win::WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            std::ptr::null(),
            win::WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        WATCHER_HWND.store(hwnd as isize, Ordering::SeqCst);

        // 注册显示器电源通知：息屏唤醒不一定发 PBT_APMRESUME*，必须单独订阅。
        let h1 = win::RegisterPowerSettingNotification(
            hwnd,
            &win::GUID_MONITOR_POWER_ON,
            win::DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        let h2 = win::RegisterPowerSettingNotification(
            hwnd,
            &win::GUID_SESSION_DISPLAY_STATUS,
            win::DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        let h3 = win::RegisterPowerSettingNotification(
            hwnd,
            &win::GUID_CONSOLE_DISPLAY_STATE,
            win::DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        POWER_NOTIFY_1.store(h1 as isize, Ordering::SeqCst);
        POWER_NOTIFY_2.store(h2 as isize, Ordering::SeqCst);
        POWER_NOTIFY_3.store(h3 as isize, Ordering::SeqCst);

        // 兜底：部分驱动息屏/亮屏不广播任何消息，6 秒内自动恢复。
        win::SetTimer(hwnd, win::TIMER_PERIODIC, win::PERIODIC_MS, std::ptr::null_mut());

        let mut msg: win::Msg = std::mem::zeroed();
        while win::GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.message == win::WM_QUIT {
                break;
            }
            win::TranslateMessage(&msg);
            win::DispatchMessageW(&msg);
        }

        for handle in [POWER_NOTIFY_1.load(Ordering::SeqCst), POWER_NOTIFY_2.load(Ordering::SeqCst), POWER_NOTIFY_3.load(Ordering::SeqCst)] {
            if handle != 0 {
                win::UnregisterPowerSettingNotification(handle as win::HpowerNotify);
            }
        }
        WATCHER_HWND.store(0, Ordering::SeqCst);
    }
}

/// 应用启动时调用一次。
pub fn init() {
    #[cfg(target_os = "windows")]
    {
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("mec-display-watcher".to_string())
                .spawn(watcher_thread);
        });
    }
}

pub fn shutdown() {
    #[cfg(target_os = "windows")]
    {
        let hwnd = WATCHER_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                win::PostMessageW(hwnd as win::Hwnd, win::WM_MEC_QUIT, 0, 0);
            }
        }
    }
}
