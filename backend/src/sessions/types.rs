//! 会话线程与外部会话采集共用的纯数据类型（零业务模块依赖，打破 `session_threads ↔ {kimi,pi,dsh}_sessions / usage_stats` 循环）。

use serde::{Deserialize, Serialize};

/// 账号筛选里的「未归属账号」：没有经本地代理归账到任何连接的本机记录。
pub const UNATTRIBUTED: &str = "__unattributed__";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionFilter {
    #[serde(default)]
    pub platform: Option<String>,
    /// 连接 id 或 [`UNATTRIBUTED`]
    #[serde(default)]
    pub connection: Option<String>,
    /// 项目目录（cwd 原值）；空字符串表示「未识别项目」
    #[serde(default)]
    pub project: Option<String>,
    /// 只看最后活跃时间不早于此刻（UTC 毫秒）
    #[serde(default)]
    pub since: Option<i64>,
    #[serde(default)]
    pub search: Option<String>,
    /// 只查这些原始会话 ID（统一网关页定向取数用；前端筛选不会传）
    #[serde(skip)]
    pub session_ids: Option<Vec<String>>,
    /// 跳过 Kimi / Pi / dsh 这类「直接读本地会话文件」的来源。
    #[serde(skip)]
    pub skip_external: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionItem {
    pub platform: String,
    pub session_id: String,
    pub title: Option<String>,
    /// 项目目录；源文件不可读或没写 cwd 时为 None（归入「未识别项目」）
    pub cwd: Option<String>,
    pub first_ts: i64,
    pub last_ts: i64,
    pub requests: i64,
    pub fresh_input: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub output: Option<i64>,
    /// 只有全部请求归到同一连接时才有值；含未归属请求时为 None
    pub connection_id: Option<String>,
    /// running / done / failed：来自来源生命周期事件；recent：只有写入时间；usage_only：没有可读源文件
    pub status: String,
    pub has_source: bool,
}

#[derive(Debug, Serialize)]
pub struct SessionList {
    pub sessions: Vec<SessionItem>,
    /// 筛选内去重来源文件数
    pub files: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ToolCall {
    pub name: String,
    /// 调用摘要：文件路径、命令等；取不到时为空
    pub summary: String,
    /// 工具输出片段；来源没记录输出时为 None，不补写结果
    pub output: Option<String>,
    /// 工具结果是否失败
    pub failed: Option<bool>,
    /// 退出码
    pub exit_code: Option<i32>,
    /// 调用涉及的文件路径，只供交接事实区使用
    #[serde(skip)]
    pub paths: Vec<String>,
}

/// 库型来源（ZCode）记录的单条 token 用量：缺哪个字段就是 None，不补 0。
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct TokenUsage {
    pub total: Option<i64>,
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub reasoning: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Message {
    /// "user" | "assistant"
    pub role: String,
    pub ts: Option<i64>,
    pub text: String,
    pub tools: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
pub struct MessagePage {
    pub available: bool,
    pub total: usize,
    pub offset: usize,
    pub messages: Vec<Message>,
    pub models: Vec<String>,
    pub cost: f64,
    pub priced_requests: i64,
    pub requests: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}
