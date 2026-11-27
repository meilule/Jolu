//! 本机与 WSL 的 AI 工具探测、版本检查与自定义 Agent 管理。

pub(crate) mod custom;
pub(crate) mod discovery;
pub mod hooks;
pub(crate) mod version;
pub(crate) mod wsl_probe;
pub(crate) mod zcode_version;
