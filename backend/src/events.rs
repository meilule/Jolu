//! 后端发给前端的事件：名字集中在这里，载荷见 `docs/specs/2026-11-16-baseline-and-contracts.md` §4。
//!
//! 前端的对应表在 `frontend/src/lib/events.ts`（事件名 → 载荷类型），两边改名要一起改；
//! `names_match_frontend_table` 测试会核对两边的名字集合一致。
//!
//! 约定：
//! - 发送一律用这里的常量，不再写字符串字面量；
//! - `settings-changed` 只通过 [`settings_changed`] 发：发送时现读带修订号的快照（见
//!   `settings::Store::snapshot`），前端丢弃修订号更小的那份，多线程乱序到达也不会让旧快照盖掉新的；
//! - `refresh-requested` 只通过 [`refresh_requested`] 发：载荷是 `null`（全部刷新）或连接 id（只刷这个连接）；
//! - `gateway-catalog-changed` 在模型目录可能变化时发（[`catalog_changed`]），前端重读 `gateway_catalog`。

use tauri::{AppHandle, Emitter, Manager};

pub(crate) const SETTINGS_CHANGED: &str = "settings-changed";
pub(crate) const CONNECTIONS_CHANGED: &str = "connections-changed";
pub(crate) const REFRESH_REQUESTED: &str = "refresh-requested";
pub(crate) const COLLECTION_STATUS: &str = "collection-status";
pub(crate) const LIVE_USAGE: &str = "live-usage";
pub(crate) const GATEWAY_ROUTE_START: &str = "gateway-route-start";
pub(crate) const GATEWAY_ROUTE: &str = "gateway-route";
pub(crate) const GATEWAY_WIRING_CHANGED: &str = "gateway-wiring-changed";
pub(crate) const GATEWAY_CATALOG_CHANGED: &str = "gateway-catalog-changed";
pub(crate) const AUTOMATION_CHANGED: &str = "automation-changed";
pub(crate) const WARMUP_FINISHED: &str = "warmup-finished";
pub(crate) const DAILY_CHECKIN_CHANGED: &str = "daily-checkin-changed";
pub(crate) const LIBRARY_CHANGED: &str = "library-changed";
pub(crate) const PRIVACY_FIRST_MASK: &str = "privacy-first-mask";
pub(crate) const HANDOFF_CHAT: &str = "handoff-chat";
pub(crate) const DOCK_CHANGED: &str = "dock-changed";
pub(crate) const DOCK_HINT: &str = "dock-hint";
pub(crate) const ISLAND_POINTER: &str = "island-pointer";
pub(crate) const ISLAND_BLUR: &str = "island-blur";
pub(crate) const ISLAND_REFRESH: &str = "island-refresh";
pub(crate) const MAIN_INTENT: &str = "main-intent";
pub(crate) const APP_NOTICE: &str = "app-notice";
pub(crate) const UPDATE_DOWNLOAD_PROGRESS: &str = "update-download-progress";
pub(crate) const MENU_REOPEN: &str = "menu-reopen";
pub(crate) const MENU_STATE_CHANGED: &str = "menu-state-changed";
pub(crate) const TRAY_SUMMARY_OPEN: &str = "tray-summary-open";
pub(crate) const TRAY_SUMMARY_CLOSE: &str = "tray-summary-close";
pub(crate) const STARTUP_ERROR: &str = "startup-error";
/// 主窗口当前在看哪个连接（4.2b #9「跟随主窗口」）；载荷见 `commands::island_window::MainContext`
pub(crate) const MAIN_CONTEXT: &str = "main-context";

/// 全部事件名（测试用来与前端表核对）。
#[cfg(test)]
pub(crate) const ALL: [&str; 29] = [
    SETTINGS_CHANGED, CONNECTIONS_CHANGED, REFRESH_REQUESTED, COLLECTION_STATUS, LIVE_USAGE,
    GATEWAY_ROUTE_START, GATEWAY_ROUTE, GATEWAY_WIRING_CHANGED, GATEWAY_CATALOG_CHANGED,
    AUTOMATION_CHANGED, WARMUP_FINISHED, DAILY_CHECKIN_CHANGED, LIBRARY_CHANGED, PRIVACY_FIRST_MASK,
    HANDOFF_CHAT, DOCK_CHANGED, DOCK_HINT, ISLAND_POINTER, ISLAND_BLUR, ISLAND_REFRESH, MAIN_INTENT,
    APP_NOTICE, UPDATE_DOWNLOAD_PROGRESS, MENU_REOPEN, MENU_STATE_CHANGED, TRAY_SUMMARY_OPEN,
    TRAY_SUMMARY_CLOSE, STARTUP_ERROR, MAIN_CONTEXT,
];

/// 发 `settings-changed`：载荷是发送这一刻的设置快照（含修订号），不是调用方手里的副本。
pub(crate) fn settings_changed(app: &AppHandle) {
    let snapshot = app.state::<crate::Cfg>().0.snapshot();
    let _ = app.emit(SETTINGS_CHANGED, snapshot);
}

/// 发 `refresh-requested`：`None` 表示全部刷新（载荷 `null`），`Some(id)` 只刷这个连接。
pub(crate) fn refresh_requested(app: &AppHandle, connection_id: Option<&str>) {
    let _ = app.emit(REFRESH_REQUESTED, connection_id);
}

/// 发 `gateway-catalog-changed`：模型目录可能变了（来源增删改名、开关供网关使用、
/// 模型清单刷新、路由分组增删改），前端重读 `gateway_catalog`。
pub(crate) fn catalog_changed(app: &AppHandle) {
    let _ = app.emit(GATEWAY_CATALOG_CHANGED, ());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 前端 `lib/events.ts` 的 `EVENTS` 表与这里的名字集合一致（改名 / 增删要两边一起改）
    #[test]
    fn names_match_frontend_table() {
        let table = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/src/lib/events.ts")).expect("读前端事件表");
        let mut frontend: Vec<String> = table
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let start = line.find(": \"")? + 3;
                let rest = &line[start..];
                let end = rest.find('"')?;
                line.split(':').next().filter(|key| key.chars().all(|c| c.is_ascii_uppercase() || c == '_')).map(|_| rest[..end].to_string())
            })
            .collect();
        frontend.sort();
        let mut backend: Vec<String> = ALL.iter().map(|name| name.to_string()).collect();
        backend.sort();
        assert_eq!(backend, frontend);
    }

    #[test]
    fn names_are_unique() {
        let mut names = ALL.to_vec();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), ALL.len());
    }
}
