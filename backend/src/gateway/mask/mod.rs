//! 网关隐私脱敏（P5）：请求离开本机前把密钥、私钥、令牌、邮箱等换成占位符，
//! 回复（非流式 JSON 与 SSE 流，含工具调用参数）回来后再换回真值。
//!
//! 语义参照 magpie `internal/redact`（Privacy masking）：
//! - **占位符稳定**：同一原值永远换成同一个占位符（原值哈希前缀），prompt cache、
//!   思考签名与多轮对话不因换写而失效；一个占位符绝不对应两个值。
//! - **全链路回填**：非流式响应体与 SSE 流里的占位符都换回，agent 执行命令拿到的
//!   是真值；占位符可能被 SSE 分块切割，用尾部保留缓冲处理（见 [`StreamUnmarker`]）。
//! - **只在内存**：占位符 → 原值表只放在进程内存，不落盘、不进日志；表里只留原值
//!   本身（解引用即真值），进程重启后由下一次请求重新学得。
//! - **失败即拒发**：脱敏开启但占位符表不可写时请求直接 503，绝不原样放行。
//!
//! 3.7 起每类值可选处理方式（遮住 / 只记录 / 拦下，[`Action`]）与「从不脱敏」豁免清单
//! （整段匹配、不分大小写）；每次命中留一条只在内存里的记录（[`records`]，给设置页的
//! 对照视图用：只存种类、长度、开头几个字与前后文，不存原值），第一次真的遮住时调
//! [`on_first_mask`] 注册的钩子（系统通知）。交接摘要的脱敏（`handoff_facts::redact`）
//! 也调本模块的 [`sensitive_spans`]，两边用同一套识别规则与同一份豁免清单。
//!
//! 规则是对 [`crate::handoff_facts::redact`] 遮蔽形状的子集复用：前缀 Key 共用同一份
//! [`crate::handoff_facts::PREFIX_KEYS`]；私钥块、`Bearer`/`Authorization`、
//! `token=`/`api_key=` 一类凭证键值、邮箱与校验位合法的卡号 / 身份证号在这里按
//! 「可回填」重写。handoff_facts 只需要把命中换成 `***`，网关需要能换回原值，
//! 所以不直接调它的 redact，而是共用常量、各自产出（避免两套前缀清单各写各的）。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock, RwLock};

use sha2::{Digest, Sha256};

use crate::infra::redact::{
    collect_card_ranges, collect_idcard_ranges, set_allow_list, PREFIX_KEYS, PREFIX_MIN,
};


mod response;
#[cfg(test)]
mod tests;

pub(crate) use response::{prepare_request, restore_middleware, restore_response};

/* ───────────────────────────── 开关（进程级） ───────────────────────────── */

pub(crate) static ENABLED: AtomicBool = AtomicBool::new(false);
pub(crate) static MASKER: LazyLock<Masker> = LazyLock::new(Masker::new);

/// 设置页开关（进程级；改完立即生效，不需要重启代理）。
/// 观测模式不经过网关 router，天然不受影响；`privacy_masking_set` 命令与启动时调用。
pub(crate) fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// 脱敏开启时返回引擎；关闭时返回 None（调用方对请求体与响应体都不做任何处理）。
pub(crate) fn request_masker() -> Option<&'static Masker> {
    if ENABLED.load(Ordering::Relaxed) { Some(&MASKER) } else { None }
}

/* ─────────────────────── 处理方式与豁免（3.7） ─────────────────────── */

/// 一类值的处理方式（magpie `RedactActions` 同款三档）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// 换成占位符（默认）
    Mask,
    /// 原样发出，只留记录
    Warn,
    /// 整个请求不发，给调用方一条说明原因的错误
    Block,
}

impl Action {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "mask" => Some(Self::Mask),
            "warn" => Some(Self::Warn),
            "block" => Some(Self::Block),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Mask => "mask",
            Self::Warn => "warn",
            Self::Block => "block",
        }
    }
}

/// 处理方式表 + 豁免清单。
#[derive(Clone, Debug, Default)]
pub(crate) struct Policy {
    pub(crate) actions: HashMap<&'static str, Action>,
    pub(crate) allow: HashSet<String>,
}

