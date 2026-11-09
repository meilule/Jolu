//! 服务函数层（批次 G §3.11）：把业务逻辑从 `#[tauri::command]` 里抽出来。
//!
//! ## 为什么要有这一层
//!
//! Tauri 命令只能从 WebView 的 IPC 调用，CLI 调不到。想把同一套能力给 `jolu` 命令行用，
//! 业务逻辑就不能写在 `#[tauri::command]` 函数体里。这里的函数：
//!
//! - 只接显式参数（`&Connection` / `&Settings` / 普通值），**不接 `AppHandle`**；
//! - 需要发事件或读全局状态时，通过调用方传入的回调 / 已取好的值完成；
//! - 返回领域类型或 `String` 错误，不碰 `tauri::State`。
//!
//! 命令侧因此变成薄壳：解包 `State` → 调这里 → 发事件。
//!
//! ## 抽取顺序与边界
//!
//! 按 `docs/plans/2026-11-09-magpie-parity-batch3.md` §3.11：先抽五个高频命令，
//! 每抽一个跑一次全量后端测试。**不追求一次性抽完**——`lib.rs` 里还有大量命令
//! 依赖 `AppHandle` 做副作用（重启监听、发通知），抽它们要先设计回调接口，风险更高。
//!
//! ## 不做什么
//!
//! - 不在这一层做参数校验以外的副作用：写配置文件、拉进程、重启监听都留在命令侧或原有模块；
//! - 不吞错：所有失败原样以 `String` 返回，由命令侧决定怎么报给前端。

pub mod paths;
pub mod settings;

pub mod gateway {
    //! 统一网关相关的服务函数。与 `crate::gateway`（协议与路由实现）区分：
    //! 那边是「怎么转发」，这边是「一个完整操作要做什么」。

    pub mod catalog;
    pub mod client_keys;
    pub mod groups;
    pub mod middleware;
    pub mod quota;
    pub mod status;
}
