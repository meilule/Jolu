//! ZCode 的「礼包领取」（不是每日签到）。
//!
//! 与 WorkBuddy / Qoder / Trae 的三点不同：
//!   1. **不是每天固定奖励**：ZCode 按活动发放一批礼包（如 300,000,000 tokens 的
//!      GLM-5.3-Flash 额度），挂着等用户点「领取」；领完就没了，下一批由服务端决定。
//!   2. **要先查再领**：`GET billing/preview` 列出当前可领的 plan 列表，每个 plan 带
//!      entitlements（额度、单位、周期）。空列表 = 当前没有可领礼包（不是失败）。
//!   3. **单位是 tokens 而不是积分**：`Outcome.unit` 从响应的 `unit_type` / `meter`
//!      取，取不到写「额度」，不硬编码成「积分」。
//!
//! 接口来自 ZCode 客户端（`D:\develop\zcode\resources\app.asar` 静态分析，2026-11-19）：
//!
//! ```text
//! GET  /api/v1/zcode-plan/billing/preview?app_version=<ver>&platform=win32-x64
//!      Authorization: Bearer <本机会话 JWT>
//!   → data.plans[] = [{ plan_id, name, description, priority,
//!                       entitlements: [{ entitlement_id, show_name, meter,
//!                                        unit_type, grant_units, period, ... }] }]
//!
//! POST /api/v1/zcode-plan/billing/claim
//!      Authorization: Bearer <JWT>  Content-Type: application/json
//!      X-ZCode-App-Version: <ver>   X-Platform: win32-x64
//!      { "plan_id": "..." }
//!   → 成功 data.plan { user_plan_id, plan_id, status, starts_at, ends_at, entitlements }
//!   → 失败 code != 0 + data.message + 可选 data.plan.ends_at（下次可领时间）
//! ```
//!
//! 领取策略（鼠鼠 2026-11-19 确认）：**全部可领的都领**，逐个报结果；只发一次，
//! 不自动重试；服务端拒绝时不猜原因，message 原样透传。

use crate::checkin_types::{Outcome, State};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

/// 礼包活动（banner）的来源。**不是** `billing/preview`——那个端点对我们的账号只回
/// `{code:0}` 没有 data（2026-11-19 实测），真正带 `plan_id` 的是营销 banner 接口
/// （ZCode 客户端 app.asar 的 `marketing/touch` GET，`claim_zcode_plan` 按钮的 args 里）。
const MARKETING: &str = "https://zcode.z.ai/api/v1/marketing/touch";
const MAX_BYTES: u64 = 1024 * 1024;

/// 没有可领礼包时的说明。**不能只说「没有」**：用户可能刚在 ZCode 客户端领过
/// （Jolu 只负责提示，实际领取发生在客户端，Jolu 看不到那次动作），
/// 所以把「可能已领」这条也讲明白，避免用户以为「明明领了却说没有」。
const INACTIVE_MESSAGE: &str = "当前没有可领取的礼包；如果你已在 ZCode 客户端领取，属正常";

pub(crate) struct Account {
    jwt: String,
    /// 账号身份：JWT 派生的哈希（`zcode_quota::account_identity` 同款口径），
    /// 换账号时能被识别为不同账户，不落邮箱或令牌原文。
    identity: String,
}

