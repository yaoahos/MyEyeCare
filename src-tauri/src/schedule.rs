//! 按时间自动调节色温/亮度（昼夜两态日程）。
//!
//! 借鉴 LightBulb / redshift 的昼夜模型：只设「白天」「夜晚」两组色温+亮度，
//! 在日出/日落时刻用余弦平滑过渡，不再让用户拖拽多控制点曲线。默认关闭，开启后：
//! - 调度线程每 30 秒计算一次“此刻应有的色温/亮度”，差异超阈值才写 gamma；
//! - 用户手动调节会打 30 分钟临时覆盖标记，到期自动交还给日程；
//! - 息屏唤醒 / 显示变化时，display 模块也通过 effective() 取当前日程值重应用。

use crate::settings::Settings;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

/// 手动覆盖到期时间戳（毫秒），0 表示无覆盖。与前端 state 双向同步。
static OVERRIDE_UNTIL: AtomicI64 = AtomicI64::new(0);
/// 上次已写入的生效值 (temperature, brightness_bits)，用于差异阈值判断。
static LAST_APPLIED: Mutex<Option<(u32, u64)>> = Mutex::new(None);

const TICK_MS: u32 = 30_000;
const TEMP_THRESHOLD: u32 = 30;
const BRIGHT_THRESHOLD: f64 = 0.01;

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn GetLocalTime(st: *mut SystemTime);
}

/// 当前本地时间距离 00:00 的分钟数。
#[cfg(target_os = "windows")]
fn now_minute() -> u16 {
    unsafe {
        let mut st = SystemTime::default();
        GetLocalTime(&mut st);
        st.hour * 60 + st.minute
    }
}

