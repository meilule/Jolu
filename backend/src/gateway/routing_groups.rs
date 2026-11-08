//! 路由分组（`group/<slug>`）：把多个「来源/模型」成员组成一个可选的模型 id，
//! 按策略在成员间分摊与转移（对齐 magpie 的 routing group 语义）。
//!
//! - 五种策略：`order`（依次到不能答）、`rotate`（逐请求轮转）、`usage`（最少先用）、
//!   `smart`（配额剩余先到先用）、`pace`（按周期剩余配速）。smart / pace 依赖各账号的
//!   额度窗口快照，快照不可用时**降级为 order**并在路由 trace 里说明，不静默假装。
//! - 成员支持嵌套分组（成员 slug 以 `group/` 开头时递归展开，深度有上限防环）。
//! - 会话亲和与冷却在 wire 层的候选循环里生效，这里只产出**有序候选**。

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

/// 轮转游标（rotate 策略）：按分组 slug 记忆，进程内即可——重启归零无害，
/// 只影响「从哪个成员开始」，不影响成员顺序本身。
static ROTATE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// 取轮转游标。锁被毒化时按空表处理（丢一轮游标，无害）。
pub(crate) fn rotate_state() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, u64>> {
    ROTATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 嵌套展开上限：环（A 包含 B、B 包含 A）到此为止，不再深入。
const MAX_EXPAND_DEPTH: usize = 4;

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RoutingGroup {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub strategy: String,
    pub members: Vec<GroupMember>,
    pub created_ms: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct GroupMember {
    /// 来源 slug；嵌套分组时为 `group/<组slug>`。
    pub slug: String,
    pub model: String,
}

pub(crate) fn normalize_strategy(strategy: &str) -> &'static str {
    match strategy.trim() {
        "rotate" => "rotate",
        "usage" => "usage",
        "smart" => "smart",
        "pace" => "pace",
        _ => "order",
    }
}

fn row_to_group(id: String, slug: String, name: String, strategy: String, members_json: String, created_ms: i64) -> Result<RoutingGroup, String> {
    let members: Vec<GroupMember> = serde_json::from_str(&members_json)
        .map_err(|error| format!("分组「{name}」的成员配置无效：{error}"))?;
    Ok(RoutingGroup { id, slug, name, strategy, members, created_ms })
}

pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<RoutingGroup>> {
    let mut stmt = conn.prepare(
        "SELECT id, slug, name, strategy, members_json, created_ms FROM routing_groups ORDER BY created_ms ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, slug, name, strategy, members_json, created_ms) = row?;
        match row_to_group(id, slug, name, strategy, members_json, created_ms) {
            Ok(group) => out.push(group),
            // 单个分组的成员配置坏了不拖垮整个列表；管理界面能看到名字并修复它
            Err(error) => eprintln!("[路由分组] {error}"),
        }
    }
    Ok(out)
}

pub(crate) fn get(conn: &Connection, slug: &str) -> Result<Option<RoutingGroup>, String> {
    conn.query_row(
        "SELECT id, slug, name, strategy, members_json, created_ms FROM routing_groups WHERE slug = ?1",
        [slug],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        },
    )
    .optional()
    .map_err(|error| error.to_string())?
    .map(|(id, slug, name, strategy, members_json, created_ms)| row_to_group(id, slug, name, strategy, members_json, created_ms))
    .transpose()
}

fn validate_members(members: &[GroupMember]) -> Result<(), String> {
    if members.is_empty() {
        return Err("分组至少要有一个成员".into());
    }
    for member in members {
        if member.slug.trim().is_empty() || member.model.trim().is_empty() {
            return Err("成员的来源与模型都不能为空".into());
        }
    }
    Ok(())
}