impl Account {
    /// 本机 ZCode 会话（`monitor:native:zcode:main` 这条连接）。
    /// edition 目前只有 `main`，保留参数以便将来区分国际 / 国内站。
    pub(crate) fn from_native(_edition: &str) -> Result<Self, Outcome> {
        if crate::validation::isolated() {
            return Err(Outcome::new(
                State::Unauthorized,
                "隔离验收不读取真实登录或请求官方签到接口",
            ));
        }
        let jwt = crate::gateway::adapter::zcode_start::jwt_at().ok_or_else(|| {
            Outcome::new(State::Unauthorized, "ZCode 会话凭据缺失：请先在 ZCode 中重新登录")
        })?;
        let identity = crate::zcode_quota::account_identity().map_err(|_| {
            Outcome::new(State::Unauthorized, "无法确认 ZCode 当前登录的账号，请刷新后重试")
        })?;
        Ok(Self { jwt, identity })
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    /// 状态：先看有没有可领的新礼包，再看已领取的是否还在生效。
    ///
    /// 服务端的两份数据（2026-11-20 实测）：
    ///   - `marketing/touch` 的 banner：**可领的**活动（点「领取」前的提示）；
    ///   - `billing/balance` 的 `plans`：**已领取且生效中**的套餐（`status: "active"`），
    ///     带 `plan_id` / `name` / `entitlements` / `ends_at`。
    /// 领取后 banner 消失、plans 里多一条 —— 所以「领了」是可查的服务端事实，
    /// 不用靠本地记录推断（那是方案 A 的兜底，见 daily_checkin 的 annotate_zcode_claimed）。
    pub(crate) fn status(&self) -> Outcome {
        let plans = match self.preview() {
            Ok(plans) => plans,
            Err(outcome) => return outcome,
        };
        if plans.is_empty() {
            // 没有可领的：查已领取的礼包是否仍在生效
            return match self.active_plan() {
                Ok(Some(claimed)) => claimed,
                Ok(None) => Outcome::new(State::Inactive, INACTIVE_MESSAGE).with_unit("tokens"),
                // 余额接口读不到时不能断言「没有」：如实报查询失败
                Err(outcome) => outcome,
            };
        }
        {
            {
                let total: f64 = plans.iter().map(|plan| plan.grant_total()).sum();
                let mut outcome = Outcome::new(
                    State::Ready,
                    format!(
                        "{} 个礼包可领取，共 {}",
                        plans.len(),
                        format_units(total, plans[0].unit.as_str())
                    ),
                );
                outcome.reward = Some(total);
                outcome.unit = plans[0].unit.clone();
                outcome.period_key = Some(plans_key(&plans));
                outcome
            }
        }
    }

    /// 查 `billing/balance` 里已领取且生效中的礼包套餐。
    /// 有 → `Claimed`（带额度和到期时间）；没有 → `Ok(None)` 交给调用方判 `Inactive`。
    fn active_plan(&self) -> Result<Option<Outcome>, Outcome> {
        let client = self.client()?;
        let response = client
            .get(crate::zcode_quota::balance_url())
            .bearer_auth(&self.jwt)
            .header("Accept", "application/json")
            .header("X-Device-Mid", crate::zcode_quota::device_mid())
            .header("X-Client-Language", "zh-CN")
            .header("X-ZCode-App-Version", crate::zcode_quota::app_version())
            .header("X-Platform", platform_tag())
            .send()
            .map_err(|_| Outcome::new(State::Failed, "无法连接 ZCode 余额接口，请检查网络"))?;
        let status = response.status().as_u16();
        let body = read_body(response, "礼包余额")?;
        if !(200..300).contains(&status) {
            return Err(http_error(status));
        }
        let value: Value = serde_json::from_slice(&body)
            .map_err(|_| Outcome::new(State::Failed, "ZCode 余额响应无法识别"))?;
        Ok(parse_active_plan(&value))
    }

    /// 领取：**Jolu 不代领**，只如实告诉用户去 ZCode 客户端领。
    ///
    /// 为什么：ZCode 的领取接口要求阿里云人机验证（`code 3007 captcha verify failed`，
    /// 2026-11-19 实测）。验证 token 由阿里云验证码 SDK 在浏览器环境里生成，纯后端拿不到；
    /// 绕过它等于规避对方的风控，不做。所以 Jolu 的能力边界是「查得到、领不了」：
    /// 卡片如实显示待领额度，并给一个打开 ZCode 的入口。
    ///
    /// 状态用 `Unsupported`（不是 `Failed`）：这不是失败，是**该操作不在 Jolu 的职责内**，
    /// 与「查不到」「没有」区分开，避免用户以为重试能解决。
    pub(crate) fn claim(&self, before_post: impl FnOnce() -> Result<(), Outcome>) -> Outcome {
        let plans = match self.preview() {
            Ok(plans) if plans.is_empty() => {
                return Outcome::new(State::Inactive, INACTIVE_MESSAGE).with_unit("tokens")
            }
            Ok(plans) => plans,
            Err(outcome) => return outcome,
        };
        // 连接状态复查（与其它三家一致：连接被移除时不继续）
        if let Err(outcome) = before_post() {
            return outcome;
        }
        let total: f64 = plans.iter().map(Plan::grant_total).sum();
        let unit = plans[0].unit.clone();
        let names = plans
            .iter()
            .map(|plan| plan.name.as_str())
            .collect::<Vec<_>>()
            .join("、");
        let mut outcome = Outcome::new(
            State::Unsupported,
            format!(
                "有 {} 待领取（{names}）：领取需要人机验证，请在 ZCode 客户端完成",
                format_units(total, &unit)
            ),
        );
        outcome.reward = Some(total);
        outcome.unit = unit;
        outcome.period_key = Some(plans_key(&plans));
        outcome
    }

    fn preview(&self) -> Result<Vec<Plan>, Outcome> {
        let url = format!("{MARKETING}?seq=1&locale=zh-CN");
        let client = self.client()?;
        let response = client
            .get(&url)
            .bearer_auth(&self.jwt)
            .header("Accept", "application/json")
            // 三个头缺一个就 400（2026-11-19 实测）：X-Device-Mid 是客户端每次请求现生成的
            // v4 UUID（`zcode_quota::device_mid`），X-Client-Language 与版本号同样必需
            .header("X-Device-Mid", crate::zcode_quota::device_mid())
            .header("X-Client-Language", "zh-CN")
            .header("X-ZCode-App-Version", crate::zcode_quota::app_version())
            .header("X-Platform", platform_tag())
            .send()
            .map_err(|_| Outcome::new(State::Failed, "无法连接 ZCode 礼包接口，请检查网络"))?;
        let status = response.status().as_u16();
        let body = read_body(response, "礼包活动")?;
        if !(200..300).contains(&status) {
            return Err(http_error(status));
        }
        let value: Value = serde_json::from_slice(&body)
            .map_err(|_| Outcome::new(State::Failed, "ZCode 礼包活动响应无法识别"))?;
        if let Some(code) = value.get("code").and_then(Value::as_i64) {
            if code != 0 {
                let message = value
                    .get("msg")
                    .or_else(|| value.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("ZCode 拒绝查询礼包活动")
                    .to_string();
                return Err(business_error(code, message));
            }
        }
        Ok(parse_marketing_plans(&value))
    }


    fn client(&self) -> Result<reqwest::blocking::Client, Outcome> {
        crate::net::blocking_builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| Outcome::new(State::Failed, "无法创建 ZCode 礼包客户端"))
    }
}

/// 一个可领的礼包（`preview` 的一项）。
#[derive(Debug, Clone)]
struct Plan {
    id: String,
    name: String,
    /// 该礼包全部 entitlement 的额度之和
    grant: f64,
    /// 单位：`unit_type` 优先，其次 `meter`，都没有写「额度」
    unit: String,
}

impl Plan {
    fn grant_total(&self) -> f64 {
        self.grant
    }
}


/// 从 `billing/balance` 的 `data.plans` 里找已领取且生效中的礼包套餐。
///
/// 2026-11-20 实测的响应形状（领取 WEEKEND BUILD 之后）：
/// ```json
/// {"data":{"plans":[{"user_plan_id":"upl_…","plan_id":"zcode-v3-start-plan-wk-1120",
///   "name":"ZCODE WEEKEND BUILD","status":"active","starts_at":…,"ends_at":…,
///   "entitlements":[{"show_name":"GLM-5.3-Flash","meter":"model_usage",
///     "unit_type":"token","grant_units":500000000,"period":"one_time"}]}],
///   "balances":[]}}
/// ```
/// 只认 `status == "active"` 且未过期的：其它状态（expired / cancelled）如实不算已领取。
/// `balances` 为空是正常的——礼包额度记在 `plans`，`balances` 是另一类（订阅余额桶）。
fn parse_active_plan(value: &Value) -> Option<Outcome> {
    let now = value
        .get("data")
        .and_then(|d| d.get("server_time"))
        .and_then(Value::as_i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let plans = value.get("data")?.get("plans")?.as_array()?;
    let plan = plans.iter().find(|plan| {
        let active = plan.get("status").and_then(Value::as_str) == Some("active");
        let not_expired = plan
            .get("ends_at")
            .and_then(Value::as_i64)
            .is_none_or(|ends_at| ends_at > now);
        active && not_expired
    })?;
    let name = plan.get("name").and_then(Value::as_str).map(str::trim).filter(|name| !name.is_empty()).unwrap_or("礼包");
    let plan_id = plan.get("plan_id").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let entitlements = plan.get("entitlements").and_then(Value::as_array);
    let grant: f64 = entitlements
        .into_iter()
        .flatten()
        .filter(|e| e.get("meter").and_then(Value::as_str) == Some("model_usage"))
        .filter_map(|e| e.get("grant_units").and_then(Value::as_f64))
        .fold(0.0, f64::max);
    let unit = entitlements
        .into_iter()
        .flatten()
        .find_map(|e| e.get("unit_type").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty()).map(str::to_string))
        .unwrap_or_else(|| "tokens".into());
    // 到期时间写进文案：礼包是限时的，用户要知道还能用多久
    let expiry = plan
        .get("ends_at")
        .and_then(Value::as_i64)
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|at| at.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string());
    let detail = if grant > 0.0 { format!("：{}", format_units(grant, &unit)) } else { String::new() };
    let message = match expiry {
        Some(at) => format!("已领取 {name}{detail}，{at} 前有效"),
        None => format!("已领取 {name}{detail}"),
    };
    let mut outcome = Outcome::new(State::Claimed, message);
    outcome.reward = (grant > 0.0).then_some(grant);
    outcome.unit = unit;
    outcome.period_key = (!plan_id.is_empty()).then(|| format!("zcode:{plan_id}"));
    outcome.reset_at = plan.get("ends_at").and_then(Value::as_i64);
    Some(outcome)
}

