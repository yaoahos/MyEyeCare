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
/// force=false 时先读回当前 ramp 比较，完全一致则跳过写入——
/// 这是消除“拖动窗口/切换页面时闪来闪去”的关键：驱动对重复写入 gamma
/// 也会触发一次短暂的亮度跳变，幂等化后只有真正变化（或被外部重置）才写。
#[cfg(target_os = "windows")]
fn push_ramp(ramp: &GammaRamp, force: bool) -> usize {
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
            // 非强制时先读回比较：只在“明显不同”时才写。
            // 关键：很多驱动的 GetDeviceGammaRamp 读回值会与写入值差 1~2 个 LSB
            // （内部量化/缩放），用“完全相等”判断会永远判为不同，导致每次
            // reapply 都重写 gamma —— 而驱动对重复写入也会触发一次亮度跳变，
            // 这正是“切窗/拖动时闪来闪去”的根因。改用容差比较即可消除。
            let need_write = if force {
                true
            } else {
                let mut current = GammaRamp {
                    red: [0; 256],
                    green: [0; 256],
                    blue: [0; 256],
                };
                if GetDeviceGammaRamp(dc, &mut current as *mut GammaRamp as *mut c_void) == 0 {
                    true // 读不出来就照常写
                } else {
                    ramp_differs(&current, ramp)
                }
            };
            if need_write {
                let ok = SetDeviceGammaRamp(dc, ramp as *const GammaRamp as *mut c_void);
                if ok != 0 {
                    applied += 1;
                }
            } else {
                applied += 1; // 已经是正确的 ramp，视为生效
            }
            DeleteDC(dc);
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
fn push_ramp(_ramp: &(), _force: bool) -> usize {
    0
}

/// 判断两块 ramp 是否有“可见差异”。容差 4/65535 ≈ 0.006%，远低于人眼阈值，
/// 足以吸收驱动的读回量化噪声，同时真正的参数变化（色温/亮度/柔和度）远超此值。
#[cfg(target_os = "windows")]
fn ramp_differs(a: &GammaRamp, b: &GammaRamp) -> bool {
    const TOL: i32 = 4;
    for i in 0..256usize {
        if ((a.red[i] as i32) - (b.red[i] as i32)).abs() > TOL
            || ((a.green[i] as i32) - (b.green[i] as i32)).abs() > TOL
            || ((a.blue[i] as i32) - (b.blue[i] as i32)).abs() > TOL
        {
            return true;
        }
    }
    false
}

#[cfg(not(target_os = "windows"))]
fn ramp_differs(_a: &(), _b: &()) -> bool {
    false
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
fn build_ramp(brightness: f64, kelvin: u32, softness: u32) -> GammaRamp {
    let brightness = brightness.clamp(0.2, 1.0);
    let exponent = 1.0 / brightness;
    let gains = channel_gains(kelvin);

    // 柔和度：gamma 层轻微抬黑位 + 中段压缩，降低对比度但不让文字发灰。
    // lift 上限 0.03（黑位最高约 8/255，肉眼仍是黑），且亮度越低 lift 越小，
    // 避免暗部发糊；中段压缩让中间调斜率略降，黑端/白端基本不动以保留锐度。
    let soft = (softness.min(100) as f64) / 100.0;
    let lift = 0.03 * soft * brightness.max(0.5);
    let mid_exp = 1.0 + 0.35 * soft;

    let mut ramp = GammaRamp {
        red: [0; 256],
        green: [0; 256],
        blue: [0; 256],
    };
    for i in 0..256usize {
        let base = (i as f64 / 255.0).powf(exponent);
        let shaped = if soft > 0.0 {
            // 先抬黑位，再压中段；两端钳制保护
            let lifted = lift + (1.0 - lift) * base;
            lifted.powf(mid_exp).min(1.0)
        } else {
            base
        };
        ramp.red[i] = (shaped * gains[0] * 65535.0).round().clamp(0.0, 65535.0) as u16;
        ramp.green[i] = (shaped * gains[1] * 65535.0).round().clamp(0.0, 65535.0) as u16;
        ramp.blue[i] = (shaped * gains[2] * 65535.0).round().clamp(0.0, 65535.0) as u16;
    }
    ramp
}

static GAMMA_LOCK: Mutex<()> = Mutex::new(());

/// 内部使用：应用效果，返回成功屏幕数。
/// save_snapshot=true 时把传入配置存为“用户配置快照”；
/// 日程/显示监听应用的是计算后的生效值，不能污染快照，故传 false。
fn apply_inner(settings: &Settings, save_snapshot: bool) -> usize {
    if save_snapshot {
        crate::settings::set_current(settings);
    }
    let _guard = GAMMA_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    #[cfg(target_os = "windows")]
    let applied = {
        let ramp = if settings.enabled {
            build_ramp(settings.brightness, settings.temperature, settings.softness)
        } else {
            identity_ramp()
        };
        push_ramp(&ramp, false)
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

/// 应用用户配置（会更新快照）。用于启动时和前端手动调节。
pub fn apply(settings: &Settings) {
    apply_inner(settings, true);
}

/// 应用计算后的生效值，不写快照。用于日程 tick 与显示变化重应用。
pub fn apply_effective(settings: &Settings) {
    apply_inner(settings, false);
}

pub fn reset() {
    let _guard = GAMMA_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(target_os = "windows")]
    push_ramp(&identity_ramp(), true);
}

#[tauri::command]
pub fn apply_effect(settings: Settings) -> Result<usize, String> {
    let applied = apply_inner(&settings, true);
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
