//! 路由分组（`group/<slug>`）的命令行增删改（5.5，`jolu group add|set|rm`）。
//! 规则与校验全在 `crate::gateway::routing_groups`；网关每次请求都从库里读分组，
//! 命令行写进库就立刻生效，不需要通知桌面端（桌面端界面下次打开分组页时看到）。

use rusqlite::Connection;

use crate::gateway::routing_groups::{self, GroupMember};

/// 命令行看到的分组
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteGroupView {
    pub slug: String,
    pub name: String,
    pub strategy: String,
    /// `来源/模型`（嵌套分组为 `group/<slug>`）
    pub members: Vec<String>,
}

/// `来源/模型` → 成员；模型 id 里可以再有 `/`，只在第一个 `/` 处拆
pub(crate) fn parse_member(text: &str) -> Result<GroupMember, String> {
    let (slug, model) = text
        .trim()
        .split_once('/')
        .filter(|(slug, model)| !slug.trim().is_empty() && !model.trim().is_empty())
        .ok_or_else(|| format!("成员「{text}」要写成 来源/模型，嵌套分组写 group/<分组>"))?;
    Ok(GroupMember { slug: slug.trim().into(), model: model.trim().into(), paused_until_ms: None, weight: None, tier: None })
}

fn view(group: routing_groups::RoutingGroup) -> RouteGroupView {
    RouteGroupView {
        slug: group.slug,
        name: group.name,
        strategy: group.strategy,
        members: group.members.iter().map(|m| format!("{}/{}", m.slug, m.model)).collect(),
    }
}

pub fn list(conn: &Connection) -> Result<Vec<RouteGroupView>, String> {
    Ok(routing_groups::list(conn).map_err(|e| e.to_string())?.into_iter().map(view).collect())
}

/// 新建；策略 order / rotate / usage / smart / pace（intent 要判定模型，只在桌面端建）
pub fn add(conn: &Connection, slug: &str, strategy: &str, members: &[String]) -> Result<RouteGroupView, String> {
    if !matches!(strategy, "order" | "rotate" | "usage" | "smart" | "pace") {
        return Err(format!("策略只能是 order / rotate / usage / smart / pace（按意图请在桌面端建），收到「{strategy}」"));
    }
    let members = members.iter().map(|m| parse_member(m)).collect::<Result<Vec<_>, _>>()?;
    routing_groups::create(conn, slug, slug, strategy, &members, None).map(view)
}

/// 整体替换成员（暂停、权重、档位不保留——命令行只管顺序）
pub fn set_members(conn: &Connection, slug: &str, members: &[String]) -> Result<RouteGroupView, String> {
    if routing_groups::get(conn, slug)?.is_none() {
        return Err(format!("没有分组「{slug}」"));
    }
    let members = members.iter().map(|m| parse_member(m)).collect::<Result<Vec<_>, _>>()?;
    routing_groups::update(conn, slug, None, None, Some(&members), None)?;
    routing_groups::get(conn, slug)?.map(view).ok_or_else(|| "更新后读不到分组".into())
}

pub fn remove(conn: &Connection, slug: &str) -> Result<(), String> {
    if routing_groups::get(conn, slug)?.is_none() {
        return Err(format!("没有分组「{slug}」"));
    }
    routing_groups::delete(conn, slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let path = std::env::temp_dir().join(format!("jolu-route-groups-{}-{}.db", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        crate::db::open(&path).unwrap()
    }

    #[test]
    fn members_split_on_first_slash() {
        let m = parse_member("openrouter/anthropic/claude-sonnet").unwrap();
        assert_eq!((m.slug.as_str(), m.model.as_str()), ("openrouter", "anthropic/claude-sonnet"));
        assert!(parse_member("nomodel").is_err());
        assert!(parse_member("/x").is_err());
    }

    #[test]
    fn add_set_list_remove_round_trip() {
        let conn = conn();
        let made = add(&conn, "main", "rotate", &["kimi/k2".into(), "zcode/glm-5".into()]).unwrap();
        assert_eq!(made.members, vec!["kimi/k2", "zcode/glm-5"]);
        assert!(add(&conn, "main", "order", &["kimi/k2".into()]).unwrap_err().contains("已被占用"));
        assert!(add(&conn, "x", "intent", &["kimi/k2".into()]).is_err());
        add(&conn, "outer", "order", &["group/main".into()]).unwrap();
        assert!(set_members(&conn, "main", &["group/outer".into()]).unwrap_err().contains("成环"));
        assert_eq!(set_members(&conn, "main", &["zcode/glm-5".into()]).unwrap().members, vec!["zcode/glm-5"]);
        assert_eq!(list(&conn).unwrap().len(), 2);
        remove(&conn, "outer").unwrap();
        assert!(remove(&conn, "outer").is_err());
    }
}