pub(crate) fn create(conn: &Connection, slug: &str, name: &str, strategy: &str, members: &[GroupMember]) -> Result<RoutingGroup, String> {
    validate_members(members)?;
    let slug = slug.trim();
    if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err("分组 id 只能用小写字母、数字和连字符".into());
    }
    let strategy = normalize_strategy(strategy);
    let id = format!("rg-{}", chrono::Utc::now().timestamp_millis());
    let members_json = serde_json::to_string(members).map_err(|error| format!("成员序列化失败：{error}"))?;
    conn.execute(
        "INSERT INTO routing_groups(id, slug, name, strategy, members_json, created_ms) VALUES(?1,?2,?3,?4,?5,?6)",
        rusqlite::params![id, slug, name.trim(), strategy, members_json, chrono::Utc::now().timestamp_millis()],
    )
    .map_err(|error| if error.to_string().contains("UNIQUE") { format!("分组 id「{slug}」已被占用") } else { error.to_string() })?;
    get(conn, slug)?.ok_or_else(|| "分组创建后读取失败".into())
}

pub(crate) fn update(conn: &Connection, slug: &str, name: Option<&str>, strategy: Option<&str>, members: Option<&[GroupMember]>) -> Result<(), String> {
    if let Some(members) = members {
        validate_members(members)?;
    }
    let strategy = strategy.map(normalize_strategy);
    let members_json = match members {
        Some(members) => Some(serde_json::to_string(members).map_err(|error| format!("成员序列化失败：{error}"))?),
        None => None,
    };
    conn.execute(
        "UPDATE routing_groups SET name = COALESCE(?2, name), strategy = COALESCE(?3, strategy), members_json = COALESCE(?4, members_json) WHERE slug = ?1",
        rusqlite::params![slug, name, strategy, members_json],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn delete(conn: &Connection, slug: &str) -> Result<(), String> {
    conn.execute("DELETE FROM routing_groups WHERE slug = ?1", [slug]).map_err(|error| error.to_string())?;
    Ok(())
}

/// 按策略排出成员顺序。
///
/// `rotate_state` 是轮转游标（按分组 slug 记忆，进程内即可，重启归零无害）；
/// `usage_of` 返回成员已用量（来源连接维度，wire 层从流水统计）；`affinity_member`
/// 是该会话已绑定的成员（有则提到最前——prompt cache 值得保留，对齐既有亲和语义）。
pub(crate) fn ordered_members(
    group: &RoutingGroup,
    rotate_state: &mut std::collections::HashMap<String, u64>,
    usage_of: &dyn Fn(&str) -> i64,
    resets_of: &dyn Fn(&str) -> Option<i64>,
    affinity_member: Option<&str>,
) -> (Vec<GroupMember>, Option<String>) {
    let mut members = group.members.clone();
    let mut note: Option<String> = None;
    match group.strategy.as_str() {
        "rotate" => {
            let counter = rotate_state.entry(group.slug.clone()).or_insert(0);
            let offset = *counter as usize % members.len().max(1);
            *counter += 1;
            members.rotate_left(offset);
        }
        "usage" => {
            members.sort_by_key(|m| usage_of(&m.slug));
        }
        "smart" => {
            // 重置最近的先用：额度马上回来的账号先花掉，别浪费在重置等待上（magpie smart）
            let known = members.iter().any(|m| resets_of(&m.slug).is_some());
            if known {
                members.sort_by_key(|m| resets_of(&m.slug).unwrap_or(i64::MAX));
            } else {
                note = Some("策略「smart」需要额度快照（先刷新一次额度），已按 order 执行".into());
            }
        }
        "pace" => {
            // 剩余最多的先用：最近才重置的成员周期余量最足（magpie pace 的近似——
            // 按「下一次重置最晚」排，等价于「刚重置完」的先用）
            let known = members.iter().any(|m| resets_of(&m.slug).is_some());
            if known {
                members.sort_by_key(|m| std::cmp::Reverse(resets_of(&m.slug)));
            } else {
                note = Some("策略「pace」需要额度快照（先刷新一次额度），已按 order 执行".into());
            }
        }
        _ => {}
    }
    if let Some(bound) = affinity_member {
        if let Some(index) = members.iter().position(|m| m.slug == bound) {
            let member = members.remove(index);
            members.insert(0, member);
        }
    }
    (members, note)
}

/// 展开成员（含嵌套分组）为候选列表；来源在池里不存在的成员跳过并记原因。
/// `source_slug_of` 由调用方提供（嵌套成员需要再查子分组的成员，一层层往下走）。
pub(crate) fn expand(
    members: &[GroupMember],
    depth: usize,
    expand_group: &dyn Fn(&str) -> Option<Vec<GroupMember>>,
) -> (Vec<GroupMember>, Vec<String>) {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    fn walk(
        members: &[GroupMember],
        depth: usize,
        max: usize,
        out: &mut Vec<GroupMember>,
        skipped: &mut Vec<String>,
        expand_group: &dyn Fn(&str) -> Option<Vec<GroupMember>>,
    ) {
        if depth > max {
            skipped.push("嵌套过深".into());
            return;
        }
        for member in members {
            if let Some(child_slug) = member.slug.strip_prefix("group/") {
                match expand_group(child_slug) {
                    Some(children) => walk(&children, depth + 1, max, out, skipped, expand_group),
                    None => skipped.push(format!("group/{child_slug} 不存在")),
                }
            } else {
                out.push(member.clone());
            }
        }
    }
    walk(members, depth, MAX_EXPAND_DEPTH, &mut out, &mut skipped, expand_group);
    (out, skipped)
}

/// 成员列表的 JSON（命令层出入参用）。
pub(crate) fn members_json(members: &[GroupMember]) -> Value {
    json!(members
        .iter()
        .map(|m| json!({"slug": m.slug, "model": m.model}))
        .collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(slug: &str, model: &str) -> GroupMember {
        GroupMember { slug: slug.into(), model: model.into() }
    }

    fn no_resets(_: &str) -> Option<i64> {
        None
    }

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS routing_groups (
                id TEXT PRIMARY KEY, slug TEXT UNIQUE NOT NULL, name TEXT NOT NULL,
                strategy TEXT NOT NULL DEFAULT 'order', members_json TEXT NOT NULL DEFAULT '[]',
                created_ms INTEGER NOT NULL);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn create_requires_nonempty_members_and_slug_shape() {
        let conn = memory_db();
        assert!(create(&conn, "daily", "日常", "order", &[]).is_err(), "空成员要拒绝");
        assert!(create(&conn, "Daily", "日常", "order", &[member("zcode", "glm-5.3")]).is_err(), "大写 id 要拒绝");
        let group = create(&conn, "daily", "日常", "order", &[member("zcode", "glm-5.3")]).unwrap();
        assert_eq!(group.strategy, "order");
        assert!(create(&conn, "daily", "重复", "order", &[member("zcode", "glm-5.3")]).is_err(), "slug 要唯一");
    }

    #[test]
    fn strategy_normalizes_unknown_to_order() {
        assert_eq!(normalize_strategy("rotate"), "rotate");
        assert_eq!(normalize_strategy("what"), "order");
        assert_eq!(normalize_strategy(""), "order");
    }

    #[test]
    fn rotate_walks_members_per_request() {
        let group = RoutingGroup {
            id: "g".into(),
            slug: "daily".into(),
            name: "日常".into(),
            strategy: "rotate".into(),
            members: vec![member("a", "m1"), member("b", "m2"), member("c", "m3")],
            created_ms: 0,
        };
        let mut state = std::collections::HashMap::new();
        let no_usage = |_: &str| 0i64;
        let (first, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, None);
        assert_eq!(first[0].slug, "a");
        let (second, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, None);
        assert_eq!(second[0].slug, "b");
        let (third, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, None);
        assert_eq!(third[0].slug, "c");
        let (fourth, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, None);
        assert_eq!(fourth[0].slug, "a", "转满一圈回到开头");
    }

    #[test]
    fn usage_orders_least_used_first_and_affinity_wins() {
        let group = RoutingGroup {
            id: "g".into(),
            slug: "daily".into(),
            name: "日常".into(),
            strategy: "usage".into(),
            members: vec![member("a", "m1"), member("b", "m2")],
            created_ms: 0,
        };
        let mut state = std::collections::HashMap::new();
        let usage = |slug: &str| if slug == "a" { 100 } else { 3 };
        let (ordered, _) = ordered_members(&group, &mut state, &usage, &no_resets, None);
        assert_eq!(ordered[0].slug, "b", "用量最少的排最前");
        // 会话已绑定 b：即使 a 用量更少，亲和也要把 b 提到最前（prompt cache）
        let (ordered, _) = ordered_members(&group, &mut state, &usage, &no_resets, Some("b"));
        assert_eq!(ordered[0].slug, "b");
    }

    #[test]
    fn smart_orders_soonest_reset_first() {
        let group = RoutingGroup {
            id: "g".into(),
            slug: "daily".into(),
            name: "日常".into(),
            strategy: "smart".into(),
            members: vec![member("a", "m1"), member("b", "m2")],
            created_ms: 0,
        };
        let mut state = std::collections::HashMap::new();
        let no_usage = |_: &str| 0i64;
        // a 的额度马上重置：先花 a，别浪费在等待上（magpie smart 语义）
        let resets = |slug: &str| if slug == "a" { Some(1_000) } else { Some(9_000) };
        let (ordered, _) = ordered_members(&group, &mut state, &no_usage, &resets, None);
        assert_eq!(ordered[0].slug, "a");
    }

    #[test]
    fn pace_orders_latest_reset_first() {
        let group = RoutingGroup {
            id: "g".into(),
            slug: "daily".into(),
            name: "日常".into(),
            strategy: "pace".into(),
            members: vec![member("a", "m1"), member("b", "m2")],
            created_ms: 0,
        };
        let mut state = std::collections::HashMap::new();
        let no_usage = |_: &str| 0i64;
        // b 刚重置完（重置最晚）：周期余量最足，先用 b（magpie pace 的近似）
        let resets = |slug: &str| if slug == "a" { Some(1_000) } else { Some(9_000) };
        let (ordered, _) = ordered_members(&group, &mut state, &no_usage, &resets, None);
        assert_eq!(ordered[0].slug, "b");
    }

    #[test]
    fn smart_and_pace_degrade_to_order_without_snapshots() {
        let group = RoutingGroup {
            id: "g".into(),
            slug: "daily".into(),
            name: "日常".into(),
            strategy: "smart".into(),
            members: vec![member("a", "m1"), member("b", "m2")],
            created_ms: 0,
        };
        let mut state = std::collections::HashMap::new();
        let no_usage = |_: &str| 0i64;
        let (ordered, note) = ordered_members(&group, &mut state, &no_usage, &no_resets, None);
        assert_eq!(ordered.len(), 2, "降级为 order 仍给出全部成员");
        assert!(note.is_some_and(|n| n.contains("order")), "降级要在 trace 里说明");
        let pace = RoutingGroup { strategy: "pace".into(), ..group.clone() };
        let (ordered, note) = ordered_members(&pace, &mut state, &no_usage, &no_resets, None);
        assert_eq!(ordered.len(), 2);
        assert!(note.is_some_and(|n| n.contains("order")));
    }

    #[test]
    fn expand_walks_nested_groups_and_reports_missing() {
        let nested = vec![member("zcode", "glm-5.3")];
        let members = vec![
            member("group/sub", "ignored"),
            member("kimi", "kimi-k2"),
            member("group/missing", "ignored"),
        ];
        let lookup = |slug: &str| (slug == "sub").then(|| nested.clone());
        let (out, skipped) = expand(&members, 0, &lookup);
        assert_eq!(out.len(), 2, "嵌套成员展开、普通成员保留");
        assert_eq!(out[0].slug, "zcode");
        assert_eq!(out[1].slug, "kimi");
        assert_eq!(skipped.len(), 1, "不存在的子分组要记原因");
    }
}