/// 从营销 banner 响应里取可领礼包。结构（2026-11-19 实测）：
/// `data.deliveries[] → banner.buttons[] → action.type == "claim_zcode_plan"
///   → action.args.plan_id`，额度在同层 `banner.background.args.zcode_plan`。
fn parse_marketing_plans(value: &Value) -> Vec<Plan> {
    let Some(deliveries) = value
        .get("data")
        .and_then(|d| d.get("deliveries"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for delivery in deliveries {
        if delivery.get("resource_position").and_then(Value::as_str) != Some("banner") {
            continue;
        }
        let Some(banner) = delivery.get("banner") else { continue };
        // plan_id 来自按钮动作；plan 的展示字段在同层 background.args.zcode_plan
        let plan_id = banner
            .get("buttons")
            .and_then(Value::as_array)
            .and_then(|buttons| {
                buttons.iter().find_map(|button| {
                    let action = button.get("action")?;
                    if action.get("type").and_then(Value::as_str) != Some("claim_zcode_plan") {
                        return None;
                    }
                    action
                        .get("args")
                        .and_then(|args| args.get("plan_id"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string)
                })
            });
        let Some(plan_id) = plan_id else { continue };
        let meta = banner
            .get("background")
            .and_then(|bg| bg.get("args"))
            .and_then(|args| args.get("zcode_plan"));
        let name = meta
            .and_then(|plan| plan.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&plan_id)
            .to_string();
        let entitlements = meta.and_then(|plan| plan.get("entitlements")).and_then(Value::as_array);
        // 只算 model_usage 的额度（客户端同款过滤），否则 banner 里的其它条目会混进总数
        let grant: f64 = entitlements
            .into_iter()
            .flatten()
            .filter(|e| e.get("meter").and_then(Value::as_str) == Some("model_usage"))
            .filter_map(|e| e.get("grant_units").and_then(Value::as_f64))
            .fold(0.0, f64::max);
        let unit = entitlements
            .into_iter()
            .flatten()
            .filter(|e| e.get("meter").and_then(Value::as_str) == Some("model_usage"))
            .find_map(|e| {
                e.get("unit_type")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "tokens".into());
        out.push(Plan { id: plan_id, name, grant, unit });
    }
    out
}


/// 同一批礼包的稳定标识：plan id 排序后哈希，用于签到记录去重（同批不重复记）。
fn plans_key(plans: &[Plan]) -> String {
    use sha2::{Digest, Sha256};
    let mut ids: Vec<&str> = plans.iter().map(|plan| plan.id.as_str()).collect();
    ids.sort_unstable();
    let mut hash = Sha256::new();
    for id in ids {
        hash.update((id.len() as u64).to_be_bytes());
        hash.update(id.as_bytes());
    }
    format!("zcode:{:x}", hash.finalize())[..20].to_string()
}

fn platform_tag() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// 与 ZCode 客户端一致的错误码分类。
fn business_error(code: i64, message: String) -> Outcome {
    let message = if message.is_empty() { "ZCode 拒绝请求".to_string() } else { message };
    let state = match code {
        401 => State::Unauthorized,
        403 => State::Forbidden,
        429 => State::RateLimited,
        _ => State::Failed,
    };
    Outcome::new(state, message)
}

fn http_error(status: u16) -> Outcome {
    let state = match status {
        401 => State::Unauthorized,
        403 => State::Forbidden,
        429 => State::RateLimited,
        _ => State::Failed,
    };
    Outcome::new(state, format!("ZCode 接口返回 HTTP {status}"))
}

fn read_body(response: reqwest::blocking::Response, what: &str) -> Result<Vec<u8>, Outcome> {
    let mut bytes = Vec::new();
    response.take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(|_| {
        Outcome::new(State::Failed, format!("读取 ZCode {what}失败"))
    })?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Outcome::new(State::Failed, format!("ZCode {what}响应超过大小限制")));
    }
    Ok(bytes)
}


fn format_units(value: f64, unit: &str) -> String {
    let number = if value.fract() == 0.0 {
        format!("{:.0}", value)
    } else {
        format!("{value:.2}")
    };
    // 千分位
    let grouped = number
        .split('.')
        .enumerate()
        .map(|(index, part)| {
            if index == 0 {
                let digits: Vec<char> = part.chars().collect();
                let mut out = String::new();
                for (at, ch) in digits.iter().enumerate() {
                    if at > 0 && (digits.len() - at) % 3 == 0 {
                        out.push(',');
                    }
                    out.push(*ch);
                }
                out
            } else {
                format!(".{part}")
            }
        })
        .collect::<String>();
    format!("{grouped} {unit}")
}

impl Outcome {
    /// 小工具：设置单位后返回自身（`Outcome::new(...).with_unit("tokens")`）。
    fn with_unit(mut self, unit: &str) -> Self {
        self.unit = unit.to_string();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 本机实测的 marketing 响应缩影（2026-11-19，字段照抄、值脱敏）。
    fn marketing_body() -> Value {
        json!({
            "code": 0,
            "msg": "",
            "data": {
                "server_time": 1_795_090_460,
                "language": "zh-CN",
                "deliveries": [{
                    "campaign_id": "2101141259813175296",
                    "resource_position": "banner",
                    "priority": 50,
                    "banner": {
                        "buttons": [
                            { "text": { "format": "", "content": "" }, "action": { "type": "close", "args": null } },
                            { "text": { "format": "", "content": "" },
                              "action": { "type": "claim_zcode_plan",
                                          "args": { "plan_id": "zcode-v3-start-plan-1119" } } }
                        ],
                        "background": {
                            "type": "bundle",
                            "args": {
                                "zcode_plan": {
                                    "plan_id": "zcode-v3-start-plan-1119",
                                    "name": "ZCODE GLOBAL BUILD",
                                    "priority": 111,
                                    "starts_at": 1_795_086_000,
                                    "ends_at": 1_795_136_400,
                                    "entitlements": [
                                        { "entitlement_id": "e1", "show_name": "GLM-5.3-Flash 额度",
                                          "meter": "model_usage", "unit_type": "token",
                                          "grant_units": 300_000_000.0, "period": "one_time" }
                                    ]
                                }
                            }
                        }
                    }
                }]
            }
        })
    }

    /// 领取后 balance 里出现的套餐（2026-11-20 真实响应缩影）。
    fn balance_after_claim() -> Value {
        json!({
            "code": 0,
            "data": {
                "server_time": 1_795_182_232i64,
                "plans": [{
                    "user_plan_id": "upl_2123773540963475456",
                    "plan_id": "zcode-v3-start-plan-wk-1120",
                    "name": "ZCODE WEEKEND BUILD",
                    "description": "",
                    "priority": 101,
                    "status": "active",
                    "starts_at": 1_795_182_044i64,
                    "ends_at": 1_795_395_600i64,
                    "entitlements": [{
                        "entitlement_id": "ent-zcode-v3-start-plan-wk-1120-1",
                        "show_name": "GLM-5.3-Flash",
                        "meter": "model_usage",
                        "unit_type": "token",
                        "capabilities": ["model:glm-5.3-flash"],
                        "grant_units": 500_000_000.0,
                        "period": "one_time",
                        "priority": 101
                    }]
                }],
                "balances": []
            }
        })
    }

    #[test]
    fn claimed_plan_from_balance_is_read_as_claimed() {
        // 领完 banner 消失，但 balance.plans 里有生效中的套餐：应显示「已领取」而不是「没有」
        let outcome = parse_active_plan(&balance_after_claim()).expect("应识别为已领取");
        assert_eq!(outcome.state, State::Claimed);
        assert!(outcome.message.contains("ZCODE WEEKEND BUILD"), "{}", outcome.message);
        assert_eq!(outcome.reward, Some(500_000_000.0));
        assert_eq!(outcome.unit, "token");
        assert!(outcome.message.contains("前有效"), "限时礼包要带到期时间：{}", outcome.message);
        assert_eq!(outcome.period_key.as_deref(), Some("zcode:zcode-v3-start-plan-wk-1120"));
    }

    #[test]
    fn inactive_plan_or_no_plan_is_not_claimed() {
        // 没有 plans：未领取
        assert!(parse_active_plan(&json!({ "code": 0, "data": { "plans": [], "balances": [] } })).is_none());
        // 状态不是 active：不算已领取
        let mut value = balance_after_claim();
        value["data"]["plans"][0]["status"] = json!("expired");
        assert!(parse_active_plan(&value).is_none());
        // 已过期（ends_at 早于 server_time）：不算生效中
        let mut value = balance_after_claim();
        value["data"]["plans"][0]["ends_at"] = json!(1_795_182_000i64);
        assert!(parse_active_plan(&value).is_none());
    }

    #[test]
    fn parses_banner_plan_id_from_claim_button() {
        let plans = parse_marketing_plans(&marketing_body());
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].id, "zcode-v3-start-plan-1119", "plan_id 来自 claim_zcode_plan 按钮的 args");
        assert_eq!(plans[0].name, "ZCODE GLOBAL BUILD");
        assert_eq!(plans[0].grant, 300_000_000.0);
        assert_eq!(plans[0].unit, "token");
    }

    #[test]
    fn ignores_deliveries_without_claim_action() {
        // 只有 close 按钮的 banner 不算可领
        let value = json!({ "code": 0, "data": { "deliveries": [{
            "resource_position": "banner",
            "banner": { "buttons": [{ "action": { "type": "close" } }] }
        }] } });
        assert!(parse_marketing_plans(&value).is_empty());
        // 非 banner 位置的不算
        let value = json!({ "code": 0, "data": { "deliveries": [{
            "resource_position": "toast",
            "banner": { "buttons": [{ "action": { "type": "claim_zcode_plan", "args": { "plan_id": "x" } } }] }
        }] } });
        assert!(parse_marketing_plans(&value).is_empty());
        // 没有 data / deliveries 不报错
        assert!(parse_marketing_plans(&json!({ "code": 0 })).is_empty());
    }

    #[test]
    fn only_model_usage_entitlements_count_toward_the_total() {
        let mut value = marketing_body();
        // 追加一条非 model_usage 的 entitlement，不应计入
        value["data"]["deliveries"][0]["banner"]["background"]["args"]["zcode_plan"]["entitlements"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "entitlement_id": "e2", "meter": "request", "unit_type": "次", "grant_units": 999.0 }));
        let plans = parse_marketing_plans(&value);
        assert_eq!(plans[0].grant, 300_000_000.0, "只算 model_usage 的额度（客户端同款过滤）");
        assert_eq!(plans[0].unit, "token");
    }

    #[test]
    fn plans_key_is_stable_regardless_of_order() {
        let a = Plan { id: "a".into(), name: "A".into(), grant: 1.0, unit: "token".into() };
        let b = Plan { id: "b".into(), name: "B".into(), grant: 2.0, unit: "token".into() };
        assert_eq!(plans_key(&[a.clone(), b.clone()]), plans_key(&[b, a]));
    }

    #[test]
    fn formats_units_with_grouping() {
        assert_eq!(format_units(300_000_000.0, "token"), "300,000,000 token");
        assert_eq!(format_units(1234.5, "token"), "1,234.50 token");
        assert_eq!(format_units(0.0, "token"), "0 token");
    }

    #[test]
    fn http_status_maps_to_states() {
        assert_eq!(http_error(401).state, State::Unauthorized);
        assert_eq!(http_error(403).state, State::Forbidden);
        assert_eq!(http_error(429).state, State::RateLimited);
        assert_eq!(http_error(500).state, State::Failed);
    }
}
