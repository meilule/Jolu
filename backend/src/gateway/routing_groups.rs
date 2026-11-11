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
use std::collections::HashMap;
use std::sync::Mutex;

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
/// 保存时允许的嵌套深度（magpie R2 同值）；超过或成环的分组直接拒存。
const MAX_SAVE_DEPTH: usize = 8;

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
    /// 临时暂停到这个时刻（毫秒）；None = 未暂停。旧数据缺这个字段按未暂停处理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_until_ms: Option<i64>,
    /// 「轮转」时的权重（每轮出现几次）；None / 0 按 1。其他策略不看它。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<u32>,
}

/// 成员失败后的休置时长：可转移错误（429 / 5xx 等）之后这个成员排到候选末尾一会儿，
/// 别让下一个请求又先撞上它（magpie 成员 rest 的近似；账号级冷却另由 429 的 Retry-After 管）。
const MEMBER_REST_MS: i64 = 60_000;

static RESTED: std::sync::LazyLock<Mutex<HashMap<String, i64>>> = std::sync::LazyLock::new(Default::default);

fn rest_key(group: &str, member: &GroupMember) -> String {
    format!("{group}\0{}\0{}", member.slug, member.model)
}

/// 记一次成员失败：`group` 是分组 slug。
pub(crate) fn rest_member(group: &str, slug: &str, model: &str, now_ms: i64) {
    let key = format!("{group}\0{slug}\0{model}");
    if let Ok(mut map) = RESTED.lock() {
        map.retain(|_, until| *until > now_ms);
        map.insert(key, now_ms + MEMBER_REST_MS);
    }
}

fn is_rested(group: &str, member: &GroupMember, now_ms: i64) -> bool {
    RESTED.lock().map(|map| map.get(&rest_key(group, member)).is_some_and(|until| *until > now_ms)).unwrap_or(false)
}

impl GroupMember {
    /// 是否在 `now` 处于暂停中（已到期视为恢复）。
    pub(crate) fn is_paused(&self, now: i64) -> bool {
        self.paused_until_ms.is_some_and(|until| until > now)
    }
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

/// 成员去重（同一「来源/模型」只留第一次出现的那个，保留它的暂停设置）。
/// 为什么：重复成员在顺序 / 轮转里会被算两次，「轮转」也就不再均匀（magpie R1）。
pub(crate) fn dedupe_members(members: &[GroupMember]) -> Vec<GroupMember> {
    let mut seen = std::collections::HashSet::new();
    members
        .iter()
        .filter(|m| seen.insert((m.slug.trim().to_string(), m.model.trim().to_string())))
        .cloned()
        .collect()
}

/// 保存前检查嵌套：引用的分组要存在，展开不能回到自己（成环），深度不超过 [`MAX_SAVE_DEPTH`]。
/// `lookup` 按 slug 取别的分组的成员（保存中的这个分组用新成员）。
pub(crate) fn check_nesting(
    slug: &str,
    members: &[GroupMember],
    lookup: &dyn Fn(&str) -> Option<Vec<GroupMember>>,
) -> Result<(), String> {
    fn visit(
        current: &str,
        members: &[GroupMember],
        path: &mut Vec<String>,
        lookup: &dyn Fn(&str) -> Option<Vec<GroupMember>>,
    ) -> Result<(), String> {
        if path.len() > MAX_SAVE_DEPTH {
            return Err(format!("分组嵌套超过 {MAX_SAVE_DEPTH} 层：{}", path.join(" → ")));
        }
        for member in members {
            let Some(child) = member.slug.strip_prefix("group/").or_else(|| (member.slug == "group").then_some(member.model.as_str())) else { continue };
            if path.iter().any(|seen| seen == child) {
                return Err(format!("分组嵌套成环：{} → {child}", path.join(" → ")));
            }
            let Some(grandchildren) = lookup(child) else {
                return Err(format!("成员引用的分组「{child}」不存在"));
            };
            path.push(child.to_string());
            visit(child, &grandchildren, path, lookup)?;
            path.pop();
        }
        let _ = current;
        Ok(())
    }
    visit(slug, members, &mut vec![slug.to_string()], lookup)
}

/// 从数据库读别的分组的成员（`check_nesting` 用）。
fn lookup_in(conn: &Connection) -> impl Fn(&str) -> Option<Vec<GroupMember>> + '_ {
    move |slug: &str| get(conn, slug).ok().flatten().map(|group| group.members)
}

