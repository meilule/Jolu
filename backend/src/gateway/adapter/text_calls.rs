//! 模型以文本写出的工具调用：`<tool_call>{json}</tool_call>`、Qwen 的
//! `<function=…><parameter=…>`、Anthropic 的 `<invoke name="…">`、GLM 自己的模板
//! （`<tool_call>名<arg_key>k</arg_key><arg_value>v</arg_value>`）与 DeepSeek 的 DSML
//! （`<｜DSML｜invoke name="…">`，先归一成 `<invoke>` 再按同一套规则读，3.5）。
//! 移植自 magpie-community/plugins `packages/qoder` 的 Splitter（MIT）：
//! 完整调用块到齐前先扣住文本；写了一半或读不懂的调用是错误，不当作回答。
use serde_json::Value;

const CALL_OPEN: &str = "<tool_call>";
const CALL_CLOSE: &str = "</tool_call>";
const XML_STARTS: [&str; 3] = ["<invoke ", concat!("<", "antml:", "invoke "), "<function="];
const XML_MARKS: [&str; 5] = ["<function_calls>", concat!("<", "antml:", "function_calls>"), "<invoke ", concat!("<", "antml:", "invoke "), "<function="];
const XML_CLOSES: [&str; 5] = ["</function_calls>", concat!("</", "antml:", "function_calls>"), "</invoke>", concat!("</", "antml:", "invoke>"), "</function>"];
pub(super) const CUT_SHORT: &str = "模型以文本写出的工具调用被截断，未作为回答转发";
pub(super) const UNREADABLE: &str = "模型以文本写出的工具调用无法解析，未作为回答转发";

#[derive(Debug, PartialEq)]
pub(super) enum Piece {
    Text(String),
    Call { name: String, args: String },
    Error(&'static str),
}

/// `<` 之后可选的 `antml:` 前缀。
fn tag(s: &str) -> Option<&str> {
    let rest = s.strip_prefix('<')?;
    Some(rest.strip_prefix("antml:").unwrap_or(rest))
}

/// `invoke` 后至少一个空白、`name`、空白、`=`；返回 `=` 之后的余下部分。
fn invoke_name_eq(rest: &str) -> Option<&str> {
    let after = rest.strip_prefix("invoke")?;
    let trimmed = after.trim_start();
    if trimmed.len() == after.len() {
        return None;
    }
    trimmed.strip_prefix("name")?.trim_start().strip_prefix('=')
}

/// 文本调用可能开始的位置（`<function_calls>`、`<invoke name=`、`<function=`）。
fn xml_at(s: &str) -> Option<usize> {
    s.match_indices('<').map(|(i, _)| i).find(|&i| {
        let Some(rest) = tag(&s[i..]) else { return false };
        rest.starts_with("function_calls>") || invoke_name_eq(rest).is_some() || (s[i..].starts_with("<function="))
    })
}

/// 末尾仍在写的开标签：`<invoke …` 或 `<function=…`，300 字符内没有 `<` `>`。
fn xml_hold(s: &str) -> Option<usize> {
    let i = s.rfind('<')?;
    let open = |rest: &str| -> bool { !rest.contains('>') && rest.chars().count() <= 300 };
    let raw = &s[i..];
    if let Some(rest) = raw.strip_prefix("<function=") {
        return open(rest).then_some(i);
    }
    let rest = tag(raw)?;
    let after = rest.strip_prefix("invoke")?;
    let mut chars = after.chars();
    (chars.next().is_some_and(char::is_whitespace) && open(chars.as_str())).then_some(i)
}

/// 代码块或行内代码之中：其中的标签是在描述调用，不是调用。
fn quoted(text: &str) -> bool {
    if text.matches("```").count() % 2 == 1 {
        return true;
    }
    let line = &text[text.rfind('\n').map_or(0, |i| i + 1)..];
    line.replace("```", "").matches('`').count() % 2 == 1
}

fn callish(v: &str) -> bool {
    v.trim_start().starts_with('{') || v.contains("<function=") || v.contains("<arg_key>")
}

/// GLM 的调用模板：工具名，然后每个参数 `<arg_key>k</arg_key><arg_value>v</arg_value>`。
/// 值按工具声明：字符串类型原样，否则当 JSON 读（读不懂仍按原文）。只有名字、没有参数的，
/// 只在名字确实是请求里的工具时才算调用（参照 magpie trae 插件 `glmCall`）。
fn glm_call(v: &str, tools: &[Value]) -> Option<(String, String)> {
    let v = v.trim();
    let at = v.find("<arg_key>");
    let name = at.map_or(v, |i| &v[..i]).trim();
    if name.is_empty() || name.len() > 128 || !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '-')) {
        return None;
    }
    if at.is_none() && !declared(tools, name) {
        return None;
    }
    let props = props(tools, name);
    let mut args = serde_json::Map::new();
    let mut rest = at.map_or("", |i| &v[i..]);
    while let Some(k) = rest.find("<arg_key>") {
        let after = &rest[k + "<arg_key>".len()..];
        let Some(key_end) = after.find("</arg_key>") else { break };
        let key = after[..key_end].trim();
        let after = &after[key_end + "</arg_key>".len()..];
        let Some(value_start) = after.find("<arg_value>") else { break };
        let after = &after[value_start + "<arg_value>".len()..];
        let Some(value_end) = after.find("</arg_value>") else { break };
        if !key.is_empty() {
            args.insert(key.into(), value_of(&after[..value_end], props.and_then(|p| p.get(key))));
        }
        rest = &after[value_end + "</arg_value>".len()..];
    }
    if at.is_some() && args.is_empty() {
        return None;
    }
    Some((name.into(), Value::Object(args).to_string()))
}

