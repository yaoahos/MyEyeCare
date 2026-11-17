//! 开机自启：直接读写当前用户的 Run 注册表项，
//! 不引入额外插件，安装包体积和后台占用都保持最小。

#[cfg(all(target_os = "windows", not(feature = "portable")))]
mod imp {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;

    type HKey = *mut c_void;
    const HKEY_CURRENT_USER: HKey = 0x8000_0001usize as HKey;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const KEY_SET_VALUE: u32 = 0x0002;
    const REG_SZ: u32 = 1;
    const ERROR_SUCCESS: i32 = 0;
    const APP_NAME: &str = "MyEyeCare";

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            hkey: HKey,
            sub_key: *const u16,
            options: u32,
            desired: u32,
            result: *mut HKey,
        ) -> i32;
        fn RegSetValueExW(
            hkey: HKey,
            name: *const u16,
            reserved: u32,
            value_type: u32,
            data: *const u8,
            len: u32,
        ) -> i32;
        fn RegDeleteValueW(hkey: HKey, name: *const u16) -> i32;
        fn RegQueryValueExW(
            hkey: HKey,
            name: *const u16,
            reserved: *mut u32,
            value_type: *mut u32,
            data: *mut u8,
            len: *mut u32,
        ) -> i32;
        fn RegCloseKey(hkey: HKey) -> i32;
    }

    fn wide(value: &str) -> Vec<u16> {
        let mut buf: Vec<u16> = OsStr::new(value).encode_wide().collect();
        buf.push(0);
        buf
    }

    fn open_run_key(desired: u32) -> Option<HKey> {
        let sub_key = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
        let mut handle: HKey = std::ptr::null_mut();
        let code = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                sub_key.as_ptr(),
                0,
                desired,
                &mut handle,
            )
        };
        if code == ERROR_SUCCESS {
            Some(handle)
        } else {
            None
        }
    }

    pub fn is_enabled() -> bool {
        let Some(hkey) = open_run_key(KEY_QUERY_VALUE) else {
            return false;
        };
        let name = wide(APP_NAME);
        let mut value_type = 0u32;
        let mut buffer = [0u16; 1024];
        let mut len = (buffer.len() * 2) as u32;
        let code = unsafe {
            RegQueryValueExW(
                hkey,
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut value_type,
                buffer.as_mut_ptr() as *mut u8,
                &mut len,
            )
        };
        unsafe {
            RegCloseKey(hkey);
        }
        code == ERROR_SUCCESS
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        let hkey = open_run_key(KEY_SET_VALUE).ok_or_else(|| "无法打开启动项注册表".to_string())?;
        let name = wide(APP_NAME);
        let result = if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let command = format!("\"{}\"", exe.to_string_lossy());
            let data = wide(&command);
            unsafe {
                RegSetValueExW(
                    hkey,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    data.as_ptr() as *const u8,
                    (data.len() * 2) as u32,
                )
            }
        } else {
            unsafe { RegDeleteValueW(hkey, name.as_ptr()) }
        };
        unsafe {
            RegCloseKey(hkey);
        }
        const ERROR_FILE_NOT_FOUND: i32 = 2;
        // 仅「删除不存在的项」视为成功；其他错误（如拒绝访问）必须如实上报
        if result == ERROR_SUCCESS || (!enabled && result == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            let action = if enabled { "写入" } else { "删除" };
            Err(format!("{}启动项失败，错误码 {}", action, result))
        }
    }
}

#[cfg(any(not(target_os = "windows"), feature = "portable"))]
mod imp {
    pub fn is_enabled() -> bool {
        false
    }

    pub fn set_enabled(_enabled: bool) -> Result<(), String> {
        #[cfg(feature = "portable")]
        {
            Err("绿色便携版不写入开机自启，避免注册表残留".to_string())
        }
        #[cfg(not(feature = "portable"))]
        {
            Err("当前平台不支持开机自启".to_string())
        }
    }
}

#[tauri::command]
pub fn is_autostart_enabled() -> Result<bool, String> {
    Ok(imp::is_enabled())
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    imp::set_enabled(enabled)
}

/// 启动时按配置同步开机自启，让安装版首次运行默认开启。
pub fn apply_saved_state(enabled: bool) -> Result<(), String> {
    imp::set_enabled(enabled)
}
