// ---------------- 轻量 Protobuf 线格式解析器 ----------------

pub(crate) fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut val = 0u64;
    let mut shift = 0u32;
    for _ in 0..10 {
        let byte = *buf.get(*pos)?;
        *pos += 1;
        val |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(val);
        }
        shift += 7;
    }
    None
}

pub(crate) enum FieldValue<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Skipped,
}

pub(crate) fn next_field<'a>(buf: &'a [u8], pos: &mut usize) -> Option<(u32, FieldValue<'a>)> {
    if *pos >= buf.len() {
        return None;
    }
    let tag = read_varint(buf, pos)?;
    let field_no = (tag >> 3) as u32;
    let wire_type = (tag & 0x07) as u8;
    let value = match wire_type {
        0 => FieldValue::Varint(read_varint(buf, pos)?),
        1 => {
            if *pos + 8 > buf.len() {
                return None;
            }
            *pos += 8;
            FieldValue::Skipped
        }
        2 => {
            let len = read_varint(buf, pos)? as usize;
            let end = pos.checked_add(len)?;
            if end > buf.len() {
                return None;
            }
            let slice = &buf[*pos..end];
            *pos = end;
            FieldValue::Bytes(slice)
        }
        5 => {
            if *pos + 4 > buf.len() {
                return None;
            }
            *pos += 4;
            FieldValue::Skipped
        }
        _ => return None,
    };
    Some((field_no, value))
}

pub(crate) fn parse_timestamp_ms(buf: &[u8]) -> Option<i64> {
    let mut pos = 0;
    let mut seconds: Option<i64> = None;
    let mut nanos: i64 = 0;
    while let Some((field, val)) = next_field(buf, &mut pos) {
        if let FieldValue::Varint(v) = val {
            match field {
                1 => seconds = Some(v as i64),
                2 => nanos = v as i64,
                _ => {}
            }
        }
    }
    let secs = seconds?;
    // 合理区间校验（2020-01-01 ~ 2100-01-01），避免把非时间戳子消息误判为时间
    if !(1_577_836_800..4_102_444_800).contains(&secs) {
        return None;
    }
    secs.checked_mul(1_000)?.checked_add(nanos / 1_000_000)
}

#[derive(Debug, Default)]
pub(crate) struct ProtoModelUsage {
    pub(crate) model_enum: Option<u64>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_read_tokens: Option<i64>,
    pub(crate) response_id: Option<String>,
}

pub(crate) fn parse_model_usage(buf: &[u8]) -> ProtoModelUsage {
    let mut out = ProtoModelUsage::default();
    let mut pos = 0;
    while let Some((field, val)) = next_field(buf, &mut pos) {
        match (field, val) {
            (1, FieldValue::Varint(v)) => out.model_enum = Some(v),
            (2, FieldValue::Varint(v)) => out.input_tokens = i64::try_from(v).ok(),
            (3, FieldValue::Varint(v)) => out.output_tokens = i64::try_from(v).ok(),
            (5, FieldValue::Varint(v)) => out.cache_read_tokens = i64::try_from(v).ok(),
            (7 | 11, FieldValue::Bytes(raw)) => {
                if out.response_id.is_none() {
                    if let Ok(s) = std::str::from_utf8(raw) {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            out.response_id = Some(trimmed.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[derive(Debug, Default)]
pub(crate) struct StepMetadata {
    pub(crate) created_at_ms: Option<i64>,
    pub(crate) first_token_at_ms: Option<i64>,
    pub(crate) completed_at_ms: Option<i64>,
    pub(crate) usage: Option<ProtoModelUsage>,
}

pub(crate) fn parse_step_metadata(buf: &[u8]) -> StepMetadata {
    let mut out = StepMetadata::default();
    let mut completed_8: Option<i64> = None;
    let mut pos = 0;
    while let Some((field, val)) = next_field(buf, &mut pos) {
        if let FieldValue::Bytes(sub) = val {
            match field {
                1 => out.created_at_ms = parse_timestamp_ms(sub),
                6 => out.first_token_at_ms = parse_timestamp_ms(sub),
                7 => out.completed_at_ms = parse_timestamp_ms(sub),
                8 => completed_8 = parse_timestamp_ms(sub),
                9 => out.usage = Some(parse_model_usage(sub)),
                _ => {}
            }
        }
    }
    if out.completed_at_ms.is_none() {
        out.completed_at_ms = completed_8;
    }
    out
}

/// 将 `file:///D:/develop/Jolu` 解码成本地目录路径 `D:\develop\Jolu`（跨平台兼容）。
pub(crate) fn decode_file_uri(uri: &str) -> Option<String> {
    let raw = uri.trim();
    let rest = raw
        .strip_prefix("file:///")
        .or_else(|| raw.strip_prefix("file://"))?;
    let bytes = percent_decode(rest);
    let mut decoded = String::from_utf8(bytes).ok()?;
    if decoded.is_empty() {
        return None;
    }
    if cfg!(windows) {
        decoded = decoded.replace('/', "\\");
    } else if !decoded.starts_with('/') {
        decoded.insert(0, '/');
    }
    Some(decoded)
}

pub(crate) fn percent_decode(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

pub(crate) fn parse_first_workspace_cwd(raw: Option<&str>) -> Option<String> {
    let text = raw?.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(text) {
        for item in items {
            if let Some(uri) = item.as_str() {
                if let Some(path) = decode_file_uri(uri) {
                    return Some(path);
                }
            }
        }
    }
    decode_file_uri(text)
}