/// DeepSeek 的 DSML 标签（`<｜DSML｜…>`，竖线也可能是半角、带空格）归一成普通 XML，
/// 交给已有的 `<invoke>` / `<parameter>` 读法。返回（归一后的可交出部分，末尾可能是半个标签的待续部分）。
pub(super) fn normalize_dsml(input: &str) -> (String, String) {
    // 末尾若有一个还没闭合的 `<`，而且它后面可能是 DSML 标签开头，先扣住等下一块
    let hold_from = input.rfind('<').filter(|&i| {
        let tail = &input[i..];
        !tail.contains('>') && tail.chars().count() <= 48 && {
            let compact: String = tail.chars().filter(|c| !c.is_whitespace() && *c != '/').collect();
            let marker = compact.trim_start_matches('<');
            marker.is_empty() || "｜DSML｜".starts_with(marker) || "|DSML|".starts_with(marker) || marker.starts_with('｜') || marker.starts_with('|')
        }
    });
    let (ready, pending) = match hold_from {
        Some(i) => (&input[..i], &input[i..]),
        None => (input, ""),
    };
    let mut out = String::with_capacity(ready.len());
    let mut rest = ready;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find('>') else {
            out.push_str(tail);
            rest = "";
            break;
        };
        let tag = &tail[..=end];
        out.push_str(&dsml_tag(tag).unwrap_or_else(|| tag.to_string()));
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    (out, pending.to_string())
}

/// 一个 DSML 标签 → 等价的普通标签；不是 DSML 返回 None。
fn dsml_tag(tag: &str) -> Option<String> {
    let inner = tag.trim_start_matches('<').trim_end_matches('>').trim();
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, inner),
    };
    let after_bar = inner.strip_prefix('｜').or_else(|| inner.strip_prefix('|'))?;
    let after_bar = after_bar.trim_start_matches(|c: char| c == '｜' || c == '|' || c.is_whitespace());
    let rest = after_bar.strip_prefix("DSML")?;
    let rest = rest.trim_start_matches(|c: char| c == '｜' || c == '|' || c.is_whitespace());
    let slash = if closing { "/" } else { "" };
    if rest.starts_with("function_calls") || rest.starts_with("tool_calls") {
        return Some(format!("<{slash}function_calls>"));
    }
    if let Some(attrs) = rest.strip_prefix("invoke") {
        return Some(if closing { "</invoke>".into() } else { format!("<invoke{attrs}>") });
    }
    if let Some(attrs) = rest.strip_prefix("parameter") {
        // string="true" 这类附加属性去掉：值按工具声明的类型读
        let name = attrs.split_whitespace().find(|a| a.starts_with("name=")).unwrap_or("");
        return Some(if closing { "</parameter>".into() } else { format!("<parameter {name}>") });
    }
    None
}

