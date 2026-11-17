//! 上下文的八类组成，以及边读请求体边累计的 `Builder`（移植 magpie `promptBuilder` / `prompt` / `calibrate`）。
//!
//! 只记名字、tag、次数与 token，不存正文。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 每类最多保留多少项，其余汇总成一项（magpie `promptItems`）。
pub const MAX_ITEMS: usize = 24;

/// 上下文组成的类别，顺序固定（前端配色与顺序依赖它）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// agent 自己的提示词：system / developer / harness 消息。
    System,
    /// 工具定义。
    Tools,
    /// agent 列给模型、可按需加载的 Skills。
    Skills,
    /// 指令文件（CLAUDE.md、AGENTS.md、GEMINI.md）与提醒。
    Memory,
    /// 工具读进来的文件与图片。
    Files,
    /// 工具的其他输出。
    Results,
    /// 用户与模型说的话、模型的调用。
    Chat,
    /// 认不出的内容块 / 条目 / 角色（Jolu 相对 magpie 新增，不推断归类）。
    Unknown,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::System,
        Kind::Tools,
        Kind::Skills,
        Kind::Memory,
        Kind::Files,
        Kind::Results,
        Kind::Chat,
        Kind::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::System => "system",
            Kind::Tools => "tools",
            Kind::Skills => "skills",
            Kind::Memory => "memory",
            Kind::Files => "files",
            Kind::Results => "results",
            Kind::Chat => "chat",
            Kind::Unknown => "unknown",
        }
    }
}

/// 一类里的一项：一个工具、一个 MCP 服务器的全部工具、一个指令文件、一个读过的文件、一轮对话……
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptItem {
    pub name: String,
    /// 它是什么：mcp、namespace、user、project、local、memory、skills、reminder、image、read、shell、result、turn、more、turns、type、role……
    pub tag: String,
    /// 汇总了几个：MCP 服务器 / 命名空间下的工具数、同一工具的结果次数、汇总项的项数；1 与无意义时为 0（不输出）。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub n: i64,
    pub tokens: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// 一类组成。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptPart {
    pub kind: Kind,
    pub tokens: i64,
    /// 非对话类按 token 从大到小；对话类按轮次从早到晚。最多 `MAX_ITEMS` 项。
    #[serde(default)]
    pub items: Vec<PromptItem>,
}

/// 一次请求的上下文组成。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    /// 总量：校准前等于 `estimated`，校准后等于上游实测的输入量。
    pub tokens: i64,
    /// 本地估算的总量，校准后也保留，用来提示估算与实测的偏差（不凑平）。
    pub estimated: i64,
    /// 已按上游实测校准。
    #[serde(default)]
    pub counted: bool,
    /// 请求引用了上一次响应（Responses 的 `previous_response_id`），更早的轮次在上游那边，校准后计入对话。
    #[serde(default)]
    pub held: bool,
    /// 用户说话的轮数。
    pub turns: i64,
    /// 只含有内容的类别，按 `Kind::ALL` 的顺序。
    pub parts: Vec<PromptPart>,
    /// 逐条工具结果（V10-05 原位展开用），按出现顺序，最多 `MAX_CALLS` 条（保留最近的）。不进 `parts_json`，单独存。
    #[serde(skip)]
    pub calls: Vec<ToolResult>,
}

/// 每次请求最多记多少条工具结果明细。
pub const MAX_CALLS: usize = 200;

/// 一条工具结果：只有名字、脱敏后的命令或路径、计数，不含输出正文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    /// 调用的工具名。
    pub tool: String,
    /// 命令或读的路径（本机目录、用户名、凭证已替换）；看不出时为 `None`。
    pub target: Option<String>,
    /// 记到了哪一类：读文件的是 files，其余是 results。
    pub kind: Kind,
    pub tokens: i64,
    /// 输出的行数。
    pub lines: i64,
    /// 输出里写明的退出码；没写为 `None`（不是 0）。
    pub exit_code: Option<i64>,
    /// `target` 是否替换过内容。
    pub masked: bool,
    /// 一句话摘要，例如「退出码 0，输出 12 行」。
    pub summary: String,
    /// 正文是否采集。第一版一律不采集。
    pub captured: bool,
}

impl Prompt {
    /// 某一类，没有时为 `None`。
    pub fn part(&self, kind: Kind) -> Option<&PromptPart> {
        self.parts.iter().find(|p| p.kind == kind)
    }

