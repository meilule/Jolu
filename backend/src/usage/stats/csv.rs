use chrono::{Local, TimeZone};
use rusqlite::Connection;
use super::*;

pub fn source_label(source: &str) -> &'static str {
    if source.starts_with("gateway") {
        "模型路由"
    } else if source.starts_with("native:") {
        "原生采集"
    } else {
        "本机日志"
    }
}

pub fn billing_mode_label(mode: crate::pricing::BillingMode) -> &'static str {
    match mode {
        crate::pricing::BillingMode::Api => "API 实付",
        crate::pricing::BillingMode::Subscription => "订阅等价",
        crate::pricing::BillingMode::Free => "免费 / $0",
    }
}

pub fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// 按当前筛选导出全部调用。不含 API Key 和对话正文；未知值留空，费用列标明估算与币种。
pub fn calls_csv(conn: &Connection, f: &StatsFilter) -> rusqlite::Result<(String, i64)> {
    let reqs = load(conn, f, true, true)?;
    let mut out = String::from("\u{feff}时间,供应商,模型,普通输入,缓存读取,缓存写入,输出,总 Token,估算费用（USD）,计费口径,计价模型,首字（毫秒）,总耗时（毫秒）,思考强度,状态,会话 ID,调用 ID,数据来源\r\n");
    let opt = |v: Option<i64>| v.map(|v| v.to_string()).unwrap_or_default();
    for r in &reqs {
        let time = Local.timestamp_millis_opt(r.ts).single().map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
        let status = match (r.cancelled, r.failed, r.unknown, r.status_code) {
            (true, _, _, _) => "已取消".to_string(),
            (false, true, _, Some(code)) => format!("失败 {code}"),
            (false, true, _, None) => "失败".to_string(),
            (false, false, true, _) => "状态未知".to_string(),
            (false, false, false, _) => "成功".to_string(),
        };
        let pricing_model = r.price.as_ref().map(|p| p.pricing_model.clone()).unwrap_or_default();
        let cells = [
            time,
            crate::platforms::get(&r.platform).map_or(r.platform.clone(), |p| p.name.clone()),
            r.model.clone().unwrap_or_default(),
            opt(r.fresh_input),
            opt(r.cache_read),
            opt(r.cache_write),
            opt(r.output),
            opt(r.total),
            r.cost.map(|c| format!("{c:.6}")).unwrap_or_default(),
            billing_mode_label(r.billing_mode).to_string(),
            pricing_model,
            opt(r.first_token_ms),
            opt(r.duration_ms),
            r.effort.clone().unwrap_or_default(),
            status,
            r.session_id.clone().unwrap_or_default(),
            r.call_id.clone(),
            source_label(&r.source).to_string(),
        ];
        out.push_str(&cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Ok((out, reqs.len() as i64))
}