fn props<'a>(tools: &'a [Value], name: &str) -> Option<&'a Value> {
    tools
        .iter()
        .find(|t| t.pointer("/function/name").and_then(Value::as_str) == Some(name))
        .and_then(|t| t.pointer("/function/parameters/properties"))
}

fn declared(tools: &[Value], name: &str) -> bool {
    tools.iter().any(|t| t.pointer("/function/name").and_then(Value::as_str) == Some(name))
}

/// 参数值：schema 说是字符串就原样，否则按 JSON 读，读不了仍是字符串。
fn value_of(v: &str, prop: Option<&Value>) -> Value {
    if prop.and_then(|p| p.get("type")).and_then(Value::as_str) == Some("string") {
        return Value::String(v.into());
    }
    serde_json::from_str(v.trim()).unwrap_or_else(|_| Value::String(v.into()))
}

/// `<tool_call>` 块内的一个调用：JSON `{name, arguments}`、`<function=…>` 或 GLM 模板。
fn parse_call(v: &str, tools: &[Value]) -> Option<(String, String)> {
    if let Ok(f) = serde_json::from_str::<Value>(v.trim()) {
        let name = f["name"].as_str().map(str::trim).unwrap_or_default();
        if !name.is_empty() && f["arguments"].is_object() {
            return Some((name.into(), f["arguments"].to_string()));
        }
    }
    if !v.contains("<function=") {
        return glm_call(v, tools);
    }
    let start = v.find("<function=")?;
    let rest = &v[start + "<function=".len()..];
    let gt = rest.find('>')?;
    let name = rest[..gt].trim();
    let close = rest[gt + 1..].find("</function>")?;
    let body = &rest[gt + 1..gt + 1 + close];
    if name.is_empty() {
        return None;
    }
    let props = props(tools, name);
    let mut args = serde_json::Map::new();
    let mut at = body;
    while let Some(i) = at.find("<parameter=") {
        let rest = &at[i + "<parameter=".len()..];
        let Some(gt) = rest.find('>') else { break };
        let Some(end) = rest[gt + 1..].find("</parameter>") else { break };
        let key = rest[..gt].trim();
        if !key.is_empty() {
            args.insert(key.into(), value_of(rest[gt + 1..gt + 1 + end].trim(), props.and_then(|p| p.get(key))));
        }
        at = &rest[gt + 1 + end + "</parameter>".len()..];
    }
    Some((name.into(), Value::Object(args).to_string()))
}

/// 参数开标签：`<parameter name="k">` 或 `<parameter=k>`，返回 (起点, 终点, 键)。
fn param_opens(s: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    for (i, _) in s.match_indices('<') {
        let Some(rest) = tag(&s[i..]) else { continue };
        let Some(after) = rest.strip_prefix("parameter") else { continue };
        let prefix = s[i..].len() - after.len();
        let (key, tail) = if let Some(v) = after.strip_prefix('=') {
            let end = v.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(v.len());
            if end == 0 {
                continue;
            }
            (v[..end].to_string(), &v[end..])
        } else {
            let trimmed = after.trim_start();
            if trimmed.len() == after.len() {
                continue;
            }
            let Some(v) = trimmed.strip_prefix("name").map(str::trim_start).and_then(|v| v.strip_prefix('=')).map(str::trim_start).and_then(|v| v.strip_prefix('"')) else { continue };
            let Some(q) = v.find('"') else { continue };
            if q == 0 {
                continue;
            }
            (v[..q].to_string(), &v[q + 1..])
        };
        let tail_trim = tail.trim_start();
        let Some(_) = tail_trim.strip_prefix('>') else { continue };
        let end = i + prefix + (after.len() - tail_trim.len()) + 1;
        out.push((i, end, key));
    }
    out
}

fn param_close(s: &str) -> Option<(usize, usize)> {
    for (i, _) in s.match_indices("</") {
        let rest = &s[i + 2..];
        let rest = rest.strip_prefix("antml:").unwrap_or(rest);
        if rest.starts_with("parameter>") {
            return Some((i, s.len() - rest.len() + "parameter>".len()));
        }
    }
    None
}

