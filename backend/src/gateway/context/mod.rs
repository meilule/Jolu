//! Agent 上下文观察：从发给上游的请求体里读出上下文由什么组成（V10，执行文档
//! `docs/plans/2026-11-16-agent-context-observation-backend.md`）。
//!
//! 第 1 期只有解析：四种协议的请求体 → 八类组成（system / tools / skills / memory / files / results / chat / unknown）
//! 与逐项明细的 token 估算，以及按上游实测量的校准。还没有接到网关，也不落库。
//!
//! 算法移植自 magpie `internal/gateway/prompt.go`（MIT，commit `40f7de24`），与它的差异写在各子模块开头。
//! 只记名字、tag、次数与 token，不保存正文。

mod anthropic;
mod calls;
mod chat;
mod estimate;
mod gemini;
mod parts;
mod responses;
mod text;

#[cfg(test)]
mod tests;

use std::panic::{catch_unwind, AssertUnwindSafe};

pub use estimate::{tokens_of, IMAGE_TOKENS};
pub use parts::{Kind, Prompt, PromptItem, PromptPart, MAX_ITEMS};

/// 请求体的协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Anthropic Messages（`/v1/messages`）。
    Anthropic,
    /// OpenAI Chat Completions（`/v1/chat/completions`）。
    Chat,
    /// OpenAI Responses（`/v1/responses`）。
    Responses,
    /// Gemini 原生（`/v1beta/models/*:generateContent` / `:streamGenerateContent`）。
    Gemini,
}

/// 读出请求体的上下文组成。不是该协议的请求体（不是 JSON、缺少消息数组）时返回 `None`，调用方据此记「暂不支持」。
///
/// 发布版是 `panic = "abort"`，`catch_unwind` 只在测试与调试构建里兜底；真正的保证是实现本身不 panic：
/// 字符串只在 `find` / `char_indices` 给出的字符边界上切，JSON 嵌套深度由 serde_json 的 128 层上限挡住，
/// 并有「把样本的任意节点换成别的类型」的变异测试（`tests::mutated_bodies_never_panic`）。
pub fn parse(protocol: Protocol, body: &[u8]) -> Option<Prompt> {
    guarded(|| {
        let req: serde_json::Value = serde_json::from_slice(body).ok()?;
        if !req.is_object() {
            return None;
        }
        let mut b = parts::Builder::new();
        let ok = match protocol {
            Protocol::Anthropic => anthropic::parse(&mut b, &req),
            Protocol::Chat => chat::parse(&mut b, &req),
            Protocol::Responses => responses::parse(&mut b, &req),
            Protocol::Gemini => gemini::parse(&mut b, &req),
        };
        ok.then(|| b.finish())
    })
}

/// 跑 `f`，panic 时返回 `None`。
fn guarded<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    catch_unwind(AssertUnwindSafe(f)).ok().flatten()
}
