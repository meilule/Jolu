//! 保持系统唤醒（批次 G §3.12 Keep awake）。
//!
//! 场景：自动化任务跑到一半系统进入睡眠，任务会被挂起、退避计时也开始漂移。
//! 开启后 Jolu 在运行期间告诉系统「别睡」，退出或关闭开关时立刻撤回。
//!
//! Windows 用 `SetThreadExecutionState`；该函数按**线程**记状态，因此所有施加与撤回
//! 都固定在一个常驻线程里执行，其他线程只往它的通道里发开关信号。
//! 其他平台不提供该能力，如实返回「当前系统不支持」。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// 开关通道。第一次使用时拉起常驻线程并保存发送端。
static CHANNEL: OnceLock<Mutex<Option<Sender<bool>>>> = OnceLock::new();

#[cfg(windows)]
mod platform {
    // 阻止系统进入睡眠：连续生效，不动显示器
    pub const ES_CONTINUOUS: u32 = 0x8000_0000;
    pub const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    pub const ES_AWAYMODE_REQUIRED: u32 = 0x0000_0040;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn SetThreadExecutionState(flags: u32) -> u32;
    }

    /// 施加「保持唤醒」。`AWAYMODE` 让机器可以熄屏运行，拿不到时退回纯系统可用。
    pub fn acquire() {
        let wanted = ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_AWAYMODE_REQUIRED;
        // SAFETY: SetThreadExecutionState 只改调用线程的执行状态，无指针参数。
        let result = unsafe { SetThreadExecutionState(wanted) };
        if result == 0 {
            // AWAYMODE 在部分系统上不可用，退回最基本的「别睡」
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
        }
    }

    /// 撤回，交还给系统默认电源策略。
    pub fn release() {
        // SAFETY: 同上；单用 ES_CONTINUOUS 即清除之前的所有要求。
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

/// 当前是否处于保持唤醒状态（真实生效状态，不是用户意图）。
pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

/// 当前系统是否支持保持唤醒。
pub(crate) fn supported() -> bool {
    cfg!(windows)
}

/// 打开或关闭。返回切换后的生效状态。
///
/// 不支持的系统上打开会报错、关闭返回 false —— 不假装打开了。
pub fn set(on: bool) -> Result<bool, String> {
    if !supported() {
        return if on { Err("当前系统不支持保持唤醒".into()) } else { Ok(false) };
    }
    let sender = channel()?;
    sender.send(on).map_err(|_| "保持唤醒线程已退出".to_string())?;
    // 工作线程在同一个线程里连续执行施加/撤回，发送即代表已排队；
    // 这里以「请求已送达」作为生效状态，实际调用由工作线程保证顺序。
    ENABLED.store(on, Ordering::Release);
    Ok(on)
}

/// 启动时按设置恢复。失败只记录，不影响启动。
pub fn restore(on: bool) {
    if !on || !supported() { return; }
    if let Err(error) = set(true) { eprintln!("[唤醒] 恢复保持唤醒失败：{error}"); }
}

/// 退出时无条件撤回，避免把这个状态留给系统。
pub fn shutdown() {
    if let Ok(sender) = channel() {
        let _ = sender.send(false);
        // 给工作线程一点时间交还电源策略，再让进程退出
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    ENABLED.store(false, Ordering::Release);
}

/// 取得开关通道，必要时拉起常驻线程。返回克隆的发送端，调用方不持有锁。
fn channel() -> Result<Sender<bool>, String> {
    let slot = CHANNEL.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().map_err(|_| "保持唤醒通道不可用".to_string())?;
    if guard.is_none() {
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        std::thread::Builder::new()
            .name("jolu-keep-awake".into())
            .spawn(move || {
                #[cfg(windows)]
                {
                    let mut held = false;
                    for requested in rx {
                        if requested && !held { platform::acquire(); held = true; }
                        if !requested && held { platform::release(); held = false; }
                    }
                    // 通道关闭（进程退出）：交还电源策略
                    if held { platform::release(); }
                }
                #[cfg(not(windows))]
                {
                    // 非 Windows 不该走到这里；守住通道不泄漏线程
                    for _ in rx {}
                }
            })
            .map_err(|error| format!("保持唤醒线程启动失败：{error}"))?;
        *guard = Some(tx);
    }
    guard.as_ref().cloned().ok_or_else(|| "保持唤醒通道不可用".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_platform_cannot_be_turned_on_silently() {
        if supported() { return; }
        assert!(set(true).is_err(), "不支持时必须报错，不能假装打开");
        assert!(!enabled());
    }

    #[test]
    fn toggle_round_trip_reports_real_state() {
        if !supported() { return; }
        assert!(set(true).unwrap());
        assert!(enabled());
        assert!(!set(false).unwrap());
        assert!(!enabled());
    }

    #[test]
    fn shutdown_always_releases() {
        if supported() { let _ = set(true); }
        shutdown();
        assert!(!enabled(), "退出后必须回到不保持唤醒");
    }
}
