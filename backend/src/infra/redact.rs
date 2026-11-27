//! 共享敏感信息识别基元（零业务依赖，打破 `gateway::mask ↔ handoff_facts` 循环）。
//!
//! 收纳两处共用的前缀 Key 常量、卡号（Luhn）与 18 位身份证号校验规则，以及进程级豁免清单。

use std::collections::HashSet;
use std::sync::{LazyLock, RwLock};

/// 无键名的前缀 Key（前缀之后至少 16 位才遮）。
pub(crate) const PREFIX_KEYS: [&str; 26] = [
    "sk-",
    "sk_",
    "rk_",
    "gsk_",
    "npm_",
    "ya29.",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xoxa-",
    "xoxb-",
    "xoxp-",
    "xoxs-",
    "xoxr-",
    "xapp-",
    "hf_",
    "dop_v1_",
    "shpat_",
    "AKIA",
    "ASIA",
    "AIza",
    "pplx-",
];

pub(crate) const PREFIX_MIN: usize = 16;

/// 未分组卡号的最小连续位数（13–15 位常见于时间戳/单号，不碰）。
const CARD_MIN_UNGROUPED: usize = 16;

static ALLOW_LIST: LazyLock<RwLock<HashSet<String>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

/// 同步设置中的「从不脱敏」豁免清单（小写去空白后保存）。
pub(crate) fn set_allow_list(allow: HashSet<String>) {
    if let Ok(mut guard) = ALLOW_LIST.write() {
        *guard = allow;
    }
}

/// 判断给定片段是否在「从不脱敏」豁免清单中。
pub(crate) fn allowed_value(value: &str) -> bool {
    ALLOW_LIST
        .read()
        .is_ok_and(|set| !set.is_empty() && set.contains(&value.trim().to_lowercase()))
}

fn floor_boundary(text: &str, mut at: usize) -> usize {
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Luhn 校验（银行卡号）。
pub(crate) fn luhn_ok(digits: &[u8]) -> bool {
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

/// 扫描文本中通过 Luhn 校验的银行卡号字节区间 `(start, end)`。
pub(crate) fn collect_card_ranges(text: &[u8], out: &mut Vec<(usize, usize)>) {
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
        while j < text.len() && text[j].is_ascii_digit() {
            j += 1;
        }
        digits.extend_from_slice(&text[start..j]);
        while digits.len() < 19
            && matches!(text.get(j), Some(b' ' | b'-'))
            && text.get(j + 1).is_some_and(|c| c.is_ascii_digit())
        {
            let group_start = j + 1;
            let mut group_end = group_start;
            while group_end < text.len()
                && text[group_end].is_ascii_digit()
                && group_end - group_start < 6
            {
                group_end += 1;
            }
            digits.extend_from_slice(&text[group_start..group_end]);
            j = group_end;
        }
        i = j.max(i + 1);
        let grouped = j - start > digits.len();
        if digits.len() < 13 || digits.len() > 19 {
            continue;
        }
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
            out.push((start, j));
        }
    }
}

const ID_WEIGHTS: [u32; 17] = [7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2];
const ID_CHECK: &[u8; 11] = b"10X98765432";

/// 扫描文本中通过校验位验证的 18 位居民身份证号字节区间 `(start, end)`。
pub(crate) fn collect_idcard_ranges(text: &[u8], out: &mut Vec<(usize, usize)>) {
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
        if i > 0 && text[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        if text.get(i + 18).is_some_and(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let num = |range: std::ops::Range<usize>| -> u32 {
            window[range]
                .iter()
                .fold(0u32, |acc, b| acc * 10 + u32::from(b - b'0'))
        };
        let (year, month, day) = (num(6..10), num(10..12), num(12..14));
        if !(1900..=2099).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day)
        {
            continue;
        }
        let sum: u32 = window[..17]
            .iter()
            .enumerate()
            .map(|(k, b)| u32::from(*b - b'0') * ID_WEIGHTS[k])
            .sum();
        let check = ID_CHECK[(sum % 11) as usize];
        if last != check && last != check.to_ascii_lowercase() {
            continue;
        }
        out.push((i, i + 18));
    }
}

/// 供 `handoff_facts` 等非网关模块直接识别卡号与身份证号（跳过豁免清单）。
pub(crate) fn card_and_idcard_spans(text: &str) -> Vec<(usize, usize)> {
    let mut raw = Vec::new();
    collect_card_ranges(text.as_bytes(), &mut raw);
    collect_idcard_ranges(text.as_bytes(), &mut raw);
    raw.retain(|(s, e)| e > s);
    raw.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut out = Vec::with_capacity(raw.len());
    let mut cursor = 0usize;
    for (s, e) in raw {
        if s < cursor {
            continue;
        }
        cursor = e;
        let start = floor_boundary(text, s);
        let end = floor_boundary(text, e);
        if end > start && !allowed_value(&text[start..end]) {
            out.push((start, end));
        }
    }
    out
}
