//! 软件内自更新：检查 GitHub Release、下载安装包并启动安装。
//! 参照 SafePasswordDesktop 的实现，所有面向用户的文案集中、下载源严格白名单校验。

use serde::Serialize;

/// 更新信息（返回给前端渲染弹窗）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub has_update: bool,
    pub url: String,
    /// 安装包直链（优先 Setup.exe；找不到时为 nil，前端退回浏览器）
    pub asset_url: String,
    pub asset_size: u64,
    pub notes: String,
}

fn version_tuple(v: &str) -> (u32, u32, u32) {
    let clean = v.trim().trim_start_matches(['v', 'V']);
    let mut parts = clean.split('.').map(|p| p.trim().parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("MyEyeCare-Desktop")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("初始化网络失败：{e}"))
}

/// 检查更新。GitHub API 返回 release JSON，解析出版本号、日志与安装包直链。
#[tauri::command]
pub async fn check_update(app_version: String) -> Result<UpdateInfo, String> {
    let client = client()?;
    let url = "https://api.github.com/repos/yaoahos/MyEyeCare/releases/latest";
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|_| "连接不上更新服务，请检查网络或代理".to_string())?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("还没有可更新的版本".to_string());
    }
    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Err("更新检查已被限制，请稍后再试".to_string());
    }
    if !resp.status().is_success() {
        return Err("更新服务暂时不可用，请稍后再试".to_string());
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|_| "更新服务返回内容异常，请稍后再试".to_string())?;

    let tag = body.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
    let html_url = body.get("html_url").and_then(|v| v.as_str()).unwrap_or("");
    let notes = body.get("body").and_then(|v| v.as_str()).unwrap_or("");
    // 只选安装版 exe（*-Setup.exe），绿色版 exe 不能原地安装
    let (asset_url, asset_size) = body
        .get("assets")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .map(|a| {
                    (
                        a.get("browser_download_url")
                            .and_then(|u| u.as_str())
                            .unwrap_or(""),
                        a.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                    )
                })
                .find(|(u, _)| {
                    let lower = u.to_ascii_lowercase();
                    lower.ends_with("-setup.exe") || lower.ends_with("_setup.exe")
                })
        })
        .map(|(u, s)| (u.to_string(), s))
        .unwrap_or_default();

    let latest = tag.trim_start_matches(['v', 'V']).to_string();
    let has_update = version_tuple(&latest) > version_tuple(&app_version);
    Ok(UpdateInfo {
        current: app_version,
        latest,
        has_update,
        url: html_url.to_string(),
        asset_url,
        asset_size,
        notes: notes.to_string(),
    })
}

/// 下载安装包并启动安装程序（软件内自更新）。
/// 安全约束：仅 GitHub 官方域名、文件名白名单、强制 exe 后缀、校验 MZ 头与最小体积。
#[tauri::command]
pub async fn update_download_and_install(asset_url: String) -> Result<String, String> {
    if !asset_url.starts_with("https://") {
        return Err("安装包地址无效".to_string());
    }
    let host = asset_url["https://".len()..]
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    let host_ok = host.eq_ignore_ascii_case("github.com")
        || host.eq_ignore_ascii_case("objects.githubusercontent.com")
        || host.eq_ignore_ascii_case("github-releases.githubusercontent.com");
    if !host_ok {
        return Err("安装包地址无效".to_string());
    }

    let client = client()?;
    let resp = client
        .get(&asset_url)
        .send()
        .await
        .map_err(|_| "下载失败，请检查网络后重试".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("下载失败（HTTP {}）", resp.status().as_u16()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|_| "读取安装包失败，请重试".to_string())?;

    let dir = std::env::temp_dir().join("MyEyeCare-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败：{e}"))?;
    // 文件名白名单：只留安全字符并强制 exe 后缀，避免 ".."、分隔符等造成路径穿越
    let raw_name = asset_url.rsplit('/').next().unwrap_or("MyEyeCare-setup.exe");
    let name: String = raw_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    if !name.to_ascii_lowercase().ends_with(".exe") {
        return Err("安装包格式不支持".to_string());
    }
    // 校验文件头，避免把错误页 / 被替换的内容当安装包执行
    if !bytes.starts_with(b"MZ") || bytes.len() < 512 * 1024 {
        return Err("安装包内容异常，已取消安装".to_string());
    }
    let path = dir.join(name);
    std::fs::write(&path, &bytes).map_err(|e| format!("写入安装包失败：{e}"))?;
    std::process::Command::new(&path)
        .spawn()
        .map_err(|e| format!("启动安装程序失败：{e}"))?;
    Ok(path.to_string_lossy().to_string())
}
