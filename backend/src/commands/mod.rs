//! Tauri 命令（4.10：从 lib.rs 按功能区搬出，函数体原样不动）。
//!
//! 每个子模块 `use crate::*;` 取用根模块的共享状态与辅助函数；根模块再 `use commands::*;`
//! 把命令导回，`generate_handler!` 列表与其他调用点因此不用改。条目一律 `pub(crate)`。
//! 业务逻辑要给 CLI 复用时再抽到 `service/`（见 `service/mod.rs`），这里只做归类。

pub(crate) mod panel;
pub(crate) mod island_window;
pub(crate) mod usage_sessions;
pub(crate) mod quotas;
pub(crate) mod preferences;
pub(crate) mod capabilities;
pub(crate) mod agents;
pub(crate) mod routing;
pub(crate) mod providers;

pub(crate) use panel::*;
pub(crate) use island_window::*;
pub(crate) use usage_sessions::*;
pub(crate) use quotas::*;
pub(crate) use preferences::*;
pub(crate) use capabilities::*;
pub(crate) use agents::*;
pub(crate) use routing::*;
pub(crate) use providers::*;