    /// 按上游实测的输入量等比缩放各类与各项，四舍五入的差额补给最大的一类，保证各类之和等于实测量。
    /// 已校准过、实测量或估算为 0 时原样返回（magpie `calibrated`）。
    pub fn calibrated(&self, counted: i64) -> Prompt {
        let mut p = self.clone();
        if counted <= 0 || p.counted || p.tokens <= 0 {
            return p;
        }
        let scale = counted as f64 / p.tokens as f64;
        let round = |x: i64| (x as f64 * scale + 0.5).floor() as i64;
        for call in &mut p.calls {
            call.tokens = round(call.tokens);
        }
        let mut sum = 0;
        for part in &mut p.parts {
            part.tokens = round(part.tokens);
            for it in &mut part.items {
                it.tokens = round(it.tokens);
            }
            sum += part.tokens;
        }
        if sum != counted {
            if let Some(big) = (0..p.parts.len()).max_by_key(|&i| (p.parts[i].tokens, std::cmp::Reverse(i))) {
                p.parts[big].tokens += counted - sum;
            }
        }
        p.tokens = counted;
        p.counted = true;
        p
    }
}

#[derive(Default)]
struct Acc {
    n: i64,
    tokens: i64,
}

/// 一次工具调用：调了什么、读了哪些文件（读文件的调用才有）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub files: Vec<String>,
    /// 命令或路径（原文，记明细时再脱敏）。
    pub target: Option<String>,
}

/// 边读请求体边累计。
#[derive(Default)]
pub struct Builder {
    parts: HashMap<Kind, HashMap<(String, String), Acc>>,
    pub(super) turn: i64,
    pub(super) held: bool,
    /// 调用 id → 调用，给它的结果找名字与文件。
    pub(super) calls: HashMap<String, ToolCall>,
    /// 逐条工具结果。
    pub(super) results: Vec<ToolResult>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记一项；同类同 tag 同名的合并，次数加一。token 不大于 0 的不记。
    pub fn add(&mut self, kind: Kind, name: &str, tag: &str, tokens: i64) {
        if tokens <= 0 {
            return;
        }
        let acc = self.parts.entry(kind).or_default().entry((tag.to_string(), name.to_string())).or_default();
        acc.n += 1;
        acc.tokens += tokens;
    }

    /// 给已记的一项再加次数（命名空间里的工具数）。
    pub(super) fn add_n(&mut self, kind: Kind, name: &str, tag: &str, n: i64) {
        if let Some(acc) = self.parts.get_mut(&kind).and_then(|m| m.get_mut(&(tag.to_string(), name.to_string()))) {
            acc.n += n;
        }
    }

    /// 记到对话的当前一轮。
    pub fn chat(&mut self, tokens: i64) {
        if self.turn == 0 {
            self.turn = 1;
        }
        let name = self.turn.to_string();
        self.add(Kind::Chat, &name, "turn", tokens);
    }

    pub fn new_turn(&mut self) {
        self.turn += 1;
    }

    /// 收尾：排序、截到 `MAX_ITEMS` 项、汇总其余。
    pub fn finish(self) -> Prompt {
        let mut results = self.results;
        if results.len() > MAX_CALLS {
            results.drain(..results.len() - MAX_CALLS);
        }
        let mut p = Prompt { tokens: 0, estimated: 0, counted: false, held: self.held, turns: self.turn, parts: vec![], calls: results };
        let mut parts = self.parts;
        for kind in Kind::ALL {
            let Some(m) = parts.remove(&kind) else { continue };
            if m.is_empty() {
                continue;
            }
            let mut part = PromptPart { kind, tokens: 0, items: Vec::with_capacity(m.len()) };
            for ((tag, name), acc) in m {
                part.tokens += acc.tokens;
                // 次数只在说明问题的地方保留：MCP 服务器 / 命名空间下的工具数、同一工具的结果次数
                let n = if matches!(tag.as_str(), "mcp" | "namespace" | "result") && acc.n != 1 { acc.n } else { 0 };
                part.items.push(PromptItem { name, tag, n, tokens: acc.tokens });
            }
            if kind == Kind::Chat {
                part.items.sort_by_key(|it| it.name.parse::<i64>().unwrap_or(0));
                // 保留最近的轮次，更早的汇总成一项放最前
                if part.items.len() > MAX_ITEMS {
                    let cut = part.items.len() - MAX_ITEMS + 1;
                    let rest: Vec<PromptItem> = part.items.drain(..cut).collect();
                    let earlier = PromptItem {
                        name: "earlier".into(),
                        tag: "turns".into(),
                        n: rest.len() as i64,
                        tokens: rest.iter().map(|it| it.tokens).sum(),
                    };
                    part.items.insert(0, earlier);
                }
            } else {
                part.items.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.name.cmp(&b.name)));
                if part.items.len() > MAX_ITEMS {
                    let rest: Vec<PromptItem> = part.items.drain(MAX_ITEMS - 1..).collect();
                    part.items.push(PromptItem {
                        name: "more".into(),
                        tag: "more".into(),
                        n: rest.len() as i64,
                        tokens: rest.iter().map(|it| it.tokens).sum(),
                    });
                }
            }
            p.tokens += part.tokens;
            p.parts.push(part);
        }
        p.estimated = p.tokens;
        p
    }
}
