//! token 估算（移植 magpie `tokensOf` / `rawTokens` / `imageTokens`）。
//!
//! 只是估算：发出去之前不知道上游怎么分词，上游回了真实输入量后再按比例校准（`Prompt::calibrated`）。

use serde_json::Value;

/// 一张图按多少 token 算：约等于 Anthropic 对 1092×1092 图片的计数。
pub const IMAGE_TOKENS: i64 = 1600;

/// 一段文字的 token：ASCII 约 4 个字符一个，其余（多半是中日韩文字）一个字符一个。
pub fn tokens_of(s: &str) -> i64 {
    let mut ascii = 0i64;
    let mut other = 0i64;
    for &b in s.as_bytes() {
        if b < 0x80 {
            ascii += 1;
        } else if b >= 0xC0 {
            // UTF-8 多字节字符的首字节，一个字符只数一次
            other += 1;
        }
    }
    (ascii + 3) / 4 + other
}

/// 按字节数估算：工具定义、调用参数这类 JSON 原样算。
pub fn len_tokens(bytes: usize) -> i64 {
    (bytes as i64 + 3) / 4
}

/// 一段 JSON 的 token，按它紧凑序列化后的字节数算（不重新分配字符串）。
pub fn raw_tokens(v: &Value) -> i64 {
    len_tokens(json_len(v))
}

/// `v` 紧凑序列化后的字节数。
pub fn json_len(v: &Value) -> usize {
    match v {
        Value::Null => 4,
        Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::Number(n) => number_len(n),
        Value::String(s) => string_len(s),
        Value::Array(a) => 2 + a.iter().map(json_len).sum::<usize>() + a.len().saturating_sub(1),
        Value::Object(m) => 2 + m.iter().map(|(k, v)| string_len(k) + 1 + json_len(v)).sum::<usize>() + m.len().saturating_sub(1),
    }
}

fn number_len(n: &serde_json::Number) -> usize {
    if let Some(i) = n.as_i64() {
        let mut len = if i < 0 { 1 } else { 0 };
        let mut x = i.unsigned_abs();
        loop {
            len += 1;
            x /= 10;
            if x == 0 {
                return len;
            }
        }
    }
    n.to_string().len()
}

fn string_len(s: &str) -> usize {
    let mut len = 2 + s.len();
    for &b in s.as_bytes() {
        match b {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0C => len += 1,
            0..=0x1F => len += 5,
            _ => {}
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_of_counts_ascii_by_four_and_other_by_char() {
        assert_eq!(tokens_of(""), 0);
        assert_eq!(tokens_of("abcd"), 1);
        assert_eq!(tokens_of("abcde"), 2);
        assert_eq!(tokens_of("中文"), 2);
        assert_eq!(tokens_of("ab中文"), 3);
        assert_eq!(tokens_of("😀"), 1);
    }

    #[test]
    fn json_len_matches_compact_serialization() {
        let v: Value = serde_json::from_str(
            r#"{"a":[1,-20,3.5,true,false,null],"b":"x\"y\\z\n\u0001中","c":{},"d":[]}"#,
        )
        .unwrap();
        assert_eq!(json_len(&v), serde_json::to_string(&v).unwrap().len());
    }
}