/// invoke / function 体内的参数，以及是否每个都已闭合。
fn xml_args(s: &str, props: Option<&Value>) -> (serde_json::Map<String, Value>, bool) {
    let opens = param_opens(s);
    let mut args = serde_json::Map::new();
    let mut closed = !opens.is_empty();
    for (k, (_, end, key)) in opens.iter().enumerate() {
        let stop = opens.get(k + 1).map_or(s.len(), |next| next.0);
        let mut v = &s[*end..stop];
        match param_close(v) {
            Some((c, _)) => v = &v[..c],
            None => closed = false,
        }
        let v = v.strip_prefix("\r\n").or_else(|| v.strip_prefix('\n')).unwrap_or(v);
        let v = v.strip_suffix("\r\n").or_else(|| v.strip_suffix('\n')).unwrap_or(v);
        args.insert(key.clone(), value_of(v, props.and_then(|p| p.get(key))));
    }
    (args, closed)
}

enum Xml {
    Wait,
    Text,
    Error(&'static str),
    Call { name: String, args: String, rest: String },
}

/// 第一个闭合标签与下一个调用开头。
fn close_of(body: &str, invoke: bool) -> Option<(usize, usize)> {
    for (i, _) in body.match_indices("</") {
        let rest = &body[i + 2..];
        let rest = if invoke { rest.strip_prefix("antml:").unwrap_or(rest) } else { rest };
        let word = if invoke { "invoke" } else { "function" };
        if let Some(after) = rest.strip_prefix(word) {
            let trimmed = after.trim_start();
            if trimmed.starts_with('>') {
                return Some((i, body.len() - trimmed.len() + 1));
            }
        }
    }
    None
}

fn next_call(body: &str) -> Option<usize> {
    body.match_indices('<').map(|(i, _)| i).find(|&i| {
        body[i..].starts_with("<function=") || tag(&body[i..]).and_then(invoke_name_eq).is_some()
    })
}

fn xml_call(s: &str, end: bool, tools: &[Value]) -> Xml {
    let mut s = s;
    if let Some(rest) = tag(s).and_then(|r| r.strip_prefix("function_calls>")) {
        let rest = rest.trim_start();
        if !end && (rest.is_empty() || XML_STARTS.iter().any(|k| k.starts_with(rest))) {
            return Xml::Wait;
        }
        s = rest;
    }
    let opened = if let Some(v) = tag(s).and_then(invoke_name_eq).map(str::trim_start).and_then(|v| v.strip_prefix('"')) {
        v.find('"').filter(|q| *q > 0 && !v[..*q].contains(['<', '>'])).and_then(|q| {
            let after = v[q + 1..].trim_start().strip_prefix('>')?;
            Some((v[..q].to_string(), after, true))
        })
    } else if let Some(v) = s.strip_prefix("<function=") {
        let stop = v.find(|c: char| c.is_whitespace() || c == '<' || c == '>').unwrap_or(v.len());
        (stop > 0).then(|| v[stop..].trim_start().strip_prefix('>').map(|after| (v[..stop].to_string(), after, false))).flatten()
    } else {
        None
    };
    let Some((name, body, invoke)) = opened else {
        let partial = tag(s).is_some_and(|r| {
            let rest = r.strip_prefix("invoke").filter(|a| a.starts_with(char::is_whitespace)).or_else(|| s.strip_prefix("<function="));
            rest.is_some_and(|a| !a.contains(['<', '>']) && a.chars().count() <= 300)
        });
        return if !end && (XML_STARTS.iter().any(|k| k.starts_with(s)) || partial) { Xml::Wait } else { Xml::Text };
    };
    let name = name.trim().to_string();
    if !declared(tools, &name) {
        return Xml::Text;
    }
    let close = close_of(body, invoke);
    let next = next_call(body);
    let mut stop = close.map(|c| c.0);
    if let Some(n) = next {
        if stop.is_none_or(|s| n < s) {
            stop = Some(n);
        }
    }
    let stop = match stop {
        Some(s) => s,
        None if !end => return Xml::Wait,
        None => body.len(),
    };
    let whole = close.is_some_and(|c| c.0 == stop);
    let (args, closed) = xml_args(&body[..stop], props(tools, &name));
    if !whole && !closed {
        let named = body[..stop].contains("<parameter") || body[..stop].contains(concat!("<", "antml:", "parameter"));
        return if named { Xml::Error(CUT_SHORT) } else { Xml::Text };
    }
    let rest = if whole { body[close.unwrap().1..].to_string() } else { body[stop..].to_string() };
    Xml::Call { name, args: Value::Object(args).to_string(), rest }
}

/// 一段流式文本的切分状态。
pub(super) struct Splitter {
    tools: Vec<Value>,
    /// DSML 归一时扣住的半个标签（等下一块拼上再归一）
    dsml_pending: String,
    text_buf: String,
    call_buf: String,
    in_call: bool,
    after: bool,
    sent: String,
    pub(super) saw_tool: bool,
}

enum Bare {
    Next(String),
    Held,
    Failed,
}

impl Splitter {
    pub(super) fn new(tools: &[Value]) -> Self {
        let tools = tools.iter().filter(|t| t.pointer("/function/name").and_then(Value::as_str).is_some()).cloned().collect();
        Self { tools, dsml_pending: String::new(), text_buf: String::new(), call_buf: String::new(), in_call: false, after: false, sent: String::new(), saw_tool: false }
    }

