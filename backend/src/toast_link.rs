//! 系统通知冷启动（4.6）。
//!
//! 通知插件在 Windows 上只在提示框还在屏幕上时回报点击；进了通知中心、或 Jolu 已退出时，
//! 点通知什么也不会发生。这里改为**协议激活**：安装版发出的提示框带 `launch="jolu://open?…"`
//! 与 `activationType="protocol"`，点它由 Windows 按已注册的 `jolu:` 协议启动 Jolu：
//! - Jolu 没在运行：新进程从启动参数里读到链接，主窗口挂载后按链接定位（冷启动）；
//! - Jolu 已在运行：新进程被单实例插件拦下，参数交给已运行的实例定位。
//!
//! 开发版（`target/debug|release` 下运行）没有开始菜单快捷方式的 AppUserModelID，
//! 自建的提示框不会显示，继续走通知插件原来的路径；协议也只给安装版注册。

/// 链接指向的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkTarget {
    /// 自动化任务
    Task(i64),
    /// 供应商页的某个账号
    Connection(String),
    /// 只打开主窗口
    Main,
}

pub(crate) const SCHEME: &str = "jolu";

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = value.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// 生成链接。
pub(crate) fn link(target: &LinkTarget) -> String {
    match target {
        LinkTarget::Task(id) => format!("{SCHEME}://open?task={id}"),
        LinkTarget::Connection(id) => format!("{SCHEME}://open?connection={}", encode(id)),
        LinkTarget::Main => format!("{SCHEME}://open"),
    }
}

/// 解析链接。只认本应用生成的形状；连接 id 与通知点击同一套校验（≤128、无控制字符）。
pub(crate) fn parse(arg: &str) -> Option<LinkTarget> {
    let rest = arg.trim().trim_matches('"').strip_prefix(&format!("{SCHEME}://open"))?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let Some(query) = rest.strip_prefix('?') else {
        return rest.is_empty().then_some(LinkTarget::Main);
    };
    let (key, value) = query.split('&').next()?.split_once('=')?;
    match key {
        "task" => value.parse::<i64>().ok().filter(|id| *id > 0).map(LinkTarget::Task),
        "connection" => decode(value)
            .filter(|id| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control))
            .map(LinkTarget::Connection),
        _ => None,
    }
}

/// 从进程参数里找链接（Windows 按协议启动时链接是一个单独的参数）。
pub(crate) fn from_args<I, S>(args: I) -> Option<LinkTarget>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().find_map(|arg| parse(arg.as_ref()))
}

/// 是否安装版：开发构建（`target/debug`、`target/release` 下运行）返回 false（与通知插件同一判断）。
#[cfg(windows)]
fn installed() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let Some(dir) = exe.parent().map(|dir| dir.display().to_string().to_ascii_lowercase()) else { return false };
    !(dir.ends_with("\\target\\debug") || dir.ends_with("\\target\\release"))
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// 提示框 XML：整条点击按协议激活到 `link`。
pub(crate) fn toast_xml(title: &str, body: &str, link: &str) -> String {
    format!(
        "<toast launch=\"{}\" activationType=\"protocol\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml_escape(link),
        xml_escape(title),
        xml_escape(body)
    )
}

/// 安装版用自建提示框发送；返回 false 表示没有发（开发版或发送失败），调用方改走通知插件。
#[cfg(windows)]
pub(crate) fn show(app_id: &str, title: &str, body: &str, target: &LinkTarget) -> bool {
    if !installed() {
        return false;
    }
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    let result = (|| -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(toast_xml(title, body, &link(target))))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?.Show(&toast)
    })();
    match result {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[通知] 协议激活提示框发送失败，改用插件：{error}");
            false
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn show(_app_id: &str, _title: &str, _body: &str, _target: &LinkTarget) -> bool {
    false
}

/// 安装版启动时把 `jolu:` 协议登记到当前用户（HKCU\Software\Classes\jolu），指向当前程序。
/// 每次启动都写一遍：升级或换安装位置后路径跟着更新。卸载时由安装器删除（installer.nsi 的 `Jolu:` 段）。
#[cfg(windows)]
pub(crate) fn register_protocol() {
    if !installed() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.display().to_string();
    let base = format!("Software\\Classes\\{SCHEME}");
    let entries: [(String, &str, String); 4] = [
        (base.clone(), "", "URL:Jolu".into()),
        (base.clone(), "URL Protocol", String::new()),
        (format!("{base}\\DefaultIcon"), "", format!("\"{exe}\",0")),
        (format!("{base}\\shell\\open\\command"), "", format!("\"{exe}\" \"%1\"")),
    ];
    for (key, name, value) in entries {
        if let Err(error) = set_string(&key, name, &value) {
            eprintln!("[通知] 登记 jolu: 协议失败：{error}");
            return;
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn register_protocol() {}

#[cfg(windows)]
fn set_string(key: &str, name: &str, value: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    let wide = |text: &str| text.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (key_w, name_w, value_w) = (wide(key), wide(name), wide(value));
    let mut handle: HKEY = std::ptr::null_mut();
    // SAFETY: 所有指针指向本函数内存活的、以 0 结尾的 UTF-16 缓冲；句柄用完即关。
    unsafe {
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_w.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null(),
            &mut handle,
            std::ptr::null_mut(),
        );
        if status != ERROR_SUCCESS {
            return Err(format!("打开 {key} 失败（{status}）"));
        }
        let bytes = (value_w.len() * 2) as u32;
        let status = RegSetValueExW(handle, name_w.as_ptr(), 0, REG_SZ, value_w.as_ptr().cast(), bytes);
        RegCloseKey(handle);
        if status != ERROR_SUCCESS {
            return Err(format!("写入 {key} 失败（{status}）"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_round_trip_and_reject_foreign_shapes() {
        for target in [LinkTarget::Task(42), LinkTarget::Connection("monitor:native:zcode:main".into()), LinkTarget::Connection("账号 A&B".into()), LinkTarget::Main] {
            assert_eq!(parse(&link(&target)), Some(target.clone()), "{}", link(&target));
        }
        assert_eq!(link(&LinkTarget::Connection("a:b".into())), "jolu://open?connection=a%3Ab");
        assert_eq!(parse("\"jolu://open/?task=7\""), Some(LinkTarget::Task(7)), "Windows 可能带引号与斜杠");
        for bad in ["jolu://open?task=0", "jolu://open?task=-3", "jolu://open?task=x", "jolu://evil", "https://open?task=1", "jolu://open?other=1", "jolu://open?connection=%0A", "jolu://open?connection=%ZZ"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        let long = format!("jolu://open?connection={}", "a".repeat(129));
        assert_eq!(parse(&long), None);
        assert_eq!(from_args(["C:\\Jolu\\jolu.exe", "--autostart", "jolu://open?task=9"]), Some(LinkTarget::Task(9)));
        assert_eq!(from_args(["C:\\Jolu\\jolu.exe"]), None);
    }

    #[test]
    fn toast_xml_escapes_text_and_activates_by_protocol() {
        let xml = toast_xml("任务 <A> & \"B\"", "失败：'x'", &link(&LinkTarget::Connection("c&d".into())));
        assert!(xml.contains("activationType=\"protocol\""));
        assert!(xml.contains("launch=\"jolu://open?connection=c%26d\""));
        assert!(xml.contains("任务 &lt;A&gt; &amp; &quot;B&quot;"));
        assert!(xml.contains("失败：&apos;x&apos;"));
    }
}
