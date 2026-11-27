//! 无文件锁 CLI 平台的轻量级本机进程存活核验（消除终端退出后的 90 秒假活跃尾巴）。
//!
//! 严格安全边界（对齐 `session_presence` 权威原则）：
//! - **只剔除确证已退出的平台**：仅当系统进程快照读取成功、不存在目标原生可执行文件，且所有运行中的脚本解释器
//!   （`node.exe` / `bun.exe` / `python.exe`）命令行均成功读取且不含目标 CLI 特征时，才返回 `Some(false)`；
//! - **任何不确定一律返回 `None`（Unknown）**：快照失败、某个管理员权限 `node.exe` 无法读取命令行、或非 Windows 平台，
//!   均保留原有 90 秒最近活跃窗口，**绝不把权限不足或静默思考误判为退出**；
//! - 零外部子进程（不拉起 `powershell` / `tasklist` / `wmic`），纯内核快照 + 1.5s 内存缓存，单次开销 < 0.5ms。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const SNAPSHOT_TTL: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone)]
pub(crate) struct ProcessEntry {
    pub exe_lower: String,
    /// 仅对脚本解释器进程按需读取；`None` 表示读取失败（如权限受限），须按未知处理。
    pub cmdline_lower: Option<String>,
}

struct ToolSignature {
    native_exes: &'static [&'static str],
    interpreters: &'static [&'static str],
    cmdline_markers: &'static [&'static str],
}

fn signature_of(platform: &str) -> Option<ToolSignature> {
    Some(match platform {
        "opencode" => ToolSignature {
            native_exes: &["opencode.exe", "opencode"],
            interpreters: &["bun.exe", "node.exe"],
            cmdline_markers: &["opencode"],
        },
        "antigravity" => ToolSignature {
            native_exes: &[
                "agy.exe",
                "antigravity.exe",
                "antigravity-cli.exe",
                "language_server_windows_x64.exe",
                "agy",
                "antigravity",
            ],
            interpreters: &["node.exe", "electron.exe"],
            cmdline_markers: &["antigravity"],
        },
        "kimi" => ToolSignature {
            native_exes: &["kimi.exe", "kimi-code.exe", "kimi", "kimi-code"],
            interpreters: &["python.exe", "python3.exe", "pythonw.exe", "node.exe", "bun.exe"],
            cmdline_markers: &["kimi"],
        },
        "pi" => ToolSignature {
            native_exes: &["pi.exe", "pi"],
            interpreters: &["node.exe", "bun.exe"],
            cmdline_markers: &["pi-coding-agent", "@mariozechner/pi", "\\pi\\", "/pi/", "\\pi.js", "/pi.js"],
        },
        "dsh" => ToolSignature {
            native_exes: &["dsh.exe", "dsh"],
            interpreters: &["node.exe", "bun.exe"],
            cmdline_markers: &["deepseek-harness", "@deepseek-ai/dsh", "\\dsh\\", "/dsh/", "\\dsh.js"],
        },
        "gemini" => ToolSignature {
            native_exes: &["gemini.exe", "gemini"],
            interpreters: &["node.exe", "bun.exe"],
            cmdline_markers: &["gemini-cli", "@google/gemini", "\\gemini\\", "/gemini/", "gemini.js"],
        },
        "qwen" => ToolSignature {
            native_exes: &["qwen.exe", "qwen"],
            interpreters: &["node.exe", "bun.exe"],
            cmdline_markers: &["qwen-code", "@qwen-code", "\\qwen\\", "/qwen/", "qwen.js"],
        },
        _ => return None,
    })
}

pub(crate) fn is_lockless_cli_platform(platform: &str) -> bool {
    signature_of(platform).is_some()
}

