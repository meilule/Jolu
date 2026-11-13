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

use axum::body::Body;
use axum::response::Response;
use sha2::{Digest, Sha256};

use crate::handoff_facts::{PREFIX_KEYS, PREFIX_MIN};

/* ───────────────────────────── 开关（进程级） ───────────────────────────── */

static ENABLED: AtomicBool = AtomicBool::new(false);
static MASKER: LazyLock<Masker> = LazyLock::new(Masker::new);

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
    actions: HashMap<&'static str, Action>,
    allow: HashSet<String>,
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

    fn action(&self, kind: &str) -> Action {
        self.actions.get(kind).copied().unwrap_or(Action::Mask)
    }

    fn allows(&self, value: &str) -> bool {
        !self.allow.is_empty() && self.allow.contains(&value.trim().to_lowercase())
    }
}

static POLICY: LazyLock<RwLock<Policy>> = LazyLock::new(|| RwLock::new(Policy::default()));

/// 设置页保存 / 启动时调用。
pub(crate) fn set_policy(policy: Policy) {
    if let Ok(mut current) = POLICY.write() {
        *current = policy;
    }
}

fn policy() -> Policy {
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
const REPORT_HITS_MAX: usize = 200;
/// 内存里最多留这么多条请求记录。
const RECORDS_MAX: usize = 100;
const CONTEXT_CHARS: usize = 24;

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

static RECORDS: LazyLock<Mutex<VecDeque<MaskRecord>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));
static RECORD_SEQ: AtomicU64 = AtomicU64::new(1);
static FIRST_HOOK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static FIRST_FIRED: AtomicBool = AtomicBool::new(false);

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

fn remember(entry: &str, report: &Report) {
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

fn head_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

fn tail_chars(text: &str, n: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(n)).collect()
}

/// 原值的开头几个字（邮箱只露两个）。
fn preview(kind: &str, value: &str) -> String {
    let n = if kind == "EMAIL" { 2 } else { 4 };
    let shown: String = value.chars().take(n).collect();
    format!("{shown}…")
}

