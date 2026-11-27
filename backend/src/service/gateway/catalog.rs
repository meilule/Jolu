//! 模型目录用例编排（合并 models.dev 元数据与来源覆盖；实现位于 `gateway::catalog`）。

pub use crate::gateway::catalog::{list_with_meta as list, MetaLookup};
