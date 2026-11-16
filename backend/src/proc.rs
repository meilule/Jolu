//! 子进程的统一结束方式（4.9 / P2-3，magpie `internal/proc` 的「树杀 / 超时」部分）。
//!
//! 为什么要杀整棵树：Windows 上 `child.kill()`（TerminateProcess）只结束这一个进程，它起的
//! 子进程会变成孤儿继续跑。npm 装的 CLI 是 `.cmd` 垫片，真正干活的 `node` 是 `cmd.exe` 的子进程：
//! warm-up 超时只杀掉 `cmd.exe`，`node` / `codex` 照样把请求发完；System32 的 `wsl.exe` 会再起
//! Program Files 的 `wsl.exe` 并等它，只杀外层就每次留一个（magpie #2341，一天几十个）。
//!
//! 用法：
//! - [`wait_or_kill_tree`]：轮询等子进程结束，超时结束整棵树；
//! - [`kill_child_tree`] / [`kill_tree`]：取消时结束整棵树。
//!
//! 不适用的地方（各处注释写明了原因）：
//! - 官方登录进程（`cli_browser_auth`）：它打开的浏览器归用户所有，取消登录时不能跟着关；
//! - 网关里的 Claude 一次性进程与桥接（`gateway/adapter/claude*.rs`）：只认原生 `claude.exe`、
//!   工具与 MCP 都关掉，没有子进程可留，沿用 tokio 的 `kill_on_drop`。
//!
//! Windows 用 `taskkill /T /F`（按父进程关系找全部后代，要在父进程还活着时调用）；
//! 其他系统发 SIGTERM 给该进程。参照 magpie `proc/killtree_windows.go`（它自己遍历进程快照，
//! Jolu 沿用项目里已验证过的 taskkill）。

use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

/// 结束 `pid` 及它起的全部子进程；等 taskkill 跑完再返回。进程已经不在时什么也不做。
pub(crate) fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    }
}

/// 结束这个子进程的整棵树，并回收它（不留僵尸）。
pub(crate) fn kill_child_tree(child: &mut Child) {
    kill_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

/// 每隔 `poll` 看一次子进程是否结束，最多等 `timeout`。
/// 正常结束返回它的退出状态；超时（或读不了状态）就结束整棵树并返回 None。
pub(crate) fn wait_or_kill_tree(child: &mut Child, timeout: Duration, poll: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(poll),
            _ => {
                kill_child_tree(child);
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 正常结束：返回退出状态，不杀
    #[cfg(windows)]
    #[test]
    fn finished_child_returns_status() {
        let mut child = std::process::Command::new("cmd").args(["/C", "exit 3"]).spawn().unwrap();
        let status = wait_or_kill_tree(&mut child, Duration::from_secs(10), Duration::from_millis(20)).expect("应正常结束");
        assert_eq!(status.code(), Some(3));
    }

    /// 外层 cmd 起一个内层 cmd（孙进程）：内层 ping 约 2 秒后写标记文件。
    /// 用 `raw_arg` 原样传命令行——`args` 会把引号转义成 `\"`，cmd 不认，内层根本跑不起来
    /// （那样「杀整棵树」的测试会白通过）。
    #[cfg(windows)]
    fn spawn_nested(tag: &str) -> (Child, std::path::PathBuf, std::path::PathBuf) {
        use std::os::windows::process::CommandExt;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("jolu-proc-{tag}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("grandchild-finished.txt");
        let line = format!("/C cmd /C \"ping -n 3 127.0.0.1 >NUL & echo done>\"{}\"\"", marker.display());
        let child = std::process::Command::new("cmd").raw_arg(line).spawn().unwrap();
        (child, dir, marker)
    }

    /// 测试场景本身成立：不打断时，孙进程确实会写出标记文件
    #[cfg(windows)]
    #[test]
    fn nested_grandchild_writes_marker_when_left_alone() {
        let (mut child, dir, marker) = spawn_nested("alone");
        let status = child.wait().unwrap();
        assert!(status.success() && marker.exists(), "内层命令应能跑完并写出标记：{status:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 对照：只用 `child.kill()` 结束外层时，孙进程会留下来照样写出标记文件——
    /// 证明下面的场景确实需要杀整棵树
    #[cfg(windows)]
    #[test]
    fn plain_kill_leaves_grandchild_running() {
        let (mut child, dir, marker) = spawn_nested("plain");
        std::thread::sleep(Duration::from_millis(300));
        let _ = child.kill();
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(2_800));
        assert!(marker.exists(), "只杀外层时孙进程会继续跑完");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 超时：外层 cmd 起的内层 cmd（孙进程）也要一起结束，标记文件不会写出来
    #[cfg(windows)]
    #[test]
    fn timeout_kills_grandchildren_too() {
        let (mut child, dir, marker) = spawn_nested("tree");
        let started = Instant::now();
        assert!(wait_or_kill_tree(&mut child, Duration::from_millis(300), Duration::from_millis(20)).is_none(), "应判超时");
        assert!(started.elapsed() < Duration::from_secs(2), "超时后应马上结束");
        // 等过孙进程原本写文件的时刻
        std::thread::sleep(Duration::from_millis(2_800));
        assert!(!marker.exists(), "孙进程应已被一起结束，不该写出标记文件");
        std::fs::remove_dir_all(&dir).ok();
    }
}
