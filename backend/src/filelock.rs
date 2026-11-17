//! 跨进程文件锁（N10，参照 magpie `internal/filelock`）：同一台电脑上桌面端与 `jolu` CLI
//! 改同一个文件（先读、改、再写回）时串行执行，免得后写的覆盖先写的。
//!
//! 做法：在目标旁边用 `create_new` 建 `<文件名>.jolu-lock`（里面写持有者的进程号与时间），
//! 建成功就持有锁，`Drop` 时删掉。拿不到时每 50 毫秒重试，直到超时。持有者崩溃会留下锁文件：
//! 超过 [`STALE_AFTER`] 的锁文件当作遗留物删掉——正常的读改写都在毫秒级完成。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// 锁文件超过这么久没被删掉，当作持有者已经崩溃
pub const STALE_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct FileLock {
    path: PathBuf,
}

fn lock_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".jolu-lock");
    target.with_file_name(name)
}

fn stale(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > STALE_AFTER)
}

impl FileLock {
    /// 给 `target` 加锁，最多等 `timeout`。目录不存在时先建出来。
    pub fn acquire(target: &Path, timeout: Duration) -> Result<Self, String> {
        let path = lock_path(target);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("建目录 {} 失败：{e}", parent.display()))?;
        }
        let started = Instant::now();
        loop {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "{} {}", std::process::id(), chrono::Utc::now().to_rfc3339());
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if stale(&path) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                }
                // Windows 上锁文件正被另一进程删除时会报拒绝访问：当作暂时拿不到
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
                Err(error) => return Err(format!("建锁文件 {} 失败：{error}", path.display())),
            }
            if started.elapsed() >= timeout {
                return Err(format!("{} 正被另一个 Jolu 进程修改，请稍后再试", target.display()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jolu-filelock-{}-{name}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn second_holder_waits_and_times_out_then_gets_it_after_release() {
        let target = temp("wait");
        let first = FileLock::acquire(&target, Duration::from_millis(100)).unwrap();
        assert!(lock_path(&target).exists());
        let err = FileLock::acquire(&target, Duration::from_millis(120)).unwrap_err();
        assert!(err.contains("正被另一个 Jolu 进程修改"), "{err}");
        drop(first);
        assert!(!lock_path(&target).exists());
        let _again = FileLock::acquire(&target, Duration::from_millis(100)).unwrap();
    }

    #[test]
    fn threads_serialize_read_modify_write() {
        let target = temp("threads");
        std::fs::write(&target, "0").unwrap();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let target = target.clone();
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        let _lock = FileLock::acquire(&target, Duration::from_secs(10)).unwrap();
                        let n: u32 = std::fs::read_to_string(&target).unwrap().trim().parse().unwrap();
                        std::fs::write(&target, (n + 1).to_string()).unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "80", "没有丢失任何一次读改写");
    }

    #[test]
    fn stale_lock_from_a_crashed_holder_is_taken_over() {
        let target = temp("stale");
        let lock = lock_path(&target);
        std::fs::write(&lock, "999999 old").unwrap();
        let old = SystemTime::now() - STALE_AFTER - Duration::from_secs(5);
        std::fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
        let _held = FileLock::acquire(&target, Duration::from_millis(100)).unwrap();
    }
}