/// 「从不脱敏」清单里有没有这个值（交接摘要的脱敏也遵守）。
pub(crate) fn allowed_value(value: &str) -> bool {
    POLICY.read().is_ok_and(|p| p.allows(value))
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
const TYPES: [&str; 6] = ["APIKEY", "PRIVATEKEY", "TOKEN", "EMAIL", "CARD", "IDCARD"];

const PLACEHOLDER_PREFIX: &str = "{{JOLU_";
const PLACEHOLDER_SUFFIX: &str = "}}";
const HASH_CHARS: usize = 8;

const fn longest_type_len() -> usize {
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
const PLACEHOLDER_MAX_LEN: usize =
    PLACEHOLDER_PREFIX.len() + longest_type_len() + 1 + HASH_CHARS + PLACEHOLDER_SUFFIX.len();

/// 占位符 = 原值的 SHA-256 前 4 字节（8 位小写 hex）。纯哈希、无密钥：
/// 同一原值在进程重启后也得到同一占位符；32 位前缀对高熵密钥不可反推。
fn placeholder(kind: &str, value: &str) -> String {
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
struct Span {
    start: usize,
    end: usize,
    kind: &'static str,
}

/// 单个值最多向前取这么多字节（与 handoff_facts::VALUE_MAX 同量级）
const VALUE_MAX: usize = 4_096;
/// 私钥块最多遮这么多字节（与 handoff_facts::PEM_MAX 一致）
const PEM_MAX: usize = 8_192;
/// Bearer / Basic 之后的令牌最短长度（与 handoff_facts::token_spans 一致）
const BEARER_MIN: usize = 16;
/// 凭证键值的最短长度（与 handoff_facts::SECRET_MIN 一致）
const SECRET_MIN: usize = 8;
/// 前缀 Key 之后没有分隔符的连续数字最少视为卡号（13 位是 Visa 最短，
/// 但 13 位也常是毫秒时间戳，未分组的短数字串宁可不碰）
const CARD_MIN_UNGROUPED: usize = 16;

fn is_value_char(c: u8) -> bool {
    !c.is_ascii_whitespace()
        && !matches!(c, b'"' | b'\'' | b'`' | b',' | b';' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'<' | b'>')
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// 把字节下标退回到字符边界：规则的长度上限截断可能落在多字节字符中间
fn floor_boundary(text: &str, mut at: usize) -> usize {
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// 无键名的前缀 Key：`sk-…`、`ghp_…`、`AKIA…` 等，前缀之后至少 PREFIX_MIN 位。
fn prefix_key_spans(text: &[u8], spans: &mut Vec<Span>) {
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
fn pem_spans(text: &[u8], spans: &mut Vec<Span>) {
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
fn bearer_spans(text: &[u8], spans: &mut Vec<Span>) {
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
const KV_KEYS: [&str; 22] = [
    "token", "access_token", "refresh_token", "id_token", "auth_token", "session_token", "api_token", "bot_token", "app_token",
    "api_key", "api-key", "apikey", "x-api-key", "authorization", "proxy-authorization",
    "secret", "client_secret", "client-secret", "secret_key", "secret-key",
    "password", "passwd",
];
/// 值是这些类型名时不遮（`token: string`）
const VALUE_WORDS: [&str; 12] = [
    "string", "number", "boolean", "bool", "null", "none", "undefined", "true", "false", "object", "bytes", "text",
];

/// `token=<value>`、`api_key: <value>`、`"Authorization": "Basic …"` 等。
/// 键名按完整标识符匹配，值为连续非空白段；`Bearer `/`Basic `/`token ` 前缀不算值
/// 的一部分（与 handoff_facts::find_value 的 SCHEMES 一致）。
fn kv_spans(text: &[u8], spans: &mut Vec<Span>) {
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
fn kv_value(text: &[u8], mut j: usize) -> Option<(usize, usize)> {
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
fn kv_value_ok(value: &[u8]) -> bool {
    if value.len() < SECRET_MIN || value.starts_with(b"{{") {
        return false;
    }
    if value.iter().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    !VALUE_WORDS.iter().any(|word| lower.as_slice() == word.as_bytes())
}

fn is_local_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'%' | b'+' | b'-')
}

/// 邮箱：本地部分（≤64 字）@ 域名（至少两段，末段为 2–24 位纯字母的 TLD）。
/// `user@localhost`、`app@2.0.1`（TLD 不是纯字母）这类形似串不命中。
fn email_spans(text: &[u8], spans: &mut Vec<Span>) {
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

/// Luhn 校验（卡号）
fn luhn_ok(digits: &[u8]) -> bool {
    let mut sum = 0u32;
    for (k, &d) in digits.iter().rev().enumerate() {
        let mut v = u32::from(d - b'0');
        if k % 2 == 1 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    sum % 10 == 0
}

/// 银行卡号：连续 ≥ 16 位，或按空格 / 连字符分组（每组 ≤ 6 位）合计 13–19 位，
/// 且 Luhn 校验通过。前后都不能是字母数字（切进 base64 / 标识符的不算），
/// 前面也不能是 `.`（小数）。校验位挡住时间戳、单号等大多数长数字串。
fn card_spans(text: &[u8], spans: &mut Vec<Span>) {
    let mut i = 0;
    while i < text.len() {
        if !text[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        if i > 0 && (text[i - 1].is_ascii_alphanumeric() || text[i - 1] == b'.') {
            i += 1;
            continue;
        }
        let start = i;
        let mut digits: Vec<u8> = Vec::with_capacity(24);
        let mut j = i;
        // 第一段：连续数字（未分组的整串）
        while j < text.len() && text[j].is_ascii_digit() {
            j += 1;
        }
        digits.extend_from_slice(&text[start..j]);
        // 后续分组：分隔符 + 至多 6 位
        while digits.len() < 19
            && matches!(text.get(j), Some(b' ' | b'-'))
            && text.get(j + 1).is_some_and(|c| c.is_ascii_digit())
        {
            let group_start = j + 1;
            let mut group_end = group_start;
            while group_end < text.len() && text[group_end].is_ascii_digit() && group_end - group_start < 6 {
                group_end += 1;
            }
            digits.extend_from_slice(&text[group_start..group_end]);
            j = group_end;
        }
        i = j.max(i + 1);
        // 出现过分隔符才算分组卡号；未分组的整串按更严的下限（CARD_MIN_UNGROUPED）
        let grouped = j - start > digits.len();
        if digits.len() < 13 || digits.len() > 19 {
            continue;
        }
        // 未分组的短串（13–15 位常见于时间戳 / 单号）不碰
        if !grouped && digits.len() < CARD_MIN_UNGROUPED {
            continue;
        }
        if text
            .get(j)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'.')
        {
            continue;
        }
        if luhn_ok(&digits) {
            spans.push(Span { start, end: j, kind: "CARD" });
        }
    }
}

/// 18 位居民身份证号：17 位数字 + 校验位（0-9X），出生日期段合法且加权校验 mod 11 通过。
const ID_WEIGHTS: [u32; 17] = [7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2];
const ID_CHECK: &[u8; 11] = b"10X98765432";

fn idcard_spans(text: &[u8], spans: &mut Vec<Span>) {
    if text.len() < 18 {
        return;
    }
    for i in 0..=text.len() - 18 {
        let window = &text[i..i + 18];
        if !window[..17].iter().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let last = window[17];
        if !last.is_ascii_digit() && last != b'X' && last != b'x' {
            continue;
        }
        // 边界：前后不能是字母数字（切进更长标识符的不算）
        if i > 0 && text[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        if text.get(i + 18).is_some_and(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let num = |range: std::ops::Range<usize>| -> u32 {
            window[range].iter().fold(0u32, |acc, b| acc * 10 + u32::from(b - b'0'))
        };
        let (year, month, day) = (num(6..10), num(10..12), num(12..14));
        if !(1900..=2099).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            continue;
        }
        let sum: u32 = window[..17]
            .iter()
            .enumerate()
            .map(|(k, b)| u32::from(b - b'0') * ID_WEIGHTS[k])
            .sum();
        let check = ID_CHECK[(sum % 11) as usize];
        if last != check && last != check.to_ascii_lowercase() {
            continue;
        }
        spans.push(Span { start: i, end: i + 18, kind: "IDCARD" });
    }
}

/// 汇总所有规则的命中并去重叠：同起点先取长的，与已选区间重叠的丢弃
/// （两个规则命中同一段时值相同，去掉重复没有损失）。
fn collect_spans(text: &[u8]) -> Vec<Span> {
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
    values: Mutex<HashMap<String, String>>,
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

    fn mask_value(&self, value: &mut serde_json::Value, policy: &Policy, report: &mut Report) -> Result<(), ()> {
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
    fn mask_text(&self, text: &str, policy: &Policy, report: &mut Report) -> Result<Option<String>, ()> {
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
fn unmark_bytes(values: &Mutex<HashMap<String, String>>, buf: &mut Vec<u8>) {
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
    masker: &'a Masker,
    pending: Vec<u8>,
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

/* ───────────────────────────── 网关接入点 ───────────────────────────── */

/// 响应侧缓冲上限（与 translate.rs 的上游缓冲同量级）
const RESPONSE_BUFFER_LIMIT: usize = 64 * 1024 * 1024;

/// 请求侧入口（server.rs 在鉴权后、选路前对三类协议端点调用）。
/// - 关闭：原样；
/// - 开启且引擎可用：命中才改写，未命中原样；
/// - 开启但引擎不可用：`Err`（503 拒发）——绝不原样放行。
pub(crate) fn prepare_request(body: axum::body::Bytes, entry: &str) -> Result<axum::body::Bytes, Response> {
    let Some(masker) = request_masker() else { return Ok(body) };
    let policy = policy();
    let masked = if masker.available() { masker.mask_body_with(&body, &policy) } else { Err(()) };
    match masked {
        Ok((masked, report)) => {
            remember(entry, &report);
            if !report.blocked.is_empty() {
                let kinds: Vec<&str> = report.blocked.iter().map(|k| kind_label(k)).collect();
                return Err(crate::gateway::wire::error_response(
                    axum::http::StatusCode::BAD_REQUEST,
                    &format!("请求里带有设为「拦下」的{}，Jolu 没有把它发出；可在设置 › 隐私脱敏里改成「遮住」", kinds.join("、")),
                ));
            }
            match masked {
                Some(masked) => {
                    eprintln!("[网关] 隐私脱敏：请求体遮蔽 {} 处", report.masked);
                    Ok(masked)
                }
                None => Ok(body),
            }
        }
        Err(()) => Err(crate::gateway::wire::error_response(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "隐私脱敏已开启但引擎暂不可用，为避免原文离开本机已拒绝本次请求",
        )),
    }
}

/// 响应侧入口（server.rs 在 wire::handle 返回后调用）：把响应里的占位符换回真值。
/// - SSE：逐块回填，尾部保留缓冲处理被分块切割的占位符；
/// - JSON / 文本：整段缓冲后回填；
/// - 其余二进制：不动。
///
/// 回填是尽力而为：占位符本身不是机密，恢复不了最多原样露出占位符，不构成泄漏；
/// 这与请求侧「失败即拒发」不同——响应已经从上游回来了，拦下它没有安全收益。
/// 响应侧中间件的装饰层（think-tags）：非流式 JSON 整段处理；`text/event-stream`
/// 逐事件交给 `middleware::EventChain`（onEvent）。
pub(crate) async fn restore_middleware(response: Response, middlewares: &[crate::gateway::middleware::Middleware]) -> Response {
    if !middlewares.iter().any(|m| m.enabled && m.kind == "think-tags") {
        return response;
    }
    let is_stream = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("event-stream"));
    if is_stream {
        // 流式：逐事件处理（onEvent，3.7）
        let Some(chain) = crate::gateway::middleware::EventChain::new(middlewares) else { return response };
        let (mut parts, body) = response.into_parts();
        parts.headers.remove(axum::http::header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::from_stream(crate::gateway::middleware::transform_sse(body.into_data_stream(), chain)));
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
        // 读不出来就交回原响应——响应侧中间件是装饰，不能因此让请求失败
        return Response::from_parts(parts, axum::body::Body::empty());
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Response::from_parts(parts, axum::body::Body::from(bytes));
    };
    let next = crate::gateway::middleware::restore_chain(middlewares, text);
    let next_bytes = next.into_bytes();
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    if let Ok(value) = axum::http::HeaderValue::from_str(&next_bytes.len().to_string()) {
        parts.headers.insert(axum::http::header::CONTENT_LENGTH, value);
    }
    Response::from_parts(parts, axum::body::Body::from(next_bytes))
}

pub(crate) async fn restore_response(response: Response) -> Response {
    let Some(masker) = request_masker() else { return response };
    // 本进程从没遮过任何值：响应里不可能有占位符，直接放行
    if masker.hit_count() == 0 {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let content_type = parts
        .headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if content_type.contains("text/event-stream") {
        let unmarker = StreamUnmarker::new(masker);
        use futures_util::StreamExt as _;
        let stream = futures_util::stream::unfold(
            (body.into_data_stream(), unmarker, false),
            |(mut upstream, mut unmarker, ended)| async move {
                if ended {
                    return None;
                }
                loop {
                    match upstream.next().await {
                        Some(Ok(chunk)) => {
                            let out = unmarker.feed(&chunk);
                            if !out.is_empty() {
                                return Some((
                                    Ok::<axum::body::Bytes, std::convert::Infallible>(axum::body::Bytes::from(out)),
                                    (upstream, unmarker, false),
                                ));
                            }
                        }
                        // 上游结束（或出错）：把扣住的尾部回填后放行
                        _ => {
                            let out = unmarker.finish();
                            return Some((Ok(axum::body::Bytes::from(out)), (upstream, unmarker, true)));
                        }
                    }
                }
            },
        );
        // 分块边界变了：原 Content-Length 不再成立
        parts.headers.remove(axum::http::header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::from_stream(stream));
    }
    if !content_type.contains("json") && !content_type.contains("text") {
        // 二进制（图片 / 音频等）：没有占位符可言，原样透传
        return Response::from_parts(parts, body);
    }
    let bytes = match axum::body::to_bytes(body, RESPONSE_BUFFER_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return crate::gateway::wire::error_response(
                axum::http::StatusCode::BAD_GATEWAY,
                &format!("读取上游回复失败：{error}"),
            );
        }
    };
    let restored = if find_from(&bytes, 0, PLACEHOLDER_PREFIX.as_bytes()).is_some() {
        match std::str::from_utf8(&bytes) {
            Ok(text) => masker.unmark_text(text).into_bytes(),
            Err(_) => bytes.to_vec(),
        }
    } else {
        bytes.to_vec()
    };
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(restored))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{LazyLock, Mutex};

    /// 测试串行化进程级开关（ENABLED 是全局状态，改动的测试必须互斥）
    pub(crate) static GATE: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 从脱敏后的 JSON 文本里取出第一个占位符
    fn first_placeholder(text: &str) -> String {
        let at = text.find(PLACEHOLDER_PREFIX).expect("应有占位符");
        let close = text[at..].find(PLACEHOLDER_SUFFIX).unwrap() + at + PLACEHOLDER_SUFFIX.len();
        text[at..close].to_string()
    }

    fn mask_string(masker: &Masker, text: &str) -> String {
        let body = format!(r#"{{"text":{}}}"#, serde_json::to_string(text).unwrap());
        let Some((masked, _)) = masker.mask_body(body.as_bytes()).unwrap() else {
            panic!("应命中：{text}");
        };
        let value: serde_json::Value = serde_json::from_slice(&masked).unwrap();
        value["text"].as_str().unwrap().to_string()
    }

    /// 单字符串值的请求体
    fn body_of(text: &str) -> Vec<u8> {
        format!(r#"{{"text":{}}}"#, serde_json::to_string(text).unwrap()).into_bytes()
    }

    #[test]
    fn prefixed_keys_are_masked_with_stable_placeholders() {
        let masker = Masker::new();
        for secret in [
            "sk-ant-api03-ABCDEFGHIJKLMNOPQRST",
            "ghp_ABCDEFGHIJKLMNOPQRSTUV",
            "AKIAABCDEFGHIJKLMNOP",
            "ya29.a0AfH6SMBxLongToken123",
        ] {
            let masked = mask_string(&masker, &format!("key {secret} end"));
            assert!(!masked.contains(secret), "{secret} 不能原样出现：{masked}");
            let ph = first_placeholder(&masked);
            assert!(ph.starts_with("{{JOLU_APIKEY_") && ph.ends_with("}}"), "{ph}");
            // 同一原值再次出现：同一个占位符
            let again = mask_string(&masker, &format!("另一个 {secret}"));
            assert!(again.contains(&ph), "占位符必须稳定：{ph} vs {again}");
            // 回填精确
            assert_eq!(masker.unmark_text(&masked), format!("key {secret} end"));
        }
        // 短前缀不算 Key（与 handoff_facts 的「普通 sk-短」一致）
        let plain = body_of("普通 sk-短 文本");
        assert!(masker.mask_body(&plain).unwrap().is_none(), "短前缀不应命中");
    }

    #[test]
    fn pem_block_masks_body_between_marker_lines() {
        let masker = Masker::new();
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA1x\nab/cd==\n-----END RSA PRIVATE KEY-----";
        let masked = mask_string(&masker, pem);
        assert!(masked.starts_with("-----BEGIN RSA PRIVATE KEY-----"), "{masked}");
        assert!(masked.ends_with("-----END RSA PRIVATE KEY-----"), "{masked}");
        assert!(masked.contains("{{JOLU_PRIVATEKEY_"), "{masked}");
        assert!(!masked.contains("MIIEow"), "{masked}");
        assert_eq!(masker.unmark_text(&masked), pem);
    }

    #[test]
    fn bearer_authorization_and_kv_tokens_are_masked() {
        let masker = Masker::new();
        let cases: [(&str, &str, bool); 8] = [
            ("Authorization: Bearer abcdefghijklmnopqrstuvwxyz", "abcdefghijklmnopqrstuvwxyz", true),
            ("bearer abcdefghijklmnopqrstuvwxyz", "abcdefghijklmnopqrstuvwxyz", true),
            ("token=abcdefgh12345 保存好", "abcdefgh12345", true),
            (r#""refresh_token": "1//0gLongRefresh""#, "1//0gLongRefresh", true),
            ("api_key: abcdefgh12345", "abcdefgh12345", true),
            (r#"\"token\":\"abcdefgh12345\""#, "abcdefgh12345", true),
            ("max_tokens: 4096", "4096", false),
            ("tokenizer = bert-base-uncased", "bert-base", false),
        ];
        for (input, secret, should_mask) in cases {
            if !should_mask {
                assert!(
                    masker.mask_body(&body_of(input)).unwrap().is_none(),
                    "不应命中：{input}"
                );
                continue;
            }
            let masked = mask_string(&masker, input);
            assert!(!masked.contains(secret), "{input} → {masked}");
            assert_eq!(masker.unmark_text(&masked), input);
        }
        // 纯数字与类型名不是令牌
        assert!(masker.mask_body(br#"{"text":"token: 12345678"}"#).unwrap().is_none());
        assert!(masker.mask_body(br#"{"text":"token: string"}"#).unwrap().is_none());
        // Bearer 里的值太短不遮
        assert!(masker.mask_body(br#"{"text":"Bearer token"}"#).unwrap().is_none());
    }

    #[test]
    fn emails_are_masked_but_common_lookalikes_are_not() {
        let masker = Masker::new();
        let masked = mask_string(&masker, "发到 alice.test+dev@example.com 谢谢");
        assert!(!masked.contains("alice.test+dev@example.com"), "{masked}");
        assert!(masked.contains("{{JOLU_EMAIL_"), "{masked}");
        assert_eq!(masker.unmark_text(&masked), "发到 alice.test+dev@example.com 谢谢");
        for keep in ["user@localhost", "app@2.0.1", "@nobody", "a@b", "用户@@example.com"] {
            assert!(
                masker.mask_body(format!(r#"{{"text":{}}}"#, serde_json::to_string(keep).unwrap()).as_bytes())
                    .unwrap()
                    .is_none(),
                "不应命中：{keep}"
            );
        }
    }

    #[test]
    fn cards_and_id_numbers_need_valid_checksums() {
        let masker = Masker::new();
        let card = "4111 1111 1111 1111";
        let masked = mask_string(&masker, &format!("卡号 {card} 有效"));
        assert!(!masked.contains("4111"), "{masked}");
        assert!(masked.contains("{{JOLU_CARD_"), "{masked}");
        assert_eq!(masker.unmark_text(&masked), format!("卡号 {card} 有效"));
        // 校验位不对的长数字（时间戳、单号）不碰
        for keep in ["4111111111111112", "1737245500123", "订单号 9900000000000000"] {
            assert!(
                masker.mask_body(format!(r#"{{"text":{}}}"#, serde_json::to_string(keep).unwrap()).as_bytes())
                    .unwrap()
                    .is_none(),
                "不应命中：{keep}"
            );
        }
        // 身份证：11010519491231002X 是标准校验样例
        let id = "11010519491231002X";
        let masked = mask_string(&masker, &format!("身份证 {id}"));
        assert!(masked.contains("{{JOLU_IDCARD_"), "{masked}");
        assert_eq!(masker.unmark_text(&masked), format!("身份证 {id}"));
        // 校验位错一位就不遮
        assert!(masker.mask_body(br#"{"text":"110105194912310021"}"#).unwrap().is_none());
    }

    #[test]
    fn non_json_body_and_clean_body_pass_through() {
        let masker = Masker::new();
        // 不是 JSON：不强脱
        assert!(masker.mask_body(b"this is not json").unwrap().is_none());
        // JSON 但没有命中：原样
        assert!(
            masker
                .mask_body(&body_of(r#"你好，请用 kimi/kimi-k2 回答"#))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn masked_body_unmarks_back_to_the_original_value_tree() {
        let masker = Masker::new();
        let original = serde_json::json!({
            "model": "kimi/kimi-k2",
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "key sk-ant-api03-ABCDEFGHIJKLMNOPQRST\n发到 bob@example.com\ntoken=abcdefgh12345"}]},
                {"role": "user", "content": [{"type": "image", "source": {"type": "base64", "data": "iVBORw0KGgo="}}]}
            ]
        });
        let body = original.to_string();
        let (masked, hits) = masker.mask_body(body.as_bytes()).unwrap().unwrap();
        assert_eq!(hits, 3, "前缀 Key + 邮箱 + token= 三处");
        let masked_text = String::from_utf8(masked.to_vec()).unwrap();
        assert!(!masked_text.contains("sk-ant-api03") && !masked_text.contains("bob@example") && !masked_text.contains("abcdefgh12345"));
        // 图片 base64 不受影响
        let masked_value: serde_json::Value = serde_json::from_slice(&masked).unwrap();
        assert_eq!(masked_value["messages"][1]["content"][0]["source"]["data"], "iVBORw0KGgo=");
        // 整段回填后与原值树完全一致（占位符不含 JSON 转义字符，回填是精确替换）
        let restored: serde_json::Value = serde_json::from_str(&masker.unmark_text(&masked_text)).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn unknown_placeholders_are_left_alone() {
        let masker = Masker::new();
        let text = "看到 {{JOLU_FAKE_ab12cd34}} 和 {{JOLU_TOKEN_zzzz}} 保持原样";
        assert_eq!(masker.unmark_text(text), text);
    }

    #[test]
    fn hit_count_counts_distinct_values() {
        let masker = Masker::new();
        assert_eq!(masker.hit_count(), 0);
        let (_, hits) = masker
            .mask_body(br#"{"text":"token=abcdefgh12345 token=abcdefgh12345 token=zzzzzzz9999"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(hits, 3, "同值出现两次各算一处");
        assert_eq!(masker.hit_count(), 2, "去重后两个不同的原值");
    }

    #[test]
    fn stream_unmarker_restores_placeholders_split_at_every_offset() {
        let masker = Masker::new();
        // token= 形式：占位符对应的就是等号后的完整值，便于断言回填
        let secret = "abcdefgh12345";
        let (masked, _) = masker
            .mask_body(format!(r#"{{"messages":[{{"content":"token={secret}"}}]}}"#).as_bytes())
            .unwrap()
            .unwrap();
        let ph = first_placeholder(&String::from_utf8(masked.to_vec()).unwrap());
        let full = format!(
            "event: content_block_delta\r\ndata: {{\"delta\":{{\"text\":\"拿到 {ph} 完成\"}}}}\r\n\r\ndata: [DONE]\r\n\r\n"
        );
        let expect = full.replace(&ph, secret);
        let bytes = full.as_bytes();
        for split in 0..=bytes.len() {
            let mut unmarker = StreamUnmarker::new(&masker);
            let mut out = unmarker.feed(&bytes[..split]);
            out.extend(unmarker.feed(&bytes[split..]));
            out.extend(unmarker.finish());
            assert_eq!(String::from_utf8_lossy(&out), expect, "在第 {split} 字节切分");
        }
    }

    #[test]
    fn stream_unmarker_without_placeholders_passes_bytes_through() {
        let masker = Masker::new();
        let full = "event: delta\ndata: {\"text\":\"普通中文 delta\"}\n\n".repeat(3);
        for split in [0, 1, 7, 20, full.len() - 1, full.len()] {
            let mut unmarker = StreamUnmarker::new(&masker);
            let mut out = unmarker.feed(full[..split].as_bytes());
            out.extend(unmarker.feed(full[split..].as_bytes()));
            out.extend(unmarker.finish());
            assert_eq!(out, full.as_bytes(), "在第 {split} 字节切分");
        }
    }

    #[test]
    fn poisoned_table_fails_closed() {
        let masker = Masker::new();
        assert!(masker.available());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = masker.values.lock().unwrap();
            std::panic::panic_any("poison");
        }));
        assert!(result.is_err());
        assert!(!masker.available(), "毒化后必须判定引擎不可用");
        let body = axum::body::Bytes::from(r#"{"text":"token=abcdefgh12345"}"#);
        assert!(masker.mask_body(&body).is_err(), "表不可写时不能放行改写结果");
    }

    #[test]
    fn enabled_gate_controls_prepare_request() {
        let _guard = test_support::GATE.lock().unwrap();
        let body = axum::body::Bytes::from(r#"{"text":"token=abcdefgh12345"}"#);
        // 默认关闭：原样
        assert_eq!(prepare_request(body.clone(), "chat").unwrap(), body);
        set_enabled(true);
        let masked = prepare_request(body.clone(), "chat").unwrap();
        assert_ne!(masked, body, "开启后必须脱敏");
        assert!(!String::from_utf8_lossy(&masked).contains("abcdefgh12345"));
        let record = records().into_iter().next().expect("命中要留记录");
        assert_eq!(record.hits[0].kind, "TOKEN");
        assert!(!serde_json::to_string(&record).unwrap().contains("abcdefgh12345"), "记录里不能有原值");
        // 拦下：整个请求不发
        set_policy(Policy::new(&BTreeMap::from([("TOKEN".to_string(), "block".to_string())]), &[]).unwrap());
        let refused = prepare_request(body.clone(), "chat").unwrap_err();
        assert_eq!(refused.status(), axum::http::StatusCode::BAD_REQUEST);
        set_policy(Policy::default());
        set_enabled(false);
        assert_eq!(prepare_request(body.clone(), "chat").unwrap(), body, "关闭后必须原样");
    }

    #[test]
    fn actions_and_the_allow_list_change_what_is_sent() {
        let masker = Masker::new();
        let body = br#"{"text":"mail a@example.com key sk-ant-api03-ABCDEFGHIJKLMNOPQRST team@corp.io"}"#;
        let policy = Policy::new(
            &BTreeMap::from([("EMAIL".to_string(), "warn".to_string())]),
            &["TEAM@corp.io".to_string()],
        )
        .unwrap();
        let (masked, report) = masker.mask_body_with(body, &policy).unwrap();
        let text = String::from_utf8(masked.unwrap().to_vec()).unwrap();
        assert!(text.contains("a@example.com"), "只记录的种类原样发出：{text}");
        assert!(!text.contains("sk-ant-api03"), "其余种类照常遮住：{text}");
        assert!(text.contains("team@corp.io"), "豁免清单整段匹配、不分大小写");
        assert_eq!(report.masked, 1);
        assert_eq!(report.hits.len(), 2, "豁免的值不算命中");
        assert_eq!(report.hits[0].action, "warn");
        assert_eq!(report.hits[0].preview, "a@…");
        assert!(report.hits[1].left.contains("a@example.com"), "前文取自发出的文本（只记录的值原样）：{}", report.hits[1].left);
        assert!(Policy::new(&BTreeMap::from([("EMAIL".to_string(), "drop".to_string())]), &[]).is_err());
        assert!(Policy::new(&BTreeMap::from([("NAME".to_string(), "mask".to_string())]), &[]).is_err());
    }
}

#[cfg(test)]
mod non_ascii_tests {
    use super::*;

    /// 2026-11-13 交接里记的「脱敏把中文请求体改坏」：脱敏对中文 JSON 往返无损，
    /// 真正的原因是调用方发了非 UTF-8 字节（见 wire::missing_model_reason）。
    #[test]
    fn chinese_bodies_survive_masking_and_gbk_bytes_get_a_clear_reason() {
        let masker = Masker::new();
        let body = r#"{"model":"zcode/glm-5","messages":[{"role":"user","content":"你好，邮箱 a.b@example.com，没别的"}]}"#;
        let (masked, report) = masker.mask_body_with(body.as_bytes(), &Policy::default()).unwrap();
        let masked = masked.expect("邮箱应被遮住");
        assert_eq!(report.masked, 1);
        let value: serde_json::Value = serde_json::from_slice(&masked).expect("改写后仍是合法 JSON");
        assert_eq!(value["model"], "zcode/glm-5");
        assert!(value["messages"][0]["content"].as_str().unwrap().starts_with("你好，邮箱 "));
        let plain = r#"{"model":"m","messages":[{"role":"user","content":"你好"}]}"#;
        assert!(masker.mask_body_with(plain.as_bytes(), &Policy::default()).unwrap().0.is_none(), "没有命中就放行原文");
        // 「你好」的 GBK 字节：不是 UTF-8
        let gbk: Vec<u8> = [&br#"{"model":"m","content":""#[..], &[0xC4, 0xE3, 0xBA, 0xC3], &br#""}"#[..]].concat();
        assert!(crate::gateway::wire::missing_model_reason(&gbk).contains("UTF-8"));
        assert!(crate::gateway::wire::missing_model_reason(b"{}").contains("缺少 model"));
    }
}