impl Policy {
    /// 设置里的原样配置 → 策略。未知种类 / 处理方式报错（保存前校验）。
    pub(crate) fn new(actions: &BTreeMap<String, String>, allow: &[String]) -> Result<Self, String> {
        let mut map = HashMap::new();
        for (kind, action) in actions {
            let kind = TYPES.iter().find(|t| **t == kind.as_str()).ok_or_else(|| format!("未知的脱敏种类「{kind}」"))?;
            let action = Action::parse(action).ok_or_else(|| format!("未知的处理方式「{action}」，只能是 mask / warn / block"))?;
            if action != Action::Mask {
                map.insert(*kind, action);
            }
        }
        let allow = allow.iter().map(|v| v.trim().to_lowercase()).filter(|v| !v.is_empty()).collect();
        Ok(Self { actions: map, allow })
    }

    pub(crate) fn action(&self, kind: &str) -> Action {
        self.actions.get(kind).copied().unwrap_or(Action::Mask)
    }

    pub(crate) fn allows(&self, value: &str) -> bool {
        !self.allow.is_empty() && self.allow.contains(&value.trim().to_lowercase())
    }
}

pub(crate) static POLICY: LazyLock<RwLock<Policy>> = LazyLock::new(|| RwLock::new(Policy::default()));

/// 设置页保存 / 启动时调用。
pub(crate) fn set_policy(policy: Policy) {
    set_allow_list(policy.allow.clone());
    if let Ok(mut current) = POLICY.write() {
        *current = policy;
    }
}

pub(crate) fn policy() -> Policy {
    POLICY.read().map(|p| p.clone()).unwrap_or_default()
}

/// 种类的中文名（错误提示与界面共用）。
pub(crate) fn kind_label(kind: &str) -> &'static str {
    match kind {
        "APIKEY" => "API Key",
        "PRIVATEKEY" => "私钥",
        "TOKEN" => "令牌",
        "EMAIL" => "邮箱",
        "CARD" => "银行卡号",
        "IDCARD" => "身份证号",
        _ => "敏感值",
    }
}

/* ───────────────────────── 命中记录（3.7） ───────────────────────── */

/// 一处命中。不存原值：只有种类、长度、开头几个字与前后文（前文取自已脱敏的结果，
/// 后文截在下一处命中之前，所以记录里不会带出别的原值）。
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct Hit {
    pub kind: &'static str,
    pub action: &'static str,
    pub length: usize,
    pub preview: String,
    pub placeholder: Option<String>,
    pub left: String,
    pub right: String,
}

/// 一次请求的脱敏结果。
#[derive(Debug, Default)]
pub(crate) struct Report {
    pub hits: Vec<Hit>,
    /// 真的换成占位符的处数
    pub masked: usize,
    /// 命中「拦下」的种类
    pub blocked: Vec<&'static str>,
}

/// 单次请求最多记这么多处命中（记录只为看个究竟，不必穷举）。
pub(crate) const REPORT_HITS_MAX: usize = 200;
/// 内存里最多留这么多条请求记录。
pub(crate) const RECORDS_MAX: usize = 100;
pub(crate) const CONTEXT_CHARS: usize = 24;

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct MaskRecord {
    pub id: u64,
    pub at_ms: i64,
    /// 协议入口（messages / chat / responses）
    pub entry: String,
    pub blocked: bool,
    pub masked: usize,
    pub hits: Vec<Hit>,
}

pub(crate) static RECORDS: LazyLock<Mutex<VecDeque<MaskRecord>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));
pub(crate) static RECORD_SEQ: AtomicU64 = AtomicU64::new(1);
pub(crate) static FIRST_HOOK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
pub(crate) static FIRST_FIRED: AtomicBool = AtomicBool::new(false);

/// 最近的脱敏记录（新的在前）。
pub(crate) fn records() -> Vec<MaskRecord> {
    RECORDS.lock().map(|r| r.iter().rev().cloned().collect()).unwrap_or_default()
}

pub(crate) fn clear_records() {
    if let Ok(mut r) = RECORDS.lock() {
        r.clear();
    }
}

/// 本进程第一次真的遮住值时调一次（启动时注册；钩子自己决定要不要弹通知）。
pub(crate) fn on_first_mask(hook: impl Fn() + Send + Sync + 'static) {
    let _ = FIRST_HOOK.set(Box::new(hook));
}

