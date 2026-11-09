//! 稳定读取（对齐 magpie `internal/steady`）：跨过 Windows 上 rename 覆盖文件时的短暂窗口。
//!
//! Windows 上别的进程用「写临时文件 + rename 覆盖」更新配置时，目标文件会有约 2ms
//! 「不存在」或「被占用」（ERROR_SHARING_VIOLATION）。恰好在这个窗口里读，会把
//! 「有配置」读成「没有配置」——接线代码随后按空配置写回，用户原有的配置就被覆盖了。
//! 这里的读取遇到这两类错误时短暂重试；真正不存在的文件最多多等约 30ms 后照常报 NotFound。

use std::io;
use std::path::Path;

/// 重试次数与间隔：rename 窗口约 2ms，10 × 3ms 足够跨过，又不会让「真的不存在」多等太久。
const ATTEMPTS: u32 = 10;
const PAUSE: std::time::Duration = std::time::Duration::from_millis(3);
/// Windows `ERROR_SHARING_VIOLATION`：文件正被另一个进程以独占方式打开（rename 进行中）。
const SHARING_VIOLATION: i32 = 32;

fn transient(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || error.kind() == io::ErrorKind::PermissionDenied
        || error.raw_os_error() == Some(SHARING_VIOLATION)
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    let mut attempt = 1;
    loop {
        match std::fs::read(path) {
            Err(error) if attempt < ATTEMPTS && transient(&error) => {
                attempt += 1;
                std::thread::sleep(PAUSE);
            }
            result => return result,
        }
    }
}

/// 文件是否存在（同样跨过 rename 窗口）。用于「不存在就按空配置处理」的分支前，
/// 避免把正在被覆盖的文件当成不存在。
pub fn exists(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    (0..ATTEMPTS).any(|attempt| {
        if attempt > 0 {
            std::thread::sleep(PAUSE);
        }
        path.exists()
    })
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// 文件不存在（重试后仍不存在）返回 `Ok(None)`；其他错误照常返回。
pub fn read_optional(path: impl AsRef<Path>) -> io::Result<Option<String>> {
    match read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jolu-steady-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_replaced_by_rename_is_read_instead_of_reported_missing() {
        // 模拟 rename 覆盖窗口：读的时候文件暂时不存在，几毫秒后出现
        let dir = temp("window");
        let path = dir.join("settings.json");
        let writer = {
            let path = path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(8));
                std::fs::write(&path, br#"{"keep":true}"#).unwrap();
            })
        };
        assert_eq!(read_to_string(&path).unwrap(), r#"{"keep":true}"#);
        writer.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_really_is_missing_is_still_reported_missing() {
        let dir = temp("missing");
        let path = dir.join("absent.json");
        let started = std::time::Instant::now();
        assert_eq!(read(&path).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert!(started.elapsed() < std::time::Duration::from_millis(500), "重试只多等几十毫秒");
        assert_eq!(read_optional(&path).unwrap(), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
