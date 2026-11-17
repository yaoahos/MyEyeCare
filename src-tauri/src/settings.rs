use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// 用户自定义护眼模板（最多保存 3 个）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomTemplate {
    pub id: String,
    pub name: String,
    pub temperature: u32,
    pub brightness: f64,
    /// 该模板是否带柔色遮罩
    pub overlay_enabled: bool,
    /// 遮罩颜色（#RRGGBB）
    pub overlay_color: String,
    /// 遮罩强度 0~100
    pub overlay_strength: u32,
}

impl Default for CustomTemplate {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            temperature: 6500,
            brightness: 1.0,
            overlay_enabled: false,
            overlay_color: "#FFD98A".to_string(),
            overlay_strength: 18,
        }
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
    /// 是否启用全屏柔色遮罩（gamma 之外的增强手段）
    pub overlay_enabled: bool,
    /// 遮罩强度 0~100
    pub overlay_strength: u32,
    /// 遮罩颜色（#RRGGBB）
    pub overlay_color: String,
    /// 用户保存的自定义模板（最多 3 个）
    pub custom_templates: Vec<CustomTemplate>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            brightness: 1.0,
            temperature: 5800,
            enabled: true,
            autostart_enabled: true,
            start_minimized: true,
            check_updates_on_startup: true,
            template_id: "standard".to_string(),
            overlay_enabled: true,
            overlay_strength: 5,
            overlay_color: "#8FD6BC".to_string(),
            custom_templates: Vec::new(),
        }
    }
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
    match std::fs::read_to_string(config_path(app)) {
        Ok(text) => serde_json::from_str::<Settings>(&text).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
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
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(config_path(&app), text).map_err(|e| e.to_string())
}
