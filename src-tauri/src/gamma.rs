//! 通过 Windows 的 SetDeviceGammaRamp 调整 gamma 曲线，同时实现亮度与色温。
//! 关键点：必须对每个物理显示设备用 CreateDCW 创建真实 DC，
//! EnumDisplayMonitors 回调中给出的 DC 在很多驱动上会被 SetDeviceGammaRamp 静默拒绝。

use crate::overlay;
use crate::settings::Settings;
use std::sync::Mutex;

#[cfg(target_os = "windows")]
use std::ffi::c_void;
#[cfg(target_os = "windows")]
use std::os::raw::c_int;

#[cfg(target_os = "windows")]
type Hdc = *mut c_void;
#[cfg(target_os = "windows")]
type Bool = c_int;

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy)]
struct GammaRamp {
    red: [u16; 256],
    green: [u16; 256],
    blue: [u16; 256],
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct DisplayDeviceW {
    cb: u32,
    device_name: [u16; 32],
    device_string: [u16; 128],
    state_flags: u32,
    device_id: [u16; 128],
    device_key: [u16; 128],
}

#[cfg(target_os = "windows")]
const DISPLAY_DEVICE_ACTIVE: u32 = 0x0000_0001;

#[cfg(target_os = "windows")]
#[link(name = "gdi32")]
extern "system" {
    fn CreateDCW(
        driver: *const u16,
        device: *const u16,
        output: *const u16,
        init: *const c_void,
    ) -> Hdc;
    fn DeleteDC(hdc: Hdc) -> Bool;
    fn SetDeviceGammaRamp(hdc: Hdc, ramp: *mut c_void) -> Bool;
    fn GetDeviceGammaRamp(hdc: Hdc, ramp: *mut c_void) -> Bool;
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn EnumDisplayDevicesW(
        device: *const u16,
        index: u32,
        display_device: *mut DisplayDeviceW,
        flags: u32,
    ) -> Bool;
    fn GetDC(hwnd: Hdc) -> Hdc;
    fn ReleaseDC(hwnd: Hdc, hdc: Hdc) -> c_int;
}

/// 把 ramp 推送到所有活动显示器，返回成功应用的屏幕数量。
#[cfg(target_os = "windows")]
fn push_ramp(ramp: &GammaRamp) -> usize {
    let mut applied = 0usize;
    unsafe {
        let mut index = 0u32;
        loop {
            let mut device: DisplayDeviceW = std::mem::zeroed();
            device.cb = std::mem::size_of::<DisplayDeviceW>() as u32;
            if EnumDisplayDevicesW(std::ptr::null(), index, &mut device, 0) == 0 {
                break;
            }
            index += 1;

            if device.state_flags & DISPLAY_DEVICE_ACTIVE == 0 {
                continue;
            }

            // 用设备名创建真正的物理显示 DC —— 这是生效的关键
            let dc = CreateDCW(
                std::ptr::null(),
                device.device_name.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
            );
            if dc.is_null() {
                continue;
            }
            let ok = SetDeviceGammaRamp(dc, ramp as *const GammaRamp as *mut c_void);
            DeleteDC(dc);
            if ok != 0 {
                applied += 1;
            }
        }

        // 极端情况下的兜底：主显示器 DC
        if applied == 0 {
            let dc = GetDC(std::ptr::null_mut());
            if !dc.is_null() {
                let ok = SetDeviceGammaRamp(dc, ramp as *const GammaRamp as *mut c_void);
                ReleaseDC(std::ptr::null_mut(), dc);
                if ok != 0 {
                    applied += 1;
                }
            }
        }
    }
    applied
}

#[cfg(not(target_os = "windows"))]
fn push_ramp(_ramp: &()) -> usize {
    0
}

#[cfg(target_os = "windows")]
fn identity_ramp() -> GammaRamp {
    let mut ramp = GammaRamp {
        red: [0; 256],
        green: [0; 256],
        blue: [0; 256],
    };
    for i in 0..256usize {
        let value = (i * 257) as u16;
        ramp.red[i] = value;
        ramp.green[i] = value;
        ramp.blue[i] = value;
    }
    ramp
}

#[cfg(target_os = "windows")]
fn clamp255(v: f64) -> f64 {
    v.max(0.0).min(255.0)
}

/// Tanner Helland 近似算法：开尔文色温 -> RGB
#[cfg(target_os = "windows")]
fn kelvin_rgb(kelvin: f64) -> [f64; 3] {
    let t = (kelvin / 100.0).max(1.0);
    let (r, g, b);
    if t <= 66.0 {
        r = 255.0;
        g = 99.4708025861 * t.ln() - 161.1195681661;
        b = if t <= 19.0 {
            0.0
        } else {
            138.5177312231 * (t - 10.0).ln() - 305.0447927307
        };
    } else {
        r = 329.698727446 * (t - 60.0).powf(-0.1332047592);
        g = 288.1221695283 * (t - 60.0).powf(-0.0755148492);
        b = 255.0;
    }
    [clamp255(r), clamp255(g), clamp255(b)]
}

#[cfg(target_os = "windows")]
fn channel_gains(kelvin: u32) -> [f64; 3] {
    let current = kelvin_rgb(kelvin as f64);
    let base = kelvin_rgb(6500.0);
    [
        current[0] / base[0],
        current[1] / base[1],
        current[2] / base[2],
    ]
}

#[cfg(target_os = "windows")]
fn build_ramp(brightness: f64, kelvin: u32) -> GammaRamp {
    let brightness = brightness.clamp(0.2, 1.0);
    let exponent = 1.0 / brightness;
    let gains = channel_gains(kelvin);
    let mut ramp = GammaRamp {
        red: [0; 256],
        green: [0; 256],
        blue: [0; 256],
    };
    for i in 0..256usize {
        let value = (i as f64 / 255.0).powf(exponent);
        ramp.red[i] = (value * gains[0] * 65535.0).round().clamp(0.0, 65535.0) as u16;
        ramp.green[i] = (value * gains[1] * 65535.0).round().clamp(0.0, 65535.0) as u16;
        ramp.blue[i] = (value * gains[2] * 65535.0).round().clamp(0.0, 65535.0) as u16;
    }
    ramp
}

static GAMMA_LOCK: Mutex<()> = Mutex::new(());

/// 内部使用：应用效果，返回成功屏幕数。
fn apply_inner(settings: &Settings) -> usize {
    let _guard = GAMMA_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    #[cfg(target_os = "windows")]
    let applied = {
        let ramp = if settings.enabled {
            build_ramp(settings.brightness, settings.temperature)
        } else {
            identity_ramp()
        };
        push_ramp(&ramp)
    };
    #[cfg(not(target_os = "windows"))]
    let applied = {
        let _ = settings;
        0
    };

    // 色罩与 gamma 同步更新
    overlay::apply(settings);
    applied
}

pub fn apply(settings: &Settings) {
    apply_inner(settings);
}

pub fn reset() {
    let _guard = GAMMA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(target_os = "windows")]
    push_ramp(&identity_ramp());
}