/// 在给定进程快照中判定目标平台是否仍有宿主进程存活（纯函数，供单测直接驱动）。
pub(crate) fn platform_process_running_in(platform: &str, procs: &[ProcessEntry]) -> Option<bool> {
    let sig = signature_of(platform)?;
    if procs.iter().any(|p| sig.native_exes.contains(&p.exe_lower.as_str())) {
        return Some(true);
    }
    let mut had_unreadable_interpreter = false;
    for p in procs {
        if !sig.interpreters.contains(&p.exe_lower.as_str()) {
            continue;
        }
        match p.cmdline_lower.as_deref() {
            Some(cmd) => {
                if sig.cmdline_markers.iter().any(|m| cmd.contains(m)) {
                    return Some(true);
                }
            }
            None => {
                had_unreadable_interpreter = true;
            }
        }
    }
    if had_unreadable_interpreter {
        None
    } else {
        Some(false)
    }
}

/// 查询本机当前是否仍有该 CLI 平台进程存活（带 1.5s 短缓存；测试与隔离验收环境返回 `None` 不读宿主机进程）。
pub(crate) fn platform_process_running(platform: &str) -> Option<bool> {
    if cfg!(test) || crate::validation::isolated() {
        return None;
    }
    let _ = signature_of(platform)?;
    static CACHE: OnceLock<Mutex<Option<(Instant, Option<Vec<ProcessEntry>>)>>> = OnceLock::new();
    let cell = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let need_refresh = guard
        .as_ref()
        .is_none_or(|(at, _)| at.elapsed() >= SNAPSHOT_TTL);
    if need_refresh {
        *guard = Some((Instant::now(), snapshot_processes()));
    }
    let snapshot = guard.as_ref().and_then(|(_, snap)| snap.as_ref())?;
    platform_process_running_in(platform, snapshot)
}

#[cfg(not(windows))]
fn snapshot_processes() -> Option<Vec<ProcessEntry>> {
    None
}

#[cfg(windows)]
const INTERPRETERS: &[&str] = &[
    "node.exe",
    "bun.exe",
    "python.exe",
    "python3.exe",
    "pythonw.exe",
    "electron.exe",
];

#[cfg(windows)]
fn snapshot_processes() -> Option<Vec<ProcessEntry>> {
    use std::ffi::c_void;

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;

    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, th32_process_id: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, lppe: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: *mut c_void, lppe: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap.is_null() || snap == INVALID_HANDLE_VALUE {
        return None;
    }

    let mut raw = ProcessEntry32W {
        dw_size: std::mem::size_of::<ProcessEntry32W>() as u32,
        cnt_usage: 0,
        th32_process_id: 0,
        th32_default_heap_id: 0,
        th32_module_id: 0,
        cnt_threads: 0,
        th32_parent_process_id: 0,
        pc_pri_class_base: 0,
        dw_flags: 0,
        sz_exe_file: [0; 260],
    };

    let mut out = Vec::with_capacity(128);
    let mut ok = unsafe { Process32FirstW(snap, &mut raw) };
    while ok != 0 {
        let len = raw.sz_exe_file.iter().position(|&c| c == 0).unwrap_or(raw.sz_exe_file.len());
        let exe_lower = String::from_utf16_lossy(&raw.sz_exe_file[..len]).to_ascii_lowercase();
        let pid = raw.th32_process_id;
        let cmdline_lower = if pid > 4 && INTERPRETERS.contains(&exe_lower.as_str()) {
            query_process_cmdline(pid).map(|s| s.to_ascii_lowercase())
        } else {
            Some(String::new())
        };
        out.push(ProcessEntry { exe_lower, cmdline_lower });
        ok = unsafe { Process32NextW(snap, &mut raw) };
    }
    unsafe {
        CloseHandle(snap);
    }
    Some(out)
}

