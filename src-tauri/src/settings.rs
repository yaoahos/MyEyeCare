use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// 当前生效配置的进程内快照，供显示状态监听线程在息屏恢复后重应用。
static CURRENT: Mutex<Option<Settings>> = Mutex::new(None);

pub fn set_current(settings: &Settings) {
    if let Ok(mut guard) = CURRENT.lock() {
        *guard = Some(settings.clone());
    }
}

pub fn current() -> Option<Settings> {
    CURRENT.lock().ok().and_then(|guard| guard.clone())
}

/// 用户自定义护眼模板（最多保存 3 个）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomTemplate {
    pub id: String,
    pub name: String,
    pub temperature: u32,
    pub brightness: f64,
}

impl Default for CustomTemplate {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            temperature: 6500,
            brightness: 1.0,
        }
    }
}

/// 日程曲线上的一个控制点：从 00:00 起的分钟数 + 该时刻的目标色温/亮度。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SchedulePoint {
    /// 0..1439
    pub minute: u16,
    /// 2700..6500
    pub temperature: u32,
    /// 0.2..1.0
    pub brightness: f64,
}

impl Default for SchedulePoint {
    fn default() -> Self {
        Self { minute: 0, temperature: 5000, brightness: 0.9 }
    }
}

/// 应用全部持久化配置。字段缺失时自动使用默认值，方便后续版本平滑升级。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 亮度系数：0.2 ~ 1.0（1.0 为原始亮度）
    pub brightness: f64,
    /// 色温（开尔文）：2700 ~ 6500
    pub temperature: u32,
    /// 柔和度 0~100：gamma 层抬黑位 + 中段压缩，降低对比度但文字不发灰
    pub softness: u32,
    /// 护眼效果总开关
    pub enabled: bool,
    /// 开机自启（安装版默认开启；绿色版不写注册表）
    pub autostart_enabled: bool,
    /// 启动时最小化到托盘
    pub start_minimized: bool,
    /// 启动时自动检查更新
    pub check_updates_on_startup: bool,
    /// 当前选中的模板 id
    pub template_id: String,
    /// 用户保存的自定义模板（最多 3 个）
    pub custom_templates: Vec<CustomTemplate>,
    /// 是否启用按时间自动调节
    pub schedule_enabled: bool,
    /// 日程控制点（旧版多控制点模型，保留字段仅为兼容旧配置，新模型不再使用）
    #[serde(default)]
    pub schedule_points: Vec<SchedulePoint>,
    /// 白天色温（开尔文）
    pub day_temperature: u32,
    /// 白天亮度 0.1~1.0
    pub day_brightness: f64,
    /// 夜晚色温（开尔文）
    pub night_temperature: u32,
    /// 夜晚亮度 0.1~1.0
    pub night_brightness: f64,
    /// 日出时刻（距 00:00 的分钟），从夜晚过渡到白天
    pub sunrise_minute: u16,
    /// 日落时刻（距 00:00 的分钟），从白天过渡到夜晚
    pub sunset_minute: u16,
    /// 昼夜过渡时长（分钟），过渡窗口以日出/日落时刻为中心，前后各占一半
    pub transition_minutes: u16,
    /// 手动临时覆盖到期时间戳（毫秒）；0 表示无覆盖
    pub manual_override_until: i64,
    /// 空闲多少分钟后自动息屏；0 表示关闭，沿用系统设置
    pub idle_sleep_minutes: u32,
    /// 息屏时是否一并锁定 Windows（默认关闭）
    pub lock_on_sleep: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            brightness: 0.92,
            temperature: 5000,
            softness: 45,
            enabled: true,
            autostart_enabled: true,
            start_minimized: true,
            check_updates_on_startup: true,
            template_id: "standard".to_string(),
            custom_templates: Vec::new(),
            schedule_enabled: false,
            schedule_points: Vec::new(),
            day_temperature: 6500,
            day_brightness: 1.0,
            night_temperature: 3900,
            night_brightness: 0.85,
            sunrise_minute: 7 * 60 + 20,
            sunset_minute: 18 * 60 + 30,
            transition_minutes: 40,
            manual_override_until: 0,
            // 默认不自动息屏，沿用系统设置；快捷键在 sleep 模块固定为 Ctrl+Enter
            idle_sleep_minutes: 0,
            lock_on_sleep: false,
        }
    }
}