#[tauri::command]
pub fn apply_effect(settings: Settings) -> Result<usize, String> {
    let applied = apply_inner(&settings);
    if applied == 0 {
        Err("显示驱动拒绝了 gamma 调节，请确认未处于远程桌面/HDR 模式".to_string())
    } else {
        Ok(applied)
    }
}

#[tauri::command]
pub fn reset_effect() -> Result<(), String> {
    reset();
    Ok(())
}

/// 诊断命令：读取当前第一块屏幕的 gamma 表，返回是否已被修改过。
#[tauri::command]
pub fn gamma_probe() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let device_name: Vec<u16> = std::ffi::OsStr::new(r"\\.\DISPLAY1")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let dc = unsafe {
            CreateDCW(
                std::ptr::null(),
                device_name.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if dc.is_null() {
            return Err("无法打开主显示设备".to_string());
        }
        let mut ramp = GammaRamp {
            red: [0; 256],
            green: [0; 256],
            blue: [0; 256],
        };
        let ok = unsafe { GetDeviceGammaRamp(dc, &mut ramp as *mut GammaRamp as *mut c_void) };
        unsafe {
            DeleteDC(dc);
        }
        if ok == 0 {
            return Err("无法读取 gamma 表".to_string());
        }
        // 与原始曲线比较，判断驱动是否接受了修改
        Ok((0..256).any(|i| ramp.red[i] != (i * 257) as u16))
    }
    #[cfg(not(target_os = "windows"))]
    Err("仅支持 Windows".to_string())
}
