//! 全屏柔色遮罩层。
//!
//! 用 Win32 分层窗口（WS_EX_LAYERED + WS_EX_TRANSPARENT）实现，不创建第二个 WebView，
//! 不启动额外进程，常驻内存只有几 MB 级别；鼠标事件直接穿透，不影响任何操作。
//! gamma 负责底层调光，遮罩作为可选增强叠加在上面。

use crate::settings::Settings;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

#[cfg(target_os = "windows")]
mod win {
    use std::ffi::{c_int, c_void};
    use std::os::raw::{c_uchar, c_ushort};

    pub type Hwnd = *mut c_void;
    pub type Hdc = *mut c_void;
    pub type Hbrush = *mut c_void;
    pub type Hinstance = *mut c_void;
    pub type Atom = c_ushort;
    pub type WndProc = Option<extern "system" fn(Hwnd, u32, usize, isize) -> isize>;

    #[repr(C)]
    pub struct Rect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    #[repr(C)]
    pub struct WndClassExW {
        pub cb_size: u32,
        pub style: u32,
        pub lpfn_wnd_proc: WndProc,
        pub cb_cls_extra: c_int,
        pub cb_wnd_extra: c_int,
        pub h_instance: Hinstance,
        pub h_icon: *mut c_void,
        pub h_cursor: *mut c_void,
        pub hbr_background: Hbrush,
        pub lpsz_menu_name: *const u16,
        pub lpsz_class_name: *const u16,
        pub h_icon_sm: *mut c_void,
    }

    #[repr(C)]
    pub struct PaintStruct {
        pub hdc: Hdc,
        pub f_erase: c_int,
        pub rc_paint: Rect,
        pub f_restore: c_int,
        pub f_inc_update: c_int,
        pub rgb_reserved: [c_uchar; 32],
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

    pub const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
    pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    pub const WS_EX_LAYERED: u32 = 0x0008_0000;
    pub const WS_EX_TOPMOST: u32 = 0x0000_0008;
    pub const WS_POPUP: u32 = 0x8000_0000;
    pub const LWA_ALPHA: u32 = 0x0000_0002;
    pub const SW_HIDE: i32 = 0;
    pub const SW_SHOW: i32 = 5;
    pub const SWP_NOSIZE: u32 = 0x0001;
    pub const SWP_NOMOVE: u32 = 0x0002;
    pub const SWP_NOACTIVATE: u32 = 0x0010;
    pub const WM_PAINT: u32 = 0x000F;
    pub const WM_ERASEBKGND: u32 = 0x0014;
    pub const WM_QUIT: u32 = 0x0012;
    pub const WM_DISPLAYCHANGE: u32 = 0x007E;
    pub const WM_DPICHANGED: u32 = 0x02E0;
    pub const WM_APP: u32 = 0x8000;
    /// 更新颜色与透明度：wparam=COLORREF, lparam=alpha
    pub const WM_MEC_TINT: u32 = WM_APP + 1;
    /// 显示/隐藏：wparam=1 显示，0 隐藏
    pub const WM_MEC_VISIBLE: u32 = WM_APP + 2;
    /// 结束窗口线程
    pub const WM_MEC_QUIT: u32 = WM_APP + 3;
    /// 显示恢复后重新同步几何、层级、颜色与可见性
    pub const WM_MEC_REFRESH: u32 = WM_APP + 4;

    #[inline]
    pub fn hwnd_topmost() -> Hwnd {
        -1isize as Hwnd
    }

    // 虚拟桌面范围（覆盖多显示器）
    pub const SM_XVIRTUALSCREEN: i32 = 76;
    pub const SM_YVIRTUALSCREEN: i32 = 77;
    pub const SM_CXVIRTUALSCREEN: i32 = 78;
    pub const SM_CYVIRTUALSCREEN: i32 = 79;

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
            menu: *mut c_void,
            instance: Hinstance,
            param: *mut c_void,
        ) -> Hwnd;
        pub fn ShowWindow(hwnd: Hwnd, cmd: i32) -> c_int;
        pub fn SetWindowPos(
            hwnd: Hwnd,
            insert_after: Hwnd,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> c_int;
        pub fn SetLayeredWindowAttributes(hwnd: Hwnd, key: u32, alpha: c_uchar, flags: u32) -> c_int;
        pub fn InvalidateRect(hwnd: Hwnd, rect: *const Rect, erase: c_int) -> c_int;
        pub fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> c_int;
        pub fn PostQuitMessage(exit_code: c_int);
        pub fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> c_int;
        pub fn TranslateMessage(msg: *const Msg) -> c_int;
        pub fn DispatchMessageW(msg: *const Msg) -> isize;
        pub fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        pub fn BeginPaint(hwnd: Hwnd, paint: *mut PaintStruct) -> Hdc;
        pub fn EndPaint(hwnd: Hwnd, paint: *const PaintStruct) -> c_int;
        pub fn FillRect(hdc: Hdc, rect: *const Rect, brush: Hbrush) -> c_int;
        pub fn GetClientRect(hwnd: Hwnd, rect: *mut Rect) -> c_int;
        pub fn GetSystemMetrics(index: c_int) -> c_int;
        pub fn GetModuleHandleW(module_name: *const u16) -> Hinstance;
    }

