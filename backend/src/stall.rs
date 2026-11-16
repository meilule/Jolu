//! 下载只在真正停住时才掐断（magpie `internal/stall` 同款，4.9 / P2-1）。
//!
//! 给整个下载设一个总时长上限，会把慢但一直在走的链路半路掐断：magpie #5572 里，
//! 约 140 KB/s 的链路下 83 MB 的 Windows 更新包被 10 分钟的总上限截断。反过来，
//! 连接彻底断了、一个字节都不再来时，总上限又要白等满时长才报错。
//!
//! 这里只看「多久没收到数据」：每收到一块数据就 [`Watch::touch`] 一下把计时往后推，
//! 连续 `after` 没有任何数据才判定停住；整体花多久不限。计时从 [`Watch::new`] 起算，
//! 所以服务器始终不回应也会按同样的时长结束。

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 下载多久没收到数据就算停住（magpie `downloadStall`：60 秒，不设总时长）。
pub(crate) const DOWNLOAD_STALL: Duration = Duration::from_secs(60);

/// 一次下载的空闲计时器。
pub(crate) struct Watch {
    start: Instant,
    /// 最近一次收到数据，距 `start` 的毫秒数
    last_ms: AtomicU64,
}

/// 下载停住：连续 `after` 没有收到任何数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stalled {
    pub after: Duration,
}

impl std::fmt::Display for Stalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "连续 {} 秒没有收到数据，已停止下载", self.after.as_secs())
    }
}

impl Watch {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self { start: Instant::now(), last_ms: AtomicU64::new(0) })
    }

    /// 收到了数据：把停住的判定往后推。
    pub(crate) fn touch(&self) {
        let elapsed = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_ms.fetch_max(elapsed, Ordering::Relaxed);
    }

    fn idle(&self) -> Duration {
        let last = Duration::from_millis(self.last_ms.load(Ordering::Relaxed));
        self.start.elapsed().saturating_sub(last)
    }

    /// 跑 `work`，期间连续 `after` 没有 [`Watch::touch`] 就放弃它并返回 [`Stalled`]。
    pub(crate) async fn run<T>(&self, after: Duration, work: impl Future<Output = T>) -> Result<T, Stalled> {
        let watchdog = async {
            loop {
                let idle = self.idle();
                if idle >= after {
                    return;
                }
                // 睡到「最近一次数据 + after」再看；期间来了新数据就会再睡一轮
                tokio::time::sleep(after - idle).await;
            }
        };
        tokio::select! {
            output = work => Ok(output),
            () = watchdog => Err(Stalled { after }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap()
    }

    /// 慢但一直在来：总时长远超 `after` 也不算停住
    #[test]
    fn slow_but_moving_download_finishes() {
        runtime().block_on(async {
            let watch = Watch::new();
            let after = Duration::from_millis(120);
            let feeder = watch.clone();
            let result = watch
                .run(after, async move {
                    for _ in 0..10 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        feeder.touch();
                    }
                    "done"
                })
                .await;
            assert_eq!(result, Ok("done"), "总共约 500ms，远超 120ms，但每 50ms 都有数据");
        });
    }

    /// 来了一阵数据后断了：从最后一次数据起满 `after` 就停
    #[test]
    fn download_that_stops_sending_is_stalled() {
        runtime().block_on(async {
            let watch = Watch::new();
            let after = Duration::from_millis(100);
            let feeder = watch.clone();
            let started = Instant::now();
            let result = watch
                .run(after, async move {
                    for _ in 0..3 {
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        feeder.touch();
                    }
                    std::future::pending::<()>().await
                })
                .await;
            assert_eq!(result, Err(Stalled { after }));
            let took = started.elapsed();
            assert!(took >= Duration::from_millis(200) && took < Duration::from_secs(2), "约 120ms 数据 + 100ms 空闲：{took:?}");
        });
    }

    /// 服务器始终不回应：从开始算满 `after` 就停
    #[test]
    fn server_that_never_answers_is_stalled() {
        runtime().block_on(async {
            let watch = Watch::new();
            let after = Duration::from_millis(80);
            let result = watch.run(after, std::future::pending::<()>()).await;
            assert_eq!(result, Err(Stalled { after }));
        });
    }

    #[test]
    fn stalled_message_names_the_idle_time() {
        assert_eq!(Stalled { after: DOWNLOAD_STALL }.to_string(), "连续 60 秒没有收到数据，已停止下载");
    }
}
