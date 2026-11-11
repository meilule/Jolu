//! 命名客户端 Key 的同时请求数上限（`max_concurrency`）。
//!
//! 为什么：局域网共享时一把 Key 可能被多台设备同时用，没有上限就能一下打满所有上游账号。
//! 每日请求数 / Token 上限管的是总量，管不住瞬时并发。
//!
//! 管什么：
//! - 设了上限的 Key，超出的请求**按到达顺序排队**（tokio 的信号量是公平的），不拒绝；
//! - 客户端在排队时断开，它的等待随请求一起被丢弃，不占位也不发请求；
//! - 流式响应占着位置直到流结束（或客户端断开）——占位要覆盖整个上游调用。
//!
//! 不管什么：没有命名 Key 的本机请求不限；只读的查询（模型清单等）不经过这里。
//!
//! 顺序：在每日 Token 预扣**之前**取位置——排队中断开时不会留下预扣。Jolu 没有
//! magpie 那种「账号近上限时的轮次锁」，不存在两把锁互等的问题（magpie `0024660`）。
//!
//! 参照 magpie `internal/gateway/key_slots.go`（`e215d33`）；差异：magpie 还把等待计入
//! 每次尝试的排队时长并在 `/v1/magpie/concurrency` 列出，Jolu 暂不展示排队人数。

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// 上限的取值范围（与 magpie 一致）。
pub const MAX_CAP: u32 = 1000;

/// 每把 Key 一个信号量；上限改了就换一个新的（旧请求持有的旧许可照常释放，不影响新上限）。
static SLOTS: LazyLock<Mutex<HashMap<String, (u32, Arc<Semaphore>)>>> = LazyLock::new(Default::default);

fn semaphore_for(name: &str, cap: u32) -> Arc<Semaphore> {
    let mut slots = SLOTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match slots.get(name) {
        Some((current, semaphore)) if *current == cap => semaphore.clone(),
        _ => {
            let semaphore = Arc::new(Semaphore::new(cap as usize));
            slots.insert(name.to_string(), (cap, semaphore.clone()));
            semaphore
        }
    }
}

/// 取一个位置；满了就排队等。`cap` 为 None（不限）时立即返回 None。
pub async fn acquire(name: &str, cap: Option<u32>) -> Option<OwnedSemaphorePermit> {
    let cap = cap.filter(|c| (1..=MAX_CAP).contains(c))?;
    semaphore_for(name, cap).acquire_owned().await.ok()
}

/// 让响应体持有位置：流读完或被丢弃（客户端断开）时才释放。
pub fn hold(response: axum::response::Response, permit: Option<OwnedSemaphorePermit>) -> axum::response::Response {
    let Some(permit) = permit else { return response };
    let (parts, body) = response.into_parts();
    let stream = Held { inner: body.into_data_stream(), _permit: permit };
    axum::response::Response::from_parts(parts, axum::body::Body::from_stream(stream))
}

struct Held<S> {
    inner: S,
    _permit: OwnedSemaphorePermit,
}

impl<S: futures_util::Stream + Unpin> futures_util::Stream for Held<S> {
    type Item = S::Item;
    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<S::Item>> {
        std::pin::Pin::new(&mut self.inner).poll_next(cx)
    }
}

/// 校验界面 / CLI 给的上限：None = 不限，1–1000 有效，其余拒绝。
pub fn validate(cap: Option<i64>) -> Result<Option<u32>, String> {
    match cap {
        None => Ok(None),
        Some(n) if (1..=MAX_CAP as i64).contains(&n) => Ok(Some(n as u32)),
        Some(n) => Err(format!("同时请求数只能留空（不限）或填 1–{MAX_CAP}，当前为 {n}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn requests_over_the_cap_wait_in_order_and_go_when_a_slot_frees() {
        let first = acquire("slots-order", Some(1)).await.expect("第一个直接拿到");
        let waiting = tokio::spawn(async { acquire("slots-order", Some(1)).await.is_some() });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!waiting.is_finished(), "满了要排队，而不是拒绝或放行");
        drop(first);
        assert!(tokio::time::timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap(), "前一个结束后轮到它");
    }

    #[tokio::test]
    async fn no_cap_means_no_wait_and_a_dropped_waiter_leaves_the_queue() {
        assert!(acquire("slots-free", None).await.is_none(), "不限就不占位");
        let held = acquire("slots-cancel", Some(1)).await;
        // 排队中的请求被丢弃（客户端断开）：不应在之后拿走位置
        let abandoned = tokio::spawn(async { acquire("slots-cancel", Some(1)).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        abandoned.abort();
        drop(held);
        assert!(tokio::time::timeout(Duration::from_secs(2), acquire("slots-cancel", Some(1))).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_held_response_keeps_its_slot_until_the_body_is_read() {
        let permit = acquire("slots-body", Some(1)).await;
        let response = hold(axum::response::Response::new(axum::body::Body::from("流式正文")), permit);
        let next = tokio::spawn(async { acquire("slots-body", Some(1)).await.is_some() });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!next.is_finished(), "正文没读完前位置还占着");
        let text = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&text[..], "流式正文".as_bytes());
        assert!(tokio::time::timeout(Duration::from_secs(2), next).await.unwrap().unwrap());
    }

    #[test]
    fn caps_outside_one_to_a_thousand_are_refused() {
        assert_eq!(validate(None).unwrap(), None);
        assert_eq!(validate(Some(5)).unwrap(), Some(5));
        assert!(validate(Some(0)).is_err());
        assert!(validate(Some(1001)).is_err());
    }
}