    #[link(name = "gdi32")]
    extern "system" {
        pub fn CreateSolidBrush(color: u32) -> Hbrush;
        pub fn DeleteObject(object: *mut c_void) -> c_int;
    }
}

// CURRENT_COLOR 只由窗口线程维护，表示“已经真正画到窗口上的颜色”，是幂等判断的唯一依据。
// DESIRED_* 由任意线程写入，表示“期望状态”，供窗口线程就绪前的启动时序使用。
// 关键：apply 绝不能提前改 CURRENT_COLOR，否则窗口过程比较时永远相等、颜色变化不生效。
#[cfg(target_os = "windows")]
static CURRENT_COLOR: AtomicU32 = AtomicU32::new(0x0060_B0FF);
#[cfg(target_os = "windows")]
static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);
#[cfg(target_os = "windows")]
static DESIRED_COLOR: AtomicU32 = AtomicU32::new(0x0060_B0FF);
#[cfg(target_os = "windows")]
static DESIRED_ALPHA: AtomicU32 = AtomicU32::new(0);
#[cfg(target_os = "windows")]
static DESIRED_SHOW: AtomicU32 = AtomicU32::new(0);
// 已实际落到窗口的状态：用于幂等判断，避免重复 ShowWindow/重绘造成闪烁。
// u32::MAX 表示“尚未落地”，首次同步必定执行。
#[cfg(target_os = "windows")]
static APPLIED_ALPHA: AtomicU32 = AtomicU32::new(u32::MAX);
#[cfg(target_os = "windows")]
static APPLIED_SHOW: AtomicU32 = AtomicU32::new(u32::MAX);
// 窗口几何缓存 (x, y, w, h)，避免每次 refresh 都 SetWindowPos
#[cfg(target_os = "windows")]
static APPLIED_GEOM: std::sync::Mutex<(i32, i32, i32, i32)> = std::sync::Mutex::new((i32::MIN, i32::MIN, i32::MIN, i32::MIN));

/// 把期望状态完整同步到窗口：几何、置顶、颜色、透明度、可见性。
/// 创建时和显示恢复后都走这里，避免两处逻辑漂移。
#[cfg(target_os = "windows")]
unsafe fn sync_window(hwnd: win::Hwnd) {
    let x = win::GetSystemMetrics(win::SM_XVIRTUALSCREEN);
    let y = win::GetSystemMetrics(win::SM_YVIRTUALSCREEN);
    let width = win::GetSystemMetrics(win::SM_CXVIRTUALSCREEN);
    let height = win::GetSystemMetrics(win::SM_CYVIRTUALSCREEN);

    let alpha = DESIRED_ALPHA.load(Ordering::SeqCst) as u8;
    let show = DESIRED_SHOW.load(Ordering::SeqCst) != 0;

    // 全部状态都没变：什么都不做。这是消除拖动/切换时闪烁的核心。
    let geom_unchanged = APPLIED_GEOM
        .lock()
        .map(|g| *g == (x, y, width, height))
        .unwrap_or(false);
    if geom_unchanged
        && APPLIED_ALPHA.load(Ordering::SeqCst) == alpha as u32
        && APPLIED_SHOW.load(Ordering::SeqCst) == if show { 1 } else { 0 }
    {
        return;
    }

    // 几何变化才 SetWindowPos
    if !geom_unchanged {
        win::SetWindowPos(
            hwnd,
            win::hwnd_topmost(),
            x,
            y,
            width,
            height,
            win::SWP_NOACTIVATE,
        );
        if let Ok(mut g) = APPLIED_GEOM.lock() {
            *g = (x, y, width, height);
        }
    }
    // 透明度变化才改
    if APPLIED_ALPHA.load(Ordering::SeqCst) != alpha as u32 {
        win::SetLayeredWindowAttributes(hwnd, 0, alpha, win::LWA_ALPHA);
        APPLIED_ALPHA.store(alpha as u32, Ordering::SeqCst);
    }
    // 可见性变化才 ShowWindow；显示时再置顶一次（避免被压到底层）
    if APPLIED_SHOW.load(Ordering::SeqCst) != if show { 1 } else { 0 } {
        win::ShowWindow(hwnd, if show { win::SW_SHOW } else { win::SW_HIDE });
        APPLIED_SHOW.store(if show { 1 } else { 0 }, Ordering::SeqCst);
        if show {
            win::SetWindowPos(
                hwnd,
                win::hwnd_topmost(),
                0,
                0,
                0,
                0,
                win::SWP_NOMOVE | win::SWP_NOSIZE | win::SWP_NOACTIVATE,
            );
        }
    }
    win::InvalidateRect(hwnd, std::ptr::null(), 0);
}

