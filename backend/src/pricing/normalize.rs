//! 模型 ID 规范化与内置 / models.dev 候选匹配。

use std::collections::HashSet;

use crate::gateway::models_dev::{CachedCatalog, ModelCost};
use super::table::PRICES;
use super::Price;

/// 日期快照后缀：`-2025-09-29`（11 字符）或 `-20250929`（9 字符）。
pub(super) fn is_date_snapshot_suffix(value: &str) -> bool {
    let bytes = value.as_bytes();
    match bytes.len() {
        11 => {
            bytes[0] == b'-'
                && bytes[1..].iter().enumerate().all(|(index, byte)| {
                    if matches!(index, 4 | 7) { *byte == b'-' } else { byte.is_ascii_digit() }
                })
        }
        9 => bytes[0] == b'-' && bytes[1..].iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

pub(super) fn strip_date_suffix(model: &str) -> Option<String> {
    for suffix_len in [11usize, 9usize] {
        if model.len() > suffix_len {
            let split = model.len() - suffix_len;
            if model.is_char_boundary(split) && is_date_snapshot_suffix(&model[split..]) {
                return Some(model[..split].to_string());
            }
        }
    }
    None
}

pub(super) fn strip_bedrock_prefix(model: &str) -> Option<String> {
    const PREFIXES: &[&str] = &[
        "global.", "us-gov.", "us.", "eu.", "apac.", "jp.", "au.", "ca.", "in.",
        "anthropic.", "openai.", "google.", "meta.", "deepseek.", "xai.",
    ];
    for prefix in PREFIXES {
        if let Some(rest) = model.strip_prefix(prefix) {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

pub(super) fn strip_window_marker(model: &str) -> Option<String> {
    if let Some(idx) = model.rfind('[') {
        if model.ends_with(']') && idx > 0 {
            let inside = &model[idx + 1..model.len() - 1];
            if inside.ends_with('m') || inside.ends_with('k') {
                return Some(model[..idx].trim_end_matches('-').to_string());
            }
        }
    }
    if let Some(idx) = model.rfind('@') {
        if idx > 0 {
            let tail = &model[idx + 1..];
            if tail.chars().take_while(|c| c.is_ascii_digit()).count() > 0
                && (tail.ends_with('k') || tail.ends_with('m'))
            {
                return Some(model[..idx].to_string());
            }
        }
    }
    None
}

pub(super) fn strip_colon_tag(model: &str) -> Option<String> {
    let idx = model.rfind(':')?;
    if idx == 0 {
        return None;
    }
    let tail = &model[idx + 1..];
    // 兼容 Bedrock `-v1:0`
    if let Some(base) = model[..idx].strip_suffix("-v1").or_else(|| model[..idx].strip_suffix("-v2")) {
        if tail.chars().all(|c| c.is_ascii_digit()) {
            return Some(base.to_string());
        }
    }
    Some(model[..idx].to_string())
}

pub(super) fn colon_to_dash(model: &str) -> Option<String> {
    if model.contains(':') {
        Some(model.replace(':', "-"))
    } else {
        None
    }
}

pub(super) fn strip_effort_dash_suffix(model: &str) -> Option<String> {
    const EFFORTS: &[&str] = &["-minimal", "-low", "-medium", "-high", "-xhigh", "-max"];
    for suffix in EFFORTS {
        if let Some(rest) = model.strip_suffix(suffix) {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// 生成按优先级排序的规范化模型候选名（不随意剥离未知 `-custom` 后缀，防误匹配）。
pub fn normalize_model_candidates(raw: &str) -> Vec<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(trimmed);

    while let Some(cand) = queue.pop_front() {
        let cand = cand.trim().to_string();
        if cand.is_empty() || !seen.insert(cand.clone()) {
            continue;
        }
        out.push(cand.clone());

        if let Some((_, tail)) = cand.rsplit_once('/') {
            if !tail.is_empty() {
                queue.push_back(tail.to_string());
            }
        }
        if let Some(s) = strip_bedrock_prefix(&cand) {
            queue.push_back(s);
        }
        if let Some(s) = strip_window_marker(&cand) {
            queue.push_back(s);
        }
        if let Some(s) = colon_to_dash(&cand) {
            queue.push_back(s);
        }
        if let Some(s) = strip_colon_tag(&cand) {
            queue.push_back(s);
        }
        if let Some(s) = strip_date_suffix(&cand) {
            queue.push_back(s);
        }
        if let Some(s) = strip_effort_dash_suffix(&cand) {
            queue.push_back(s);
        }
        if cand.starts_with("claude-") {
            if cand.contains('.') {
                queue.push_back(cand.replace('.', "-"));
            }
            // 将 `claude-opus-4-6` 同步尝试 `claude-opus-4.6`（兼容 models.dev 部分点号条目）
            if let Some(dotted) = claude_dash_to_dot(&cand) {
                queue.push_back(dotted);
            }
        }
    }
    out
}

pub(super) fn claude_dash_to_dot(model: &str) -> Option<String> {
    let bytes = model.as_bytes();
    for i in 1..bytes.len().saturating_sub(2) {
        if bytes[i] == b'-' && bytes[i - 1].is_ascii_digit() && bytes[i + 1].is_ascii_digit() {
            let after_end = i + 2 == bytes.len() || !bytes[i + 2].is_ascii_digit();
            let before_start = i == 1 || bytes[i - 2] == b'-';
            if before_start && after_end {
                let mut s = model.to_string();
                s.replace_range(i..=i, ".");
                return Some(s);
            }
        }
    }
    None
}

pub(super) fn is_claude_model(model: &str, platform: Option<&str>) -> bool {
    model.starts_with("claude-") || platform == Some("claude")
}

pub(super) fn is_gemini_model(model: &str, platform: Option<&str>) -> bool {
    model.starts_with("gemini-") || platform == Some("gemini")
}

pub(super) fn lookup_builtin_candidate(platform: Option<&str>, candidate: &str) -> Option<Price> {
    PRICES.iter().copied().find(|price| {
        platform.is_none_or(|value| value == price.platform)
            && (candidate == price.model
                || candidate.strip_prefix(price.model).is_some_and(is_date_snapshot_suffix))
    })
}

pub(super) fn lookup_models_dev_candidate<'a>(
    catalog: &'a CachedCatalog,
    candidate: &str,
) -> Option<(&'a str, &'a str, &'a ModelCost)> {
    if let Some((k, meta)) = catalog.models.get_key_value(candidate) {
        if let Some(cost) = meta.cost.as_ref() {
            return Some((k.as_str(), meta.vendor.as_str(), cost));
        }
    }
    for (id, meta) in &catalog.models {
        let Some(cost) = meta.cost.as_ref() else { continue };
        if id.eq_ignore_ascii_case(candidate) {
            return Some((id.as_str(), meta.vendor.as_str(), cost));
        }
        if let Some((_, tail)) = id.rsplit_once('/') {
            if tail.eq_ignore_ascii_case(candidate) {
                return Some((id.as_str(), meta.vendor.as_str(), cost));
            }
        }
    }
    None
}