pub(crate) fn remember(entry: &str, report: &Report) {
    if report.hits.is_empty() {
        return;
    }
    let record = MaskRecord {
        id: RECORD_SEQ.fetch_add(1, Ordering::Relaxed),
        at_ms: chrono::Utc::now().timestamp_millis(),
        entry: entry.to_string(),
        blocked: !report.blocked.is_empty(),
        masked: report.masked,
        hits: report.hits.clone(),
    };
    if let Ok(mut r) = RECORDS.lock() {
        r.push_back(record);
        while r.len() > RECORDS_MAX {
            r.pop_front();
        }
    }
    if report.masked > 0 && !FIRST_FIRED.swap(true, Ordering::Relaxed) {
        if let Some(hook) = FIRST_HOOK.get() {
            hook();
        }
    }
}

pub(crate) fn head_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

pub(crate) fn tail_chars(text: &str, n: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(n)).collect()
}

/// 原值的开头几个字（邮箱只露两个）。
pub(crate) fn preview(kind: &str, value: &str) -> String {
    let n = if kind == "EMAIL" { 2 } else { 4 };
    let shown: String = value.chars().take(n).collect();
    format!("{shown}…")
}

/// 交接摘要等别处的脱敏用：本模块规则识别出的、属于 `kinds` 的敏感片段（字节区间），
/// 豁免清单里的值除外。不看处理方式——出现就遮。
pub(crate) fn sensitive_spans(text: &str, kinds: &[&str]) -> Vec<(usize, usize)> {
    let policy = policy();
    collect_spans(text.as_bytes())
        .into_iter()
        .filter(|span| kinds.contains(&span.kind))
        .filter_map(|span| {
            let start = floor_boundary(text, span.start);
            let end = floor_boundary(text, span.end);
            (end > start && !policy.allows(&text[start..end])).then_some((start, end))
        })
        .collect()
}

/* ───────────────────────────── 占位符 ───────────────────────────── */

/// 占位符类型。类型名只进占位符文本，不含任何原值信息。
pub(crate) const TYPES: [&str; 6] = ["APIKEY", "PRIVATEKEY", "TOKEN", "EMAIL", "CARD", "IDCARD"];

pub(crate) const PLACEHOLDER_PREFIX: &str = "{{JOLU_";
pub(crate) const PLACEHOLDER_SUFFIX: &str = "}}";
pub(crate) const HASH_CHARS: usize = 8;

pub(crate) const fn longest_type_len() -> usize {
    let mut max = 0;
    let mut i = 0;
    while i < TYPES.len() {
        if TYPES[i].len() > max {
            max = TYPES[i].len();
        }
        i += 1;
    }
    max
}

/// 占位符总长上限：`{{JOLU_<类型>_<8位hash>}}`。SSE 尾部保留缓冲以此为界。
pub(crate) const PLACEHOLDER_MAX_LEN: usize =
    PLACEHOLDER_PREFIX.len() + longest_type_len() + 1 + HASH_CHARS + PLACEHOLDER_SUFFIX.len();

/// 占位符 = 原值的 SHA-256 前 4 字节（8 位小写 hex）。纯哈希、无密钥：
/// 同一原值在进程重启后也得到同一占位符；32 位前缀对高熵密钥不可反推。
pub(crate) fn placeholder(kind: &str, value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut out = String::with_capacity(PLACEHOLDER_MAX_LEN);
    out.push_str(PLACEHOLDER_PREFIX);
    out.push_str(kind);
    out.push('_');
    for byte in digest.iter().take(4) {
        let _ = write!(out, "{byte:02x}");
    }
    out.push_str(PLACEHOLDER_SUFFIX);
    out
}

/* ───────────────────────────── 命中规则 ───────────────────────────── */

/// 单个命中：字节区间与占位符类型。所有规则只在 ASCII 字符上判定命中，
/// 区间端点必然落在字符边界（见 floor_boundary 的兜底），可安全切片。
pub(crate) struct Span {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: &'static str,
}

/// 单个值最多向前取这么多字节（与 handoff_facts::VALUE_MAX 同量级）
pub(crate) const VALUE_MAX: usize = 4_096;
/// 私钥块最多遮这么多字节（与 handoff_facts::PEM_MAX 一致）
pub(crate) const PEM_MAX: usize = 8_192;
/// Bearer / Basic 之后的令牌最短长度（与 handoff_facts::token_spans 一致）
pub(crate) const BEARER_MIN: usize = 16;
/// 凭证键值的最短长度（与 handoff_facts::SECRET_MIN 一致）
pub(crate) const SECRET_MIN: usize = 8;

