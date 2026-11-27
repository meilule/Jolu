//! 零业务依赖的基础设施模块（脱敏基元、路径推断、网络、进程、文件锁、自更新与隔离校验）。

pub(crate) mod filelock;
pub(crate) mod net;
pub mod paths;
pub(crate) mod proc;
pub(crate) mod redact;
pub(crate) mod stall;
pub(crate) mod steady;
pub(crate) mod updater;
pub(crate) mod validation;