#[cfg(target_os = "windows")]
extern "system" fn overlay_proc(
    hwnd: win::Hwnd,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    unsafe {
        match msg {
            win::WM_MEC_TINT => {
                let color = wparam as u32;
                let alpha = lparam as u8;
                let color_changed = CURRENT_COLOR.load(Ordering::Relaxed) != color;
                let alpha_changed = APPLIED_ALPHA.load(Ordering::SeqCst) != alpha as u32;
                if !color_changed && !alpha_changed {
                    return 0; // 幂等：颜色与透明度都没变，跳过重绘
                }
                CURRENT_COLOR.store(color, Ordering::Relaxed);
                DESIRED_COLOR.store(color, Ordering::SeqCst);
                if alpha_changed {
                    win::SetLayeredWindowAttributes(hwnd, 0, alpha, win::LWA_ALPHA);
                    APPLIED_ALPHA.store(alpha as u32, Ordering::SeqCst);
                    DESIRED_ALPHA.store(alpha as u32, Ordering::SeqCst);
                }
                if color_changed {
                    win::InvalidateRect(hwnd, std::ptr::null(), 0);
                }
                0
            }
            win::WM_MEC_VISIBLE => {
                let show = if wparam != 0 { 1 } else { 0 };
                if APPLIED_SHOW.load(Ordering::SeqCst) == show {
                    return 0; // 幂等：可见性没变
                }
                win::ShowWindow(hwnd, if show != 0 { win::SW_SHOW } else { win::SW_HIDE });
                APPLIED_SHOW.store(show, Ordering::SeqCst);
                DESIRED_SHOW.store(show, Ordering::SeqCst);
                if show != 0 {
                    win::SetWindowPos(
                        hwnd,
                        win::hwnd_topmost(),
                        0,
                        0,
                        0,
                        0,
                        win::SWP_NOMOVE | win::SWP_NOSIZE | win::SWP_NOACTIVATE,
                    );
                }
                0
            }
            win::WM_MEC_REFRESH | win::WM_DISPLAYCHANGE | win::WM_DPICHANGED => {
                sync_window(hwnd);
                0
            }
            win::WM_MEC_QUIT => {
                win::PostQuitMessage(0);
                0
            }
            win::WM_PAINT => {
                let mut ps: win::PaintStruct = std::mem::zeroed();
                let hdc = win::BeginPaint(hwnd, &mut ps);
                let mut rect: win::Rect = std::mem::zeroed();
                win::GetClientRect(hwnd, &mut rect);
                let brush = win::CreateSolidBrush(CURRENT_COLOR.load(Ordering::Relaxed));
                win::FillRect(hdc, &rect, brush);
                win::DeleteObject(brush as *mut std::ffi::c_void);
                win::EndPaint(hwnd, &ps);
                0
            }
            win::WM_ERASEBKGND => 1,
            _ => win::DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(target_os = "windows")]
fn overlay_thread() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    unsafe {
        let instance = win::GetModuleHandleW(std::ptr::null());
        let class_name: Vec<u16> = OsStr::new("MyEyeCareOverlayClass\0")
            .encode_wide()
            .collect();
        let wc = win::WndClassExW {
            cb_size: std::mem::size_of::<win::WndClassExW>() as u32,
            style: 0,
            lpfn_wnd_proc: Some(overlay_proc),
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

        let x = win::GetSystemMetrics(win::SM_XVIRTUALSCREEN);
        let y = win::GetSystemMetrics(win::SM_YVIRTUALSCREEN);
        let width = win::GetSystemMetrics(win::SM_CXVIRTUALSCREEN);
        let height = win::GetSystemMetrics(win::SM_CYVIRTUALSCREEN);

        let hwnd = win::CreateWindowExW(
            win::WS_EX_LAYERED | win::WS_EX_TRANSPARENT | win::WS_EX_TOOLWINDOW | win::WS_EX_TOPMOST,
            class_name.as_ptr(),
            std::ptr::null(),
            win::WS_POPUP,
            x,
            y,
            width,
            height,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        OVERLAY_HWND.store(hwnd as isize, Ordering::SeqCst);
        // 用期望颜色初始化首帧，避免启动瞬间闪一下硬编码默认色
        CURRENT_COLOR.store(DESIRED_COLOR.load(Ordering::SeqCst), Ordering::Relaxed);
        // 应用线程就绪前已写入的期望状态，统一由 sync_window 落地
        sync_window(hwnd);

        let mut msg: win::Msg = std::mem::zeroed();
        while win::GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.message == win::WM_QUIT {
                break;
            }
            win::TranslateMessage(&msg);
            win::DispatchMessageW(&msg);
        }
        OVERLAY_HWND.store(0, Ordering::SeqCst);
    }
}

/// 在应用启动时调用一次，创建遮罩窗口线程。
pub fn init() {
    #[cfg(target_os = "windows")]
    {
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("mec-overlay".to_string())
                .spawn(overlay_thread);
        });
    }
}