pub(crate) fn is_value_char(c: u8) -> bool {
    !c.is_ascii_whitespace()
        && !matches!(c, b'"' | b'\'' | b'`' | b',' | b';' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'<' | b'>')
}

pub(crate) fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

pub(crate) fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// 把字节下标退回到字符边界：规则的长度上限截断可能落在多字节字符中间
pub(crate) fn floor_boundary(text: &str, mut at: usize) -> usize {
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// 无键名的前缀 Key：`sk-…`、`ghp_…`、`AKIA…` 等，前缀之后至少 PREFIX_MIN 位。
pub(crate) fn prefix_key_spans(text: &[u8], spans: &mut Vec<Span>) {
    let mut i = 0;
    while i < text.len() {
        // 词首判定与 handoff_facts::token_spans 一致：前一个字符不是字母数字或下划线
        let boundary = i == 0 || !(text[i - 1].is_ascii_alphanumeric() || text[i - 1] == b'_');
        if !boundary {
            i += 1;
            continue;
        }
        let Some(prefix) = PREFIX_KEYS.iter().find(|p| text[i..].starts_with(p.as_bytes())) else {
            i += 1;
            continue;
        };
        let vs = i + prefix.len();
        let mut ve = vs;
        while ve < text.len()
            && ve - vs < VALUE_MAX
            && (text[ve].is_ascii_alphanumeric() || matches!(text[ve], b'-' | b'_' | b'.'))
        {
            ve += 1;
        }
        if ve - vs >= PREFIX_MIN {
            spans.push(Span { start: vs, end: ve, kind: "APIKEY" });
            i = ve;
            continue;
        }
        i += prefix.len().max(1);
    }
}

/// 私钥块：`-----BEGIN … PRIVATE KEY-----` 之后的正文，到 `-----END` 为止。
/// 只遮正文、保留头尾两行——占位符夹在 BEGIN/END 之间，模型仍能看出这是一把私钥。
pub(crate) fn pem_spans(text: &[u8], spans: &mut Vec<Span>) {
    let mut i = 0;
    while let Some(at) = find_from(text, i, b"-----BEGIN ") {
        let line_end = text[at..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| at + p)
            .unwrap_or(text.len());
        if find_from(&text[at..line_end], 0, b"PRIVATE KEY").is_none() {
            i = at + 1;
            continue;
        }
        let body = (line_end + 1).min(text.len());
        let limit = (body + PEM_MAX).min(text.len());
        let mut j = body;
        while j < limit && !text[j..].starts_with(b"-----END") {
            j += 1;
        }
        if j > body {
            let mut end = j;
            // 不吞掉正文最后一行的换行，BEGIN/END 的排版原样保留
            if text[end - 1] == b'\n' {
                end -= 1;
            }
            if end > body {
                spans.push(Span { start: body, end, kind: "PRIVATEKEY" });
            }
        }
        i = j.max(at + 1);
    }
}

/// `Bearer <token>`（大小写不敏感，值至少 16 位）
pub(crate) fn bearer_spans(text: &[u8], spans: &mut Vec<Span>) {
    let mut i = 0;
    while i + 7 <= text.len() {
        let boundary = i == 0 || !text[i - 1].is_ascii_alphanumeric();
        if boundary
            && text[i].to_ascii_lowercase() == b'b'
            && text[i + 1..].starts_with(b"earer ")
        {
            let vs = i + 7;
            let mut ve = vs;
            while ve < text.len() && ve - vs < VALUE_MAX && is_value_char(text[ve]) {
                ve += 1;
            }
            if ve - vs >= BEARER_MIN {
                spans.push(Span { start: vs, end: ve, kind: "TOKEN" });
                i = ve;
                continue;
            }
            i += 7;
            continue;
        }
        i += 1;
    }
}

/// 凭证键名清单：只做整名精确匹配（`max_tokens`、`tokenizer` 天然不命中），
/// 覆盖 `token=…`、`Authorization: …`、`api_key: …` 等常见写法。
pub(crate) const KV_KEYS: [&str; 22] = [
    "token", "access_token", "refresh_token", "id_token", "auth_token", "session_token", "api_token", "bot_token", "app_token",
    "api_key", "api-key", "apikey", "x-api-key", "authorization", "proxy-authorization",
    "secret", "client_secret", "client-secret", "secret_key", "secret-key",
    "password", "passwd",
];
/// 值是这些类型名时不遮（`token: string`）
pub(crate) const VALUE_WORDS: [&str; 12] = [
    "string", "number", "boolean", "bool", "null", "none", "undefined", "true", "false", "object", "bytes", "text",
];

/// `token=<value>`、`api_key: <value>`、`"Authorization": "Basic …"` 等。
/// 键名按完整标识符匹配，值为连续非空白段；`Bearer `/`Basic `/`token ` 前缀不算值
/// 的一部分（与 handoff_facts::find_value 的 SCHEMES 一致）。
pub(crate) fn kv_spans(text: &[u8], spans: &mut Vec<Span>) {
    let mut i = 0;
    while i < text.len() {
        if !is_ident_char(text[i]) {
            i += 1;
            continue;
        }
        let ks = i;
        while i < text.len() && is_ident_char(text[i]) {
            i += 1;
        }
        let lower = text[ks..i].to_ascii_lowercase();
        let Ok(name) = std::str::from_utf8(&lower) else { continue };
        if !KV_KEYS.contains(&name) {
            continue;
        }
        let Some((vs, ve)) = kv_value(text, i) else { continue };
        if !kv_value_ok(&text[vs..ve]) {
            continue;
        }
        // 键名带 key 的按 API Key 记，其余（token / authorization / password）按令牌记
        let kind = if name.contains("key") { "APIKEY" } else { "TOKEN" };
        spans.push(Span { start: vs, end: ve, kind });
        i = i.max(ve);
    }
}

/// 键名之后的值：支持 `KEY=值`、`KEY: 值`、`"KEY": "值"` 与工具输出里常见的
/// 转义 JSON `\"KEY\":\"值\"`。返回值区间。
pub(crate) fn kv_value(text: &[u8], mut j: usize) -> Option<(usize, usize)> {
    // 键的收尾引号（可带转义反斜杠）
    if text.get(j) == Some(&b'\\') && text.get(j + 1) == Some(&b'"') {
        j += 2;
    } else if text.get(j) == Some(&b'"') {
        j += 1;
    }
    while matches!(text.get(j), Some(b' ') | Some(b'\t')) {
        j += 1;
    }
    match text.get(j) {
        Some(b'=' | b':') => j += 1,
        _ => return None,
    }
    while matches!(text.get(j), Some(b' ') | Some(b'\t')) {
        j += 1;
    }
    // 值的开头引号（可带转义反斜杠）；引号里的值取到配对引号
    let escaped_quote = text.get(j) == Some(&b'\\') && text.get(j + 1) == Some(&b'"');
    let quoted = if text.get(j) == Some(&b'\\') && text.get(j + 1) == Some(&b'"') {
        j += 2;
        true
    } else if text.get(j) == Some(&b'"') {
        j += 1;
        true
    } else {
        false
    };
    if !quoted {
        // Bearer / Basic / token 前缀不算值的一部分
        for scheme in ["bearer ", "basic ", "token "] {
            if text[j..]
                .get(..scheme.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(scheme.as_bytes()))
            {
                j += scheme.len();
                break;
            }
        }
    }
    let vs = j;
    let mut ve = vs;
    while ve < text.len()
        && ve - vs < VALUE_MAX
        && if quoted {
            text[ve] != b'"' && text[ve] != b'\n'
        } else {
            is_value_char(text[ve])
        }
    {
        ve += 1;
    }
    // 转义引号的值以 `\"` 结束：收尾反斜杠不算值的一部分（与 handoff_facts::find_value 一致）
    if escaped_quote && ve > vs && text[ve - 1] == b'\\' && text.get(ve) == Some(&b'"') {
        ve -= 1;
    }
    Some((vs, ve))
}

/// 值要不要遮：长度够、不是纯数字（`"token": 12345678` 这类计数）、不是类型名、
/// 也不已经是占位符
pub(crate) fn kv_value_ok(value: &[u8]) -> bool {
    if value.len() < SECRET_MIN || value.starts_with(b"{{") {
        return false;
    }
    if value.iter().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    !VALUE_WORDS.iter().any(|word| lower.as_slice() == word.as_bytes())
}

pub(crate) fn is_local_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'%' | b'+' | b'-')
}

