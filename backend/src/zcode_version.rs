//! 本机 ZCode 客户端版本与 Jolu 适配版本的比对（C6 / P0-4）。
//!
//! Start Plan 通道与余额查询都按 ZCode 客户端的样子发请求（`zcode_quota::ZCODE_APP_VERSION`）。
//! ZCode 升级后供应商可能按新版本的请求样式做风控（405 / 3012），余额端点也按 `app_version`
//! 判定礼包能力——本机版本比 Jolu 适配的版本新时要提示，而不是等请求被拒才发现。
//! 版本号只从卸载注册表读（ZCode 自己的数据目录里没有记录），读不到就如实说不知道。

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionCheck {
    /// 本机安装的 ZCode 版本；没装或读不到为 None
    pub installed: Option<String>,
    /// Jolu 当前适配的版本
    pub expected: String,
    /// 本机版本比适配版本新（需要更新 Jolu 的着装常量）
    pub newer: bool,
}

fn parse(version: &str) -> Option<Vec<u64>> {
    let parts: Option<Vec<u64>> = version.trim().trim_start_matches('v').split('.').map(|p| p.trim().parse().ok()).collect();
    parts.filter(|parts| !parts.is_empty())
}

/// `installed` 是否比 `expected` 新；任一方解析不了按「不新」处理（不知道就不报警）。
pub fn is_newer(installed: &str, expected: &str) -> bool {
    match (parse(installed), parse(expected)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

pub fn check(installed: Option<String>) -> VersionCheck {
    let expected = crate::zcode_quota::ZCODE_APP_VERSION.to_string();
    let newer = installed.as_deref().is_some_and(|version| is_newer(version, &expected));
    VersionCheck { installed, expected, newer }
}

/// 从卸载注册表（当前用户与本机）找显示名以「ZCode」开头的条目，读它的 DisplayVersion。
#[cfg(windows)]
pub fn installed_version() -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        RRF_RT_REG_SZ,
    };
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
    fn value(key: HKEY, name: &str) -> Option<String> {
        let name = wide(name);
        let mut buffer = vec![0u16; 512];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: buffer 与 size 描述同一块可写内存；RegGetValueW 只在其中写入以 0 结尾的字符串。
        let status = unsafe {
            RegGetValueW(key, std::ptr::null(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size)
        };
        if status != 0 {
            return None;
        }
        let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..len]))
    }
    let path = wide(r"Software\Microsoft\Windows\CurrentVersion\Uninstall");
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut uninstall: HKEY = std::ptr::null_mut();
        // SAFETY: 只读打开固定路径；成功后由下面的 RegCloseKey 关闭。
        if unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ, &mut uninstall) } != 0 {
            continue;
        }
        let mut found = None;
        for index in 0.. {
            let mut name = vec![0u16; 256];
            let mut len = name.len() as u32;
            // SAFETY: name / len 描述同一块可写内存；其余输出参数不需要，传空指针。
            let status = unsafe {
                RegEnumKeyExW(uninstall, index, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
            };
            if status != 0 {
                break;
            }
            let mut entry: HKEY = std::ptr::null_mut();
            // SAFETY: name 是 RegEnumKeyExW 写入的以 0 结尾的子键名；打开成功后立即关闭。
            if unsafe { RegOpenKeyExW(uninstall, name.as_ptr(), 0, KEY_READ, &mut entry) } != 0 {
                continue;
            }
            let is_zcode = value(entry, "DisplayName").is_some_and(|display| display.trim_start().starts_with("ZCode"));
            let version = if is_zcode { value(entry, "DisplayVersion") } else { None };
            // SAFETY: entry 由上面的 RegOpenKeyExW 成功打开。
            unsafe { RegCloseKey(entry) };
            if version.is_some() {
                found = version;
                break;
            }
        }
        // SAFETY: uninstall 由上面的 RegOpenKeyExW 成功打开。
        unsafe { RegCloseKey(uninstall) };
        if found.is_some() {
            return found;
        }
    }
    None
}

#[cfg(not(windows))]
pub fn installed_version() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_compares_numerically_and_unknown_never_alarms() {
        assert!(is_newer("3.18.0", "3.17.2"));
        assert!(is_newer("3.17.10", "3.17.2"), "按数字比，不按字符串比");
        assert!(!is_newer("3.17.2", "3.17.2"));
        assert!(!is_newer("3.14.3", "3.17.2"));
        assert!(!is_newer("beta", "3.17.2"));
        assert!(!is_newer("", "3.17.2"));
    }

    #[test]
    #[ignore = "读本机注册表；仅显式只读执行"]
    fn local_installed_version_readonly_smoke() {
        println!("ZCode installed: {:?}", installed_version());
    }

    #[test]
    fn check_reports_expected_version_and_unknown_install() {
        let missing = check(None);
        assert_eq!(missing.expected, crate::zcode_quota::ZCODE_APP_VERSION);
        assert!(!missing.newer);
        assert!(check(Some("99.0.0".into())).newer);
    }
}