    fn text(&mut self, out: &mut Vec<Piece>, t: &str) {
        if t.is_empty() {
            return;
        }
        self.sent.push_str(t);
        out.push(Piece::Text(t.into()));
    }

    fn bare(&mut self, out: &mut Vec<Piece>, comb: &str, x: usize, end: bool) -> Bare {
        let (head, at) = comb.split_at(x);
        let opener_len = at[1..].find(['<', '>']).map_or(at.len(), |i| if at.as_bytes()[i + 1] == b'>' { i + 2 } else { i + 1 });
        let opener = &at[..opener_len];
        if quoted(&format!("{}{head}", self.sent)) {
            self.text(out, &format!("{head}{opener}"));
            return Bare::Next(at[opener_len..].into());
        }
        match xml_call(at, end, &self.tools) {
            Xml::Wait => {
                self.text(out, head);
                self.text_buf = at.into();
                Bare::Held
            }
            Xml::Text => {
                self.text(out, &format!("{head}{opener}"));
                Bare::Next(at[opener_len..].into())
            }
            Xml::Error(e) => {
                self.text(out, head);
                out.push(Piece::Error(e));
                Bare::Failed
            }
            Xml::Call { name, args, rest } => {
                self.text(out, head);
                self.saw_tool = true;
                out.push(Piece::Call { name, args });
                self.after = true;
                Bare::Next(rest)
            }
        }
    }

    /// 文本调用结束后紧跟的闭合标签与空白丢弃；仍可能是闭合标签时返回 None。
    fn left(&mut self, comb: &str, end: bool) -> Option<String> {
        let mut rest = comb;
        loop {
            let trimmed = rest.trim_start();
            if trimmed.len() != rest.len() {
                rest = trimmed;
                continue;
            }
            if let Some(after) = rest.strip_prefix("</") {
                let after = after.strip_prefix("antml:").unwrap_or(after);
                if let Some(next) = ["function_calls>", "invoke>", "function>"].iter().find_map(|w| after.strip_prefix(w)) {
                    rest = next;
                    continue;
                }
            }
            break;
        }
        if !end && (rest.is_empty() || XML_CLOSES.iter().any(|c| c.len() > rest.len() && c.starts_with(rest))) {
            return None;
        }
        self.after = false;
        Some(rest.into())
    }

    pub(super) fn feed(&mut self, input: &str) -> Vec<Piece> {
        let combined = std::mem::take(&mut self.dsml_pending) + input;
        let (ready, pending) = normalize_dsml(&combined);
        self.dsml_pending = pending;
        self.feed_normalized(&ready)
    }