/// 邮箱：本地部分（≤64 字）@ 域名（至少两段，末段为 2–24 位纯字母的 TLD）。
/// `user@localhost`、`app@2.0.1`（TLD 不是纯字母）这类形似串不命中。
pub(crate) fn email_spans(text: &[u8], spans: &mut Vec<Span>) {
    const LOCAL_MAX: usize = 64;
    const DOMAIN_MAX: usize = 255;
    for at in 0..text.len() {
        if text[at] != b'@' {
            continue;
        }
        let mut ls = at;
        while ls > 0 && at - ls < LOCAL_MAX && is_local_char(text[ls - 1]) {
            ls -= 1;
        }
        if ls == at {
            continue;
        }
        let ds = at + 1;
        let mut de = ds;
        let mut labels = 1usize;
        let mut tld_start = ds;
        while de < text.len() && de - ds < DOMAIN_MAX {
            let c = text[de];
            if c == b'.' {
                if de == ds || text[de - 1] == b'.' {
                    break;
                }
                labels += 1;
                de += 1;
                tld_start = de;
                continue;
            }
            if c.is_ascii_alphanumeric() || c == b'-' {
                de += 1;
                continue;
            }
            break;
        }
        if labels < 2 || de == tld_start {
            continue;
        }
        let tld = &text[tld_start..de];
        if tld.len() < 2 || tld.len() > 24 || !tld.iter().all(|b| b.is_ascii_alphabetic()) {
            continue;
        }
        spans.push(Span { start: ls, end: de, kind: "EMAIL" });
    }
}