pub(crate) fn create(conn: &Connection, slug: &str, name: &str, strategy: &str, members: &[GroupMember]) -> Result<RoutingGroup, String> {
    validate_members(members)?;
    let members = &dedupe_members(members)[..];
    let slug = slug.trim();
    if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err("分组 id 只能用小写字母、数字和连字符".into());
    }
    check_nesting(slug, members, &lookup_in(conn))?;
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
    let deduped = members.map(dedupe_members);
    let members = deduped.as_deref();
    if let Some(members) = members {
        validate_members(members)?;
        check_nesting(slug, members, &lookup_in(conn))?;
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
/// `now_ms` 用于判定暂停是否到期；`exhausted_of` 表示成员的额度是否已用满
/// （Cap hold：用满的成员降到候选末尾而不是剔除——全员用满时仍要给出候选，
/// 让上游按真实原因报错，而不是网关说「没有可用成员」）。
pub(crate) fn ordered_members(
    group: &RoutingGroup,
    rotate_state: &mut std::collections::HashMap<String, u64>,
    usage_of: &dyn Fn(&str) -> i64,
    resets_of: &dyn Fn(&str) -> Option<i64>,
    exhausted_of: &dyn Fn(&str) -> bool,
    now_ms: i64,
    affinity_member: Option<&str>,
) -> (Vec<GroupMember>, Option<String>) {
    let mut note: Option<String> = None;
    // 暂停过滤：到期的自动恢复（不写库，按时间判定即可）
    let paused: Vec<String> = group
        .members
        .iter()
        .filter(|m| m.is_paused(now_ms))
        .map(|m| m.slug.clone())
        .collect();
    let mut members: Vec<GroupMember> = group.members.iter().filter(|m| !m.is_paused(now_ms)).cloned().collect();
    if members.is_empty() && !group.members.is_empty() {
        // 全员暂停：不静默给出空列表，如实说明并把成员原样返回（含暂停中的），
        // 让上游状态决定结果——比网关凭空说「无成员」更可解释
        note = Some(format!("分组成员全部处于暂停中（{}），已忽略暂停继续尝试", paused.join("、")));
        members = group.members.clone();
    } else if !paused.is_empty() {
        note = Some(format!("已跳过暂停中的成员：{}", paused.join("、")));
    }
    match group.strategy.as_str() {
        "rotate" => {
            // 带权轮转：权重 w 的成员每轮出现 w 次（相邻），游标按总权重取模
            let counter = rotate_state.entry(group.slug.clone()).or_insert(0);
            let weights: Vec<u64> = members.iter().map(|m| m.weight.filter(|w| *w > 0).unwrap_or(1) as u64).collect();
            let total: u64 = weights.iter().sum::<u64>().max(1);
            let mut position = *counter % total;
            *counter += 1;
            let mut start = 0;
            for (index, weight) in weights.iter().enumerate() {
                if position < *weight {
                    start = index;
                    break;
                }
                position -= weight;
            }
            members.rotate_left(start);
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
    // 刚失败过的成员（休置中）排到末尾，不剔除
    let rested = members.iter().filter(|m| is_rested(&group.slug, m, now_ms)).count();
    if rested > 0 {
        members.sort_by_key(|m| is_rested(&group.slug, m, now_ms));
        let text = format!("{rested} 个成员刚失败过，休置中，已排到候选末尾");
        note = Some(match note {
            Some(existing) => format!("{existing}；{text}"),
            None => text,
        });
    }
    // Cap hold：额度用满的成员降到末尾（stable sort 保序），不剔除
    let exhausted_count = members.iter().filter(|m| exhausted_of(&m.slug)).count();
    if exhausted_count > 0 {
        members.sort_by_key(|m| exhausted_of(&m.slug));
        let note_text = format!("{exhausted_count} 个成员额度已用满，已排到候选末尾");
        note = Some(match note {
            Some(existing) => format!("{existing}；{note_text}"),
            None => note_text,
        });
    }
    // 会话亲和：已绑定的成员提到最前（保 prompt cache）。「顺序」策略除外——用户要的是
    // 「第一个能用就用第一个」：第一个休息时会话落到第二个，第一个恢复后要还给它，
    // 不能因为绑定一直粘在第二个上（magpie `82ca554`）。
    if let Some(bound) = affinity_member.filter(|_| group.strategy != "order") {
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
    // 嵌套展开后同一「来源/模型」只试一次
    let out = dedupe_members(&out);
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
        GroupMember { slug: slug.into(), model: model.into(), paused_until_ms: None, weight: None }
    }

    #[test]
    fn weighted_rotation_gives_each_member_its_share_of_turns() {
        let mut heavy = member("a", "m1");
        heavy.weight = Some(3);
        let group = RoutingGroup { id: "g".into(), slug: "weighted".into(), name: "带权".into(), strategy: "rotate".into(), members: vec![heavy, member("b", "m2")], created_ms: 0 };
        let mut state = std::collections::HashMap::new();
        let no_usage = |_: &str| 0;
        let firsts: Vec<String> = (0..8)
            .map(|_| ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 0, None).0[0].slug.clone())
            .collect();
        assert_eq!(firsts, ["a", "a", "a", "b", "a", "a", "a", "b"], "a 每轮 3 次、b 1 次");
    }

    #[test]
    fn a_member_that_just_failed_rests_at_the_back_then_returns() {
        let group = RoutingGroup { id: "g".into(), slug: "rest-test".into(), name: "休置".into(), strategy: "order".into(), members: vec![member("a", "m1"), member("b", "m2")], created_ms: 0 };
        let mut state = std::collections::HashMap::new();
        let no_usage = |_: &str| 0;
        rest_member("rest-test", "a", "m1", 1_000);
        let (ordered, note) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 1_500, None);
        assert_eq!(ordered[0].slug, "b");
        assert!(note.unwrap().contains("休置"));
        let (ordered, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 1_000 + MEMBER_REST_MS + 1, None);
        assert_eq!(ordered[0].slug, "a", "休置到期回到原位");
    }

    fn no_resets(_: &str) -> Option<i64> {
        None
    }

    fn no_exhausted(_: &str) -> bool {
        false
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
        let (first, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 0, None);
        assert_eq!(first[0].slug, "a");
        let (second, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 0, None);
        assert_eq!(second[0].slug, "b");
        let (third, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 0, None);
        assert_eq!(third[0].slug, "c");
        let (fourth, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 0, None);
        assert_eq!(fourth[0].slug, "a", "转满一圈回到开头");
    }

    #[test]
    fn duplicate_members_are_kept_once_and_nesting_is_checked_on_save() {
        let members = vec![member("a", "m1"), member("b", "m2"), member("a", "m1")];
        assert_eq!(dedupe_members(&members).len(), 2);
        let groups: std::collections::HashMap<&str, Vec<GroupMember>> = [
            ("inner", vec![member("a", "m1")]),
            ("loop-b", vec![member("group/loop-a", "x")]),
        ]
        .into_iter()
        .collect();
        let lookup = |slug: &str| groups.get(slug).cloned();
        assert!(check_nesting("outer", &[member("group/inner", "x")], &lookup).is_ok());
        let error = check_nesting("loop-a", &[member("group/loop-b", "x")], &lookup).unwrap_err();
        assert!(error.contains("成环") && error.contains("loop-a → loop-b"), "{error}");
        assert!(check_nesting("outer", &[member("group/ghost", "x")], &lookup).unwrap_err().contains("ghost"));
        assert!(check_nesting("self", &[member("group/self", "x")], &lookup).unwrap_err().contains("成环"));
        // 9 层链：超过深度上限
        let chain: std::collections::HashMap<String, Vec<GroupMember>> =
            (1..=9).map(|i| (format!("g{i}"), vec![member(&format!("group/g{}", i + 1), "x")])).chain([("g10".to_string(), vec![member("a", "m")])]).collect();
        let deep = |slug: &str| chain.get(slug).cloned();
        assert!(check_nesting("g0", &[member("group/g1", "x")], &deep).unwrap_err().contains("超过"));
    }

    #[test]
    fn order_gives_the_conversation_back_to_the_first_once_it_is_ready() {
        let mut group = RoutingGroup {
            id: "g".into(),
            slug: "ordered".into(),
            name: "顺序".into(),
            strategy: "order".into(),
            members: vec![member("a", "m1"), member("b", "m2")],
            created_ms: 0,
        };
        let mut state = std::collections::HashMap::new();
        let no_usage = |_: &str| 0;
        // a 用满时会话落到 b 并绑定 b
        let a_exhausted = |slug: &str| slug == "a";
        let (ordered, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, &a_exhausted, 0, Some("b"));
        assert_eq!(ordered[0].slug, "b");
        // a 恢复后：即使会话绑定在 b，也回到 a
        let (ordered, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 0, Some("b"));
        assert_eq!(ordered[0].slug, "a", "顺序策略不被绑定粘住");
        // 其他策略仍保留绑定
        group.strategy = "rotate".into();
        let (ordered, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 0, Some("b"));
        assert_eq!(ordered[0].slug, "b");
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
        let (ordered, _) = ordered_members(&group, &mut state, &usage, &no_resets, &no_exhausted, 0, None);
        assert_eq!(ordered[0].slug, "b", "用量最少的排最前");
        // 会话已绑定 b：即使 a 用量更少，亲和也要把 b 提到最前（prompt cache）
        let (ordered, _) = ordered_members(&group, &mut state, &usage, &no_resets, &no_exhausted, 0, Some("b"));
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
        let (ordered, _) = ordered_members(&group, &mut state, &no_usage, &resets, &no_exhausted, 0, None);
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
        let (ordered, _) = ordered_members(&group, &mut state, &no_usage, &resets, &no_exhausted, 0, None);
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
        let (ordered, note) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 0, None);
        assert_eq!(ordered.len(), 2, "降级为 order 仍给出全部成员");
        assert!(note.is_some_and(|n| n.contains("order")), "降级要在 trace 里说明");
        let pace = RoutingGroup { strategy: "pace".into(), ..group.clone() };
        let (ordered, note) = ordered_members(&pace, &mut state, &no_usage, &no_resets, &no_exhausted, 0, None);
        assert_eq!(ordered.len(), 2);
        assert!(note.is_some_and(|n| n.contains("order")));
    }

    #[test]
    fn paused_members_are_skipped_until_their_time() {
        let mut a = member("a", "m1");
        a.paused_until_ms = Some(5_000);
        let group = RoutingGroup {
            id: "g".into(),
            slug: "daily".into(),
            name: "日常".into(),
            strategy: "order".into(),
            members: vec![a, member("b", "m2")],
            created_ms: 0,
        };
        let mut state = std::collections::HashMap::new();
        let no_usage = |_: &str| 0i64;
        // 未到期：跳过 a
        let (ordered, note) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 1_000, None);
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0].slug, "b");
        assert!(note.clone().is_some_and(|n| n.contains("暂停")), "{note:?}");
        // 已到期：恢复
        let (ordered, _) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 9_000, None);
        assert_eq!(ordered.len(), 2);
    }

    #[test]
    fn all_members_paused_falls_back_instead_of_empty() {
        let mut a = member("a", "m1");
        a.paused_until_ms = Some(5_000);
        let group = RoutingGroup {
            id: "g".into(),
            slug: "daily".into(),
            name: "日常".into(),
            strategy: "order".into(),
            members: vec![a],
            created_ms: 0,
        };
        let mut state = std::collections::HashMap::new();
        let no_usage = |_: &str| 0i64;
        let (ordered, note) = ordered_members(&group, &mut state, &no_usage, &no_resets, &no_exhausted, 1_000, None);
        assert_eq!(ordered.len(), 1, "全员暂停时不能给空列表");
        assert!(note.clone().is_some_and(|n| n.contains("全部处于暂停")), "{note:?}");
    }

    #[test]
    fn exhausted_members_sink_to_the_end_but_stay() {
        let group = RoutingGroup {
            id: "g".into(),
            slug: "daily".into(),
            name: "日常".into(),
            strategy: "order".into(),
            members: vec![member("a", "m1"), member("b", "m2")],
            created_ms: 0,
        };
        let mut state = std::collections::HashMap::new();
        let no_usage = |_: &str| 0i64;
        // a 额度用满：降到末尾，但保留在候选里（全员用满时仍要给出候选）
        let exhausted = |slug: &str| slug == "a";
        let (ordered, note) = ordered_members(&group, &mut state, &no_usage, &no_resets, &exhausted, 0, None);
        assert_eq!(ordered.len(), 2, "不剔除");
        assert_eq!(ordered[0].slug, "b");
        assert_eq!(ordered[1].slug, "a");
        assert!(note.clone().is_some_and(|n| n.contains("额度已用满")), "{note:?}");
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