    fn feed_normalized(&mut self, input: &str) -> Vec<Piece> {
        let mut out = Vec::new();
        let mut input = input.to_string();
        while !input.is_empty() {
            if !self.in_call {
                let mut comb = std::mem::take(&mut self.text_buf) + &input;
                if self.after {
                    match self.left(&comb, false) {
                        None => {
                            self.text_buf = comb;
                            break;
                        }
                        Some(rest) => {
                            comb = rest;
                            if comb.is_empty() {
                                break;
                            }
                        }
                    }
                }
                let i = comb.find(CALL_OPEN);
                let x = if self.tools.is_empty() { None } else { xml_at(&comb) };
                if let Some(x) = x.filter(|x| i.is_none_or(|i| *x < i)) {
                    match self.bare(&mut out, &comb, x, false) {
                        Bare::Failed => return out,
                        Bare::Held => break,
                        Bare::Next(next) => {
                            input = next;
                            continue;
                        }
                    }
                }
                let Some(i) = i else {
                    let mut keep = 0;
                    let marks: Vec<&str> = if self.tools.is_empty() { vec![CALL_OPEN] } else { std::iter::once(CALL_OPEN).chain(XML_MARKS).collect() };
                    for mark in marks {
                        for n in (keep + 1..=(mark.len() - 1).min(comb.len())).rev() {
                            if comb.ends_with(&mark[..n]) {
                                keep = n;
                                break;
                            }
                        }
                    }
                    if let Some(h) = if self.tools.is_empty() { None } else { xml_hold(&comb) } {
                        keep = keep.max(comb.len() - h);
                    }
                    let cut = comb.len() - keep;
                    self.text(&mut out, &comb[..cut]);
                    self.text_buf = comb[cut..].into();
                    break;
                };
                self.text(&mut out, &comb[..i]);
                input = comb[i + CALL_OPEN.len()..].into();
                self.in_call = true;
                continue;
            }
            let comb = std::mem::take(&mut self.call_buf) + &input;
            let Some(j) = comb.find(CALL_CLOSE) else {
                self.call_buf = comb;
                break;
            };
            let body = &comb[..j];
            match parse_call(body, &self.tools) {
                Some((name, args)) => {
                    self.saw_tool = true;
                    out.push(Piece::Call { name, args });
                }
                None if (callish(body) || declared(&self.tools, body.trim())) && !quoted(&self.sent) => {
                    self.in_call = false;
                    out.push(Piece::Error(UNREADABLE));
                    return out;
                }
                None => {
                    let t = format!("{CALL_OPEN}{body}{CALL_CLOSE}");
                    self.text(&mut out, &t);
                }
            }
            input = comb[j + CALL_CLOSE.len()..].into();
            self.in_call = false;
        }
        out
    }