/// 旧版多控制点默认曲线，仅为兼容历史配置保留，新模型不再使用。
#[allow(dead_code)]
pub fn default_schedule_points() -> Vec<SchedulePoint> {
    vec![
        SchedulePoint { minute: 8 * 60, temperature: 5800, brightness: 1.0 },
        SchedulePoint { minute: 18 * 60, temperature: 4800, brightness: 0.9 },
        SchedulePoint { minute: 22 * 60, temperature: 3800, brightness: 0.72 },
        SchedulePoint { minute: 0, temperature: 3200, brightness: 0.58 },
    ]
}

/// `appDirectoriesOverride` 将配置目录指向安装目录下的 `data\`，
/// 因此这里通过 Tauri 的路径 API 获取，保证安装版与绿色版行为一致。
fn config_path(app: &AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let _ = std::fs::create_dir_all(&dir);
    dir.join("settings.json")
}

pub fn load_from(app: &AppHandle) -> Settings {
    let text = match std::fs::read_to_string(config_path(app)) {
        Ok(text) => text,
        Err(_) => return Settings::default(),
    };
    let mut settings = serde_json::from_str::<Settings>(&text).unwrap_or_default();
    // 老版本配置没有 softness 字段，且仍停留在旧的偏亮偏冷默认值时，
    // 平滑迁移到新护眼默认；用户自定义过的参数（模板/开关等）保持不变。
    let is_legacy_default = !text.contains("\"softness\"")
        && (settings.brightness - 1.0).abs() < 0.001
        && settings.temperature == 5800;
    if is_legacy_default {
        let defaults = Settings::default();
        settings.brightness = defaults.brightness;
        settings.temperature = defaults.temperature;
        settings.softness = defaults.softness;
        settings.template_id = defaults.template_id;
    }
    // 老配置缺少昼夜两态字段时（值为 0），用 LightBulb 风格默认值补齐。
    // 用 0 作为“未初始化”标志：这些字段的合理取值均 > 0。
    if settings.day_temperature == 0 { settings.day_temperature = 6500; }
    if settings.day_brightness <= 0.0 { settings.day_brightness = 1.0; }
    if settings.night_temperature == 0 { settings.night_temperature = 3900; }
    if settings.night_brightness <= 0.0 { settings.night_brightness = 0.85; }
    if settings.sunrise_minute == 0 { settings.sunrise_minute = 7 * 60 + 20; }
    if settings.sunset_minute == 0 { settings.sunset_minute = 18 * 60 + 30; }
    if settings.transition_minutes == 0 { settings.transition_minutes = 40; }
    // 注意 idle_sleep_minutes=0 是有效值（沿用系统设置），不能当未初始化处理。
    settings
}

#[tauri::command]
pub fn load_settings(app: AppHandle) -> Result<Settings, String> {
    Ok(load_from(&app))
}

#[tauri::command]
pub fn save_settings(app: AppHandle, mut settings: Settings) -> Result<(), String> {
    // 服务端兜底：自定义模板最多 3 个
    if settings.custom_templates.len() > 3 {
        settings.custom_templates.truncate(3);
    }
    // 后端兜底：空闲时长上限 24 小时，避免异常配置导致计时溢出。
    settings.idle_sleep_minutes = settings.idle_sleep_minutes.min(1440);
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(config_path(&app), text).map_err(|e| e.to_string())?;
    set_current(&settings);
    Ok(())
}