pub(crate) fn card_spans(text: &[u8], spans: &mut Vec<Span>) {
    let mut ranges = Vec::new();
    collect_card_ranges(text, &mut ranges);
    spans.extend(ranges.into_iter().map(|(start, end)| Span { start, end, kind: "CARD" }));
}

pub(crate) fn idcard_spans(text: &[u8], spans: &mut Vec<Span>) {
    let mut ranges = Vec::new();
    collect_idcard_ranges(text, &mut ranges);
    spans.extend(ranges.into_iter().map(|(start, end)| Span { start, end, kind: "IDCARD" }));
}

/// 汇总所有规则的命中并去重叠：同起点先取长的，与已选区间重叠的丢弃
/// （两个规则命中同一段时值相同，去掉重复没有损失）。
pub(crate) fn collect_spans(text: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    prefix_key_spans(text, &mut spans);
    pem_spans(text, &mut spans);
    bearer_spans(text, &mut spans);
    kv_spans(text, &mut spans);
    email_spans(text, &mut spans);
    card_spans(text, &mut spans);
    idcard_spans(text, &mut spans);
    spans.retain(|span| span.end > span.start);
    spans.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut kept: Vec<Span> = Vec::with_capacity(spans.len());
    let mut cursor = 0usize;
    for span in spans {
        if span.start < cursor {
            continue;
        }
        cursor = span.end;
        kept.push(span);
    }
    kept
}

/* ───────────────────────────── Masker ───────────────────────────── */

/// 脱敏引擎：占位符 → 原值的内存表。
pub(crate) struct Masker {
    /// 只在内存；不落盘、不进日志、不进导出（AGENTS：凭据只在后端）。
    pub(crate) values: Mutex<HashMap<String, String>>,
}

impl Masker {
    pub(crate) fn new() -> Self {
        Self { values: Mutex::new(HashMap::new()) }
    }

    /// 占位符表现在握着多少个不同的原值（状态查询用；单次请求的命中数见
    /// [`Masker::mask_body`] 的返回值）。
    pub(crate) fn hit_count(&self) -> usize {
        self.values
            .lock()
            .map(|values| values.len())
            .unwrap_or_else(|poisoned| poisoned.into_inner().len())
    }

    /// 引擎是否可用：占位符表一旦被毒化（持锁线程 panic）就永久视为不可用，
    /// 调用方必须拒发（503），绝不原样放行。
    pub(crate) fn available(&self) -> bool {
        !self.values.is_poisoned()
    }