#[cfg(not(target_os = "windows"))]
fn now_minute() -> u16 {
    // 非 Windows 平台仅用于编译检查，返回固定值
    12 * 60
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 当前分钟与某个过渡中心的有符号距离（分钟），范围 [-720, 720)，自动跨午夜。
fn signed_minute_diff(now: u16, center: u16) -> i32 {
    let mut d = now as i32 - center as i32;
    if d >= 720 { d -= 1440; }
    if d < -720 { d += 1440; }
    d
}

/// 昼夜两态余弦插值：返回 (色温, 亮度)。
///
/// 以日出/日落时刻为中心，前后各 transition/2 分钟为过渡窗口：
/// - 日出：夜晚 → 白天，用 (1 - cos(π·p))/2 平滑（0→1）；
/// - 日落：白天 → 夜晚，用 (1 + cos(π·p))/2 平滑（1→0）；
/// - 其余时段：白天或夜晚的固定值。
/// p 为窗口内进度 0..1；余弦保证两端斜率为 0，无跳变、无折角。
fn interpolate(s: &Settings, minute: u16) -> (u32, f64) {
    let half = (s.transition_minutes as f64 / 2.0).max(1.0);
    let sunrise_d = signed_minute_diff(minute, s.sunrise_minute) as f64;
    let sunset_d = signed_minute_diff(minute, s.sunset_minute) as f64;

    // 白天比例 day_ratio：1=全白天，0=全夜晚
    let day_ratio = if sunrise_d.abs() < half {
        // 日出窗口：从夜晚(0)升到白天(1)
        let p = (sunrise_d + half) / (2.0 * half);
        (1.0 - (std::f64::consts::PI * p).cos()) / 2.0
    } else if sunset_d.abs() < half {
        // 日落窗口：从白天(1)降到夜晚(0)
        let p = (sunset_d + half) / (2.0 * half);
        (1.0 + (std::f64::consts::PI * p).cos()) / 2.0
    } else {
        // 窗口外：把一天看成圆环，从日出顺时针到日落是白天区间。
        // 当前时刻距日出的正向偏移 < 白昼长度 → 白天，否则夜晚；跨午夜作息同样成立。
        let daylight = (s.sunset_minute as i32 - s.sunrise_minute as i32).rem_euclid(1440) as f64;
        let since_sunrise = (minute as i32 - s.sunrise_minute as i32).rem_euclid(1440) as f64;
        if since_sunrise < daylight { 1.0 } else { 0.0 }
    };

    let temp = s.night_temperature as f64
        + (s.day_temperature as f64 - s.night_temperature as f64) * day_ratio;
    let bright = s.night_brightness
        + (s.day_brightness - s.night_brightness) * day_ratio;
    (temp.round() as u32, bright)
}

pub fn set_override_until(until: i64) {
    OVERRIDE_UNTIL.store(until, Ordering::SeqCst);
    // 手动覆盖时清空上次生效值，确保到期后第一次日程写入不被阈值拦截
    if let Ok(mut guard) = LAST_APPLIED.lock() {
        *guard = None;
    }
}

fn is_overridden() -> bool {
    let until = OVERRIDE_UNTIL.load(Ordering::SeqCst);
    until > 0 && now_millis() < until
}

/// 纯日程计算：忽略覆盖状态，把 base 的色温/亮度替换为当前时间的插值结果。
fn auto_value(base: &Settings) -> Settings {
    let mut eff = base.clone();
    if !base.enabled {
        return eff;
    }
    let (temp, bright) = interpolate(base, now_minute());
    eff.temperature = temp;
    eff.brightness = bright;
    eff
}

/// 返回此刻真正应生效的配置：若在日程自动控制中，用插值结果覆盖色温/亮度。
pub fn effective(base: &Settings) -> Settings {
    if !base.enabled || !base.schedule_enabled || is_overridden() {
        return base.clone();
    }
    auto_value(base)
}

/// 调度线程的一次 tick：仅在值变化超过阈值时写 gamma。
fn tick() {
    let Some(base) = crate::settings::current() else {
        return;
    };
    if !base.enabled || !base.schedule_enabled || is_overridden() {
        return;
    }
    let eff = effective(&base);
    let bright_bits = eff.brightness.to_bits();
    let changed = {
        let guard = LAST_APPLIED.lock().ok();
        match guard.and_then(|g| *g) {
            None => true,
            Some((last_temp, last_bright)) => {
                eff.temperature.abs_diff(last_temp) >= TEMP_THRESHOLD
                    || ((eff.brightness - f64::from_bits(last_bright)).abs()) >= BRIGHT_THRESHOLD
            }
        }
    };
    if !changed {
        return;
    }
    crate::gamma::apply_effective(&eff);
    if let Ok(mut guard) = LAST_APPLIED.lock() {
        *guard = Some((eff.temperature, bright_bits));
    }
}

#[tauri::command]
pub fn set_manual_override(until: i64) {
    set_override_until(until);
}

/// 前端修改昼夜两态参数后写回：更新快照、清阈值缓存并立即按新配置生效。
#[tauri::command]
pub fn set_schedule_config(config: ScheduleConfig) {
    let Some(mut base) = crate::settings::current() else {
        return;
    };
    base.day_temperature = config.day_temperature;
    base.day_brightness = config.day_brightness;
    base.night_temperature = config.night_temperature;
    base.night_brightness = config.night_brightness;
    base.sunrise_minute = config.sunrise_minute;
    base.sunset_minute = config.sunset_minute;
    base.transition_minutes = config.transition_minutes;
    crate::settings::set_current(&base);
    if let Ok(mut guard) = LAST_APPLIED.lock() {
        *guard = None;
    }
    let eff = effective(&base);
    crate::gamma::apply_effective(&eff);
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleConfig {
    pub day_temperature: u32,
    pub day_brightness: f64,
    pub night_temperature: u32,
    pub night_brightness: f64,
    pub sunrise_minute: u16,
    pub sunset_minute: u16,
    pub transition_minutes: u16,
}

/// 供前端绘制只读曲线：按分钟数组返回对应的色温/亮度对。
/// 前端只负责展示，插值统一在后端，确保曲线值与真正生效值同源。
#[tauri::command]
pub fn get_schedule_curve(minutes: Vec<u16>) -> Vec<CurvePoint> {
    let s = crate::settings::current().unwrap_or_default();
    minutes
        .into_iter()
        .map(|m| {
            let (temperature, brightness) = interpolate(&s, m);
            CurvePoint { minute: m, temperature, brightness }
        })
        .collect()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurvePoint {
    pub minute: u16,
    pub temperature: u32,
    pub brightness: f64,
}

/// 返回此刻的生效色温/亮度/柔和度，供前端“恢复自动”时拉回界面显示。
/// 注意返回的应是“自动状态”下的值：临时清除覆盖计算一次再还原。
#[tauri::command]
pub fn get_effective_settings() -> EffectiveView {
    let base = crate::settings::current().unwrap_or_default();
    // 直接按曲线算自动值，不受当前覆盖状态影响，供“恢复自动”拉回界面
    let auto = auto_value(&base);
    EffectiveView {
        temperature: auto.temperature,
        brightness: auto.brightness,
        softness: auto.softness,
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveView {
    pub temperature: u32,
    pub brightness: f64,
    pub softness: u32,
}

/// 前端切换日程开关：写入快照并立即按新状态应用。
#[tauri::command]
pub fn set_schedule_enabled(enabled: bool) {
    let Some(mut base) = crate::settings::current() else {
        return;
    };
    base.schedule_enabled = enabled;
    crate::settings::set_current(&base);
    if let Ok(mut guard) = LAST_APPLIED.lock() {
        *guard = None;
    }
    let eff = effective(&base);
    crate::gamma::apply_effective(&eff);
}

/// 应用启动时调用一次：启动调度线程。
pub fn init() {
    #[cfg(target_os = "windows")]
    {
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("mec-schedule".to_string())
                .spawn(|| loop {
                    tick();
                    std::thread::sleep(std::time::Duration::from_millis(TICK_MS as u64));
                });
        });
    }
}