/// #RRGGBB -> COLORREF(0x00BBGGRR)，非法值回退到默认暖琥珀色。
#[cfg(target_os = "windows")]
fn hex_to_colorref(hex: &str) -> u32 {
    const DEFAULT: u32 = 0x0060_B0FF; // RGB(255,176,96)
    // 按字节校验：直接对 &str 做 [0..2] 切片会在多字节字符中间 panic（panic=abort 会导致整个应用退出）
    let bytes = hex.as_bytes();
    let s = if bytes.first() == Some(&b'#') { &bytes[1..] } else { bytes };
    if s.len() != 6 || !s.iter().all(u8::is_ascii_hexdigit) {
        return DEFAULT;
    }
    let parse = |range: &[u8]| -> u32 {
        std::str::from_utf8(range)
            .ok()
            .and_then(|v| u8::from_str_radix(v, 16).ok())
            .unwrap_or(0) as u32
    };
    (parse(&s[4..6]) << 16) | (parse(&s[2..4]) << 8) | parse(&s[0..2])
}

/// 根据当前设置更新遮罩。由 apply_effect / 启动时统一调用。
pub fn apply(settings: &Settings) {
    #[cfg(target_os = "windows")]
    {
        let show = settings.enabled && settings.overlay_enabled;
        let strength = settings.overlay_strength.min(100) as f64 / 100.0;
        // 用户选择的柔色（#RRGGBB），最大不透明度约 28%，避免画面失真
        let color: u32 = hex_to_colorref(&settings.overlay_color);
        let alpha = if show { (strength * 72.0).round() as u8 } else { 0 };
        // 无论句柄是否就绪，先写期望状态：窗口线程创建后会自行应用。
        // 注意只写 DESIRED_*，绝不动 CURRENT_COLOR —— 后者是窗口线程独享的
        // “已落地颜色”，提前写会让 TINT 的幂等判断误判为“没变化”而不重绘。
        DESIRED_COLOR.store(color, Ordering::SeqCst);
        DESIRED_ALPHA.store(alpha as u32, Ordering::SeqCst);
        DESIRED_SHOW.store(if show { 1 } else { 0 }, Ordering::SeqCst);

        let hwnd = OVERLAY_HWND.load(Ordering::SeqCst);
        if hwnd == 0 {
            return;
        }
        unsafe {
            win::PostMessageW(
                hwnd as win::Hwnd,
                win::WM_MEC_TINT,
                color as usize,
                alpha as isize,
            );
            win::PostMessageW(
                hwnd as win::Hwnd,
                win::WM_MEC_VISIBLE,
                if show { 1 } else { 0 },
                0,
            );
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = settings;
    }
}

/// 显示恢复后通知窗口线程重新同步几何、层级与颜色，避免跨线程直接操作窗口。
pub fn refresh() {
    #[cfg(target_os = "windows")]
    {
        let hwnd = OVERLAY_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                win::PostMessageW(hwnd as win::Hwnd, win::WM_MEC_REFRESH, 0, 0);
            }
        }
    }
}

/// 退出时结束遮罩窗口线程。
pub fn shutdown() {
    #[cfg(target_os = "windows")]
    {
        let hwnd = OVERLAY_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                win::PostMessageW(hwnd as win::Hwnd, win::WM_MEC_QUIT, 0, 0);
            }
        }
    }
}