    /// 脱敏 JSON 请求体：只处理字符串值（顶层与嵌套 messages / content 等），
    /// 键名与数字不动。
    /// - `Ok(Some((bytes, hits)))`：命中并已改写；
    /// - `Ok(None)`：不是 JSON 或没有命中——调用方放行原文（不是 JSON 的请求体不强脱）；
    /// - `Err(())`：占位符表不可写——调用方必须拒发（503），绝不放行原文。
    ///
    /// 返回类型比 magpie 的对应入口多一层 Result：脱敏开启却无法保证「换回」时，
    /// 宁可拒发也不能让改写后的请求离开本机。
    #[cfg(test)]
    pub(crate) fn mask_body(&self, body: &[u8]) -> Result<Option<(axum::body::Bytes, usize)>, ()> {
        let (masked, report) = self.mask_body_with(body, &Policy::default())?;
        Ok(masked.map(|bytes| (bytes, report.masked)))
    }

    /// 按策略脱敏，连同命中明细一起返回。改写过的请求体只在真的换了占位符时给出。
    pub(crate) fn mask_body_with(&self, body: &[u8], policy: &Policy) -> Result<(Option<axum::body::Bytes>, Report), ()> {
        let mut report = Report::default();
        // 不是 JSON：不强脱，放行原文
        let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
            return Ok((None, report));
        };
        self.mask_value(&mut value, policy, &mut report)?;
        let masked = (report.masked > 0).then(|| axum::body::Bytes::from(value.to_string()));
        Ok((masked, report))
    }

    pub(crate) fn mask_value(&self, value: &mut serde_json::Value, policy: &Policy, report: &mut Report) -> Result<(), ()> {
        match value {
            serde_json::Value::String(text) => {
                if let Some(masked) = self.mask_text(text, policy, report)? {
                    *text = masked;
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    self.mask_value(item, policy, report)?;
                }
            }
            serde_json::Value::Object(fields) => {
                for (_, field) in fields.iter_mut() {
                    self.mask_value(field, policy, report)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 单个字符串值的脱敏。None = 没有换任何占位符。占位符先记上表再拼进结果：
    /// 表不可写时整个请求按失败处理（拒发）。豁免清单里的值不算命中；「只记录」与
    /// 「拦下」的值原样留在文本里，只进报告。
    pub(crate) fn mask_text(&self, text: &str, policy: &Policy, report: &mut Report) -> Result<Option<String>, ()> {
        let spans: Vec<(usize, usize, &'static str)> = collect_spans(text.as_bytes())
            .into_iter()
            .map(|span| (floor_boundary(text, span.start), floor_boundary(text, span.end), span.kind))
            .filter(|(start, end, _)| end > start && !policy.allows(&text[*start..*end]))
            .collect();
        if spans.is_empty() {
            return Ok(None);
        }
        let mut out = String::with_capacity(text.len());
        let mut cursor = 0usize;
        let mut changed = false;
        for (i, &(start, end, kind)) in spans.iter().enumerate() {
            if start < cursor {
                continue;
            }
            out.push_str(&text[cursor..start]);
            let value = &text[start..end];
            let action = policy.action(kind);
            let placeholder = if action == Action::Mask {
                let ph = placeholder(kind, value);
                let mut values = self.values.lock().map_err(|_| ())?;
                values.entry(ph.clone()).or_insert_with(|| value.to_string());
                Some(ph)
            } else {
                None
            };
            if report.hits.len() < REPORT_HITS_MAX {
                let next = spans.get(i + 1).map(|s| s.0.max(end)).unwrap_or(text.len());
                report.hits.push(Hit {
                    kind,
                    action: action.as_str(),
                    length: value.chars().count(),
                    preview: preview(kind, value),
                    placeholder: placeholder.clone(),
                    left: tail_chars(&out, CONTEXT_CHARS),
                    right: head_chars(&text[end..next], CONTEXT_CHARS),
                });
            }
            match (action, placeholder) {
                (Action::Mask, Some(ph)) => {
                    out.push_str(&ph);
                    report.masked += 1;
                    changed = true;
                }
                (Action::Block, _) => {
                    if !report.blocked.contains(&kind) {
                        report.blocked.push(kind);
                    }
                    out.push_str(value);
                }
                _ => out.push_str(value),
            }
            cursor = end;
        }
        out.push_str(&text[cursor..]);
        Ok(changed.then_some(out))
    }

    /// 占位符换回真值（响应回填用）。表里没有的形似占位符原样保留；
    /// 单趟从左到右替换，换进去的原值不再参与匹配（原值里就算真有
    /// `{{JOLU_…}`] 字样的文本也不会被二次改写）。
    pub(crate) fn unmark_text(&self, text: &str) -> String {
        let mut buf = text.as_bytes().to_vec();
        unmark_bytes(&self.values, &mut buf);
        String::from_utf8(buf)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
    }
}

/// 把缓冲里完整的占位符换回原值（单趟左到右）。表锁坏了也尽力恢复读：
/// 占位符本身不是机密，恢复不了最多原样露出，不构成泄漏。
pub(crate) fn unmark_bytes(values: &Mutex<HashMap<String, String>>, buf: &mut Vec<u8>) {
    if find_from(buf, 0, PLACEHOLDER_PREFIX.as_bytes()).is_none() {
        return;
    }
    let map = match values.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut i = 0usize;
    while let Some(at) = find_from(buf, i, PLACEHOLDER_PREFIX.as_bytes()) {
        let Some(close) = find_from(buf, at + PLACEHOLDER_PREFIX.len(), PLACEHOLDER_SUFFIX.as_bytes())
        else {
            i = at + 2;
            continue;
        };
        let end = close + PLACEHOLDER_SUFFIX.len();
        let Ok(token) = std::str::from_utf8(&buf[at..end]) else {
            i = at + 2;
            continue;
        };
        let Some(value) = map.get(token) else {
            i = end;
            continue;
        };
        buf.splice(at..end, value.bytes());
        i = at + value.len();
    }
}

/* ─────────────────────────── SSE 尾部保留缓冲 ─────────────────────────── */

/// SSE 流的回填状态机：占位符是纯 ASCII（`{{JOLU_…}}`），直接在字节流上做
/// 整串替换即可，不必解析 SSE 事件或 JSON——这是「对整个累积流做尾部保留缓冲」
/// 的取巧做法：比 magpie 按事件边界维护每条 text 的尾表更简单，且保证任何
/// 分块方式下字节序列不多不少（除占位符换写外原样透传）。
///
/// 每次喂入后：先替换缓冲里的完整占位符，再把「最后一个可能是占位符开头的
/// `{{`」之后的内容扣住不发，等下一块拼上。占位符最长 PLACEHOLDER_MAX_LEN，
/// 尾部长度达到上限仍凑不出完整占位符时全部放行；流结束时把余量一次放行。
pub(crate) struct StreamUnmarker<'a> {
    pub(crate) masker: &'a Masker,
    pub(crate) pending: Vec<u8>,
}

impl<'a> StreamUnmarker<'a> {
    pub(crate) fn new(masker: &'a Masker) -> Self {
        Self { masker, pending: Vec::new() }
    }

    /// 喂入一块，返回可以立即发出的字节（可能为空）。
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(chunk);
        unmark_bytes(&self.masker.values, &mut self.pending);
        // 在尾部窗口里找所有「可能是占位符开头」的 `{`：占位符一定以相邻的 `{{`
        // 开头（末字节的 `{` 也算候选，可能与下一块拼成 `{{`），内部不再有 `{`。
        // 取窗口里**最早**的候选扣住——从后往前扫、每次命中都更新，因为
        // `…{{J` 结尾时最后一个 `{` 是真占位符的第二个花括号，扣它会把
        // `{{` 拆开。窗口内凑不出完整占位符（≥ PLACEHOLDER_MAX_LEN）就停。
        let len = self.pending.len();
        let mut hold_start = len;
        for p in (0..len).rev() {
            if len - p >= PLACEHOLDER_MAX_LEN {
                break;
            }
            if self.pending[p] == b'{' && (p + 1 == len || self.pending[p + 1] == b'{') {
                hold_start = p;
            }
        }
        let tail = self.pending.split_off(hold_start);
        std::mem::replace(&mut self.pending, tail)
    }

    /// 流结束：回填并放行扣住的余量。
    pub(crate) fn finish(&mut self) -> Vec<u8> {
        unmark_bytes(&self.masker.values, &mut self.pending);
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{LazyLock, Mutex};

    /// 测试串行化进程级开关（ENABLED 是全局状态，改动的测试必须互斥）
    pub(crate) static GATE: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
}