    pub(super) fn flush(&mut self) -> Vec<Piece> {
        // 扣住的半个标签到了结尾也没等到下半截：原样当文本交出
        let pending = std::mem::take(&mut self.dsml_pending);
        let mut out = if pending.is_empty() { Vec::new() } else { self.feed_normalized(&pending) };
        if self.in_call {
            let body = std::mem::take(&mut self.call_buf);
            self.in_call = false;
            if (callish(&body) || declared(&self.tools, body.trim())) && !quoted(&self.sent) {
                return match parse_call(&body, &self.tools) {
                    Some((name, args)) => {
                        self.saw_tool = true;
                        vec![Piece::Call { name, args }]
                    }
                    None => vec![Piece::Error(CUT_SHORT)],
                };
            }
            self.text(&mut out, &format!("{CALL_OPEN}{body}"));
            return out;
        }
        let mut t = std::mem::take(&mut self.text_buf);
        if t.is_empty() {
            return out;
        }
        if self.after {
            t = self.left(&t, true).unwrap_or_default();
        }
        while !t.is_empty() {
            let Some(x) = (if self.tools.is_empty() { None } else { xml_at(&t) }) else {
                self.text(&mut out, &t);
                break;
            };
            match self.bare(&mut out, &t, x, true) {
                Bare::Failed => break,
                Bare::Held => {
                    let held = std::mem::take(&mut self.text_buf);
                    self.text(&mut out, &held);
                    break;
                }
                Bare::Next(next) => t = if self.after { self.left(&next, true).unwrap_or_default() } else { next },
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(tools: &[Value], chunks: &[&str]) -> Vec<Piece> {
        let mut s = Splitter::new(tools);
        let mut out: Vec<Piece> = chunks.iter().flat_map(|c| s.feed(c)).collect();
        out.extend(s.flush());
        // 相邻文本合并，便于断言
        let mut merged: Vec<Piece> = Vec::new();
        for p in out {
            if let (Some(Piece::Text(prev)), Piece::Text(t)) = (merged.last_mut(), &p) {
                prev.push_str(t);
                continue;
            }
            merged.push(p);
        }
        merged
    }

    fn tool(name: &str) -> Value {
        json!({"type":"function","function":{"name":name,"parameters":{"properties":{"path":{"type":"string"},"n":{"type":"integer"}}}}})
    }

    #[test]
    fn glm_template_calls_are_read_with_schema_types() {
        let out = run(&[tool("read")], &["<tool_call>read\n<arg_key>path</arg_key>\n<arg_value>a.rs</arg_value>\n<arg_key>n</arg_key><arg_value>3</arg_value></tool_call>"]);
        assert_eq!(out, [Piece::Call { name: "read".into(), args: r#"{"n":3,"path":"a.rs"}"#.into() }]);
        let bare = run(&[tool("read")], &["<tool_call>read</tool_call>"]);
        assert_eq!(bare, [Piece::Call { name: "read".into(), args: "{}".into() }], "只有名字也算，前提是请求里有这个工具");
        assert_eq!(run(&[tool("read")], &["<tool_call>hello</tool_call>"]).len(), 1);
    }

    #[test]
    fn deepseek_dsml_calls_split_across_chunks_are_read() {
        let out = run(&[tool("read")], &[
            "先看看 <｜DSML｜function_calls>\n<｜DSML｜invoke name=\"read\">\n<｜DSML｜parameter name=\"path\" string=\"true\">a.rs</｜DSML",
            "｜parameter>\n<｜DSML｜parameter name=\"n\" string=\"false\">3</｜DSML｜parameter>\n</｜DSML｜invoke>\n</｜DSML｜function_calls>",
        ]);
        assert_eq!(out, [Piece::Text("先看看 ".into()), Piece::Call { name: "read".into(), args: r#"{"n":3,"path":"a.rs"}"#.into() }]);
        assert_eq!(normalize_dsml("纯文本 a < b").0, "纯文本 a < b", "不是 DSML 的尖括号原样");
        assert_eq!(normalize_dsml("<|DSML|invoke name=\"x\">").0, "<invoke name=\"x\">", "半角竖线也认");
    }

    #[test]
    fn plain_text_passes_through_even_split() {
        assert_eq!(run(&[], &["hello ", "<tool", " world"]), [Piece::Text("hello <tool world".into())]);
    }

    #[test]
    fn json_tool_call_block_split_across_chunks() {
        let out = run(&[tool("read")], &["先看 <tool_", "call>{\"name\":\"read\",\"argu", "ments\":{\"path\":\"a\"}}</tool_call> 完"]);
        assert_eq!(out[0], Piece::Text("先看 ".into()));
        assert_eq!(out[1], Piece::Call { name: "read".into(), args: r#"{"path":"a"}"#.into() });
        assert_eq!(out[2], Piece::Text(" 完".into()));
    }

    #[test]
    fn qwen_and_anthropic_bare_calls_for_declared_tools() {
        let out = run(&[tool("read")], &["<function=read>\n<parameter=path>\nsrc/a.rs\n</parameter>\n<parameter=n>3</parameter>\n</function>"]);
        assert_eq!(out, [Piece::Call { name: "read".into(), args: r#"{"n":3,"path":"src/a.rs"}"#.into() }]);
        let out = run(&[tool("read")], &["x <invoke name=\"read\"><parameter name=\"path\">b</parameter></invoke></function_calls> y"]);
        assert_eq!(out, [Piece::Text("x ".into()), Piece::Call { name: "read".into(), args: r#"{"path":"b"}"#.into() }, Piece::Text("y".into())]);
        // 未声明的工具、代码块里的标签都是普通文本
        assert_eq!(run(&[tool("read")], &["<function=other></function>"]), [Piece::Text("<function=other></function>".into())]);
        assert_eq!(run(&[tool("read")], &["`<function=read>`"]), [Piece::Text("`<function=read>`".into())]);
    }

    #[test]
    fn cut_short_or_unreadable_calls_are_errors() {
        assert_eq!(run(&[tool("read")], &["<tool_call>{\"name\":\"read\""]), [Piece::Error(CUT_SHORT)]);
        assert_eq!(run(&[tool("read")], &["<tool_call>{broken</tool_call>"]), [Piece::Error(UNREADABLE)]);
        assert_eq!(run(&[tool("read")], &["<function=read><parameter=path>half"]), [Piece::Error(CUT_SHORT)]);
    }
}