#[cfg(windows)]
fn query_process_cmdline(pid: u32) -> Option<String> {
    use std::ffi::c_void;

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;
    const STATUS_INFO_LENGTH_MISMATCH: i32 = -1073741820; // 0xC0000004
    const STATUS_BUFFER_OVERFLOW: i32 = -2147483643; // 0x80000005
    const STATUS_BUFFER_TOO_SMALL: i32 = -1073741789; // 0xC0000023

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *const u16,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            process_handle: *mut c_void,
            process_information_class: u32,
            process_information: *mut u8,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }

    let mut needed: u32 = 0;
    let mut buf = vec![0u8; 1024];
    let mut status = unsafe {
        NtQueryInformationProcess(
            handle,
            PROCESS_COMMAND_LINE_INFORMATION,
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut needed,
        )
    };
    if matches!(
        status,
        STATUS_INFO_LENGTH_MISMATCH | STATUS_BUFFER_OVERFLOW | STATUS_BUFFER_TOO_SMALL
    ) && needed > buf.len() as u32
        && needed <= 131_072
    {
        buf.resize(needed as usize, 0);
        status = unsafe {
            NtQueryInformationProcess(
                handle,
                PROCESS_COMMAND_LINE_INFORMATION,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut needed,
            )
        };
    }
    unsafe {
        CloseHandle(handle);
    }
    if status < 0 || buf.len() < std::mem::size_of::<UnicodeString>() {
        return None;
    }

    let ustr = unsafe { &*(buf.as_ptr() as *const UnicodeString) };
    if ustr.length == 0 || ustr.buffer.is_null() {
        return Some(String::new());
    }
    let u16_len = (ustr.length as usize) / 2;
    let buf_start = buf.as_ptr() as usize;
    let buf_end = buf_start + buf.len();
    let str_start = ustr.buffer as usize;
    let str_end = str_start.saturating_add(u16_len * 2);
    if str_start < buf_start || str_end > buf_end {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(ustr.buffer, u16_len) };
    Some(String::from_utf16_lossy(slice))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_alive_native_shim_unrelated_node_and_unreadable_elevated_node() {
        // 1) 原生二进制存活
        let procs_native = vec![ProcessEntry {
            exe_lower: "opencode.exe".into(),
            cmdline_lower: Some(String::new()),
        }];
        assert_eq!(platform_process_running_in("opencode", &procs_native), Some(true));

        // 2) PowerShell / npm node.exe 垫片运行 Qwen Code，同时有无关 vite 的 node.exe
        let procs_node_qwen = vec![
            ProcessEntry {
                exe_lower: "node.exe".into(),
                cmdline_lower: Some("c:\\program files\\nodejs\\node.exe d:\\develop\\jolu\\node_modules\\vite\\bin\\vite.js".into()),
            },
            ProcessEntry {
                exe_lower: "node.exe".into(),
                cmdline_lower: Some("node.exe c:\\users\\demo\\appdata\\roaming\\npm\\node_modules\\@qwen-code\\qwen-code\\cli.js".into()),
            },
        ];
        assert_eq!(platform_process_running_in("qwen", &procs_node_qwen), Some(true));
        // 同一份快照里 Kimi 没有任何进程，哪怕有无关 node.exe 也能精确判定已退出！
        assert_eq!(platform_process_running_in("kimi", &procs_node_qwen), Some(false));

        // 3) 若存在权限不足无法读取命令行的管理员 node.exe，绝不误判为退出（返回 None 保留会话）
        let procs_elevated = vec![ProcessEntry {
            exe_lower: "node.exe".into(),
            cmdline_lower: None,
        }];
        assert_eq!(platform_process_running_in("qwen", &procs_elevated), None);
        // 不依赖 node.exe 的 opencode（此处仅配了 bun/node：node 不可读也保守返回 None）
        assert_eq!(platform_process_running_in("opencode", &procs_elevated), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_queries_current_process_command_line_without_spawning_subprocesses() {
        let cmd = query_process_cmdline(std::process::id()).expect("own process cmdline must be readable");
        assert!(!cmd.is_empty(), "current test binary command line should not be empty");
    }
}
