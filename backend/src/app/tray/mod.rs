//! 托盘图标、悬浮摘要、右键菜单与托盘动作分发。

use std::sync::atomic::Ordering;
use std::sync::Mutex;

use tauri::{
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

use crate::app::island::{
    apply_island_platform, apply_island_position, apply_island_topmost, clones as island_clones,
    create_island_clone, destroy_island, dock, reset_window_layout, toggle_island,
};
use crate::app::state::{with_database, Cfg, Db, MainIntent, ISLAND, MAIN, TRAY_REFRESHING};
use crate::app::windows::show_main_with_intent;
use crate::{collector, quota, validation};

pub(crate) mod context_menu;
pub(crate) mod icon;
pub(crate) mod summary;

pub(crate) fn refresh_tray_menu(app: &tauri::AppHandle) {
    update_tray_icon(app);
    let _ = app.emit(crate::events::MENU_STATE_CHANGED, ());
}

fn tray_scale_factor(app: &tauri::AppHandle) -> f64 {
    app.get_webview_window(MAIN)
        .or_else(|| app.get_webview_window(ISLAND))
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(1.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayQuotaTone {
    Normal,
    Warning,
    Critical,
    Unknown,
}

fn tray_quota_summary(state: &quota::QuotaState) -> (TrayQuotaTone, Option<f64>) {
    let quota::QuotaState::Ok { windows, .. } = state else {
        return (TrayQuotaTone::Unknown, None);
    };
    let worst = windows
        .iter()
        .filter_map(|window| window.used_percent)
        .filter(|value| value.is_finite())
        .fold(None::<f64>, |current, value| {
            Some(current.map_or(value, |old| old.max(value)))
        })
        .map(|value| value.clamp(0.0, 100.0));
    match worst {
        Some(value) if value >= 90.0 => (TrayQuotaTone::Critical, Some(value)),
        Some(value) if value >= 75.0 => (TrayQuotaTone::Warning, Some(value)),
        Some(value) => (TrayQuotaTone::Normal, Some(value)),
        None => (TrayQuotaTone::Unknown, None),
    }
}

fn tray_quota_color(state: &quota::QuotaState, is_auth: bool) -> [u8; 3] {
    let (tone, _) = tray_quota_summary(state);
    let tone = if is_auth { tone } else { TrayQuotaTone::Normal };
    match tone {
        TrayQuotaTone::Normal => [6, 193, 103],
        TrayQuotaTone::Warning => [217, 133, 0],
        TrayQuotaTone::Critical => [220, 38, 38],
        TrayQuotaTone::Unknown => [154, 160, 170],
    }
}

fn tray_look(app: &tauri::AppHandle) -> ([u8; 3], usize) {
    let state = summary::current_state(app);
    let is_auth = app.state::<Cfg>().0.get().island_kind == "auth";
    (
        tray_quota_color(&state, is_auth),
        icon::output_size(tray_scale_factor(app)),
    )
}

fn tray_icon_image(app: &tauri::AppHandle) -> Image<'static> {
    let (color, size) = tray_look(app);
    icon::badge(&icon::base(size), color)
}

static TRAY_ICON_SHOWN: Mutex<Option<([u8; 3], usize)>> = Mutex::new(None);

pub(crate) fn update_tray_icon(app: &tauri::AppHandle) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let look = tray_look(app);
        let Ok(mut shown) = TRAY_ICON_SHOWN.lock() else {
            return;
        };
        if *shown == Some(look) {
            return;
        }
        if tray
            .set_icon(Some(icon::badge(&icon::base(look.1), look.0)))
            .is_ok()
        {
            *shown = Some(look);
        }
    }
}

pub(crate) fn ensure_main_tray(app: &tauri::AppHandle) -> Result<(), String> {
    if app.tray_by_id("main-tray").is_some() {
        return Ok(());
    }
    TrayIconBuilder::with_id("main-tray")
        .icon(tray_icon_image(app))
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            match &event {
                TrayIconEvent::Enter { position, .. } => {
                    summary::enter(tray.app_handle(), position.x, position.y)
                }
                TrayIconEvent::Leave { .. }
                | TrayIconEvent::Click { .. }
                | TrayIconEvent::DoubleClick { .. } => summary::close(tray.app_handle()),
                _ => {}
            }
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Right,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                if let Err(error) = context_menu::open(tray.app_handle(), false, None) {
                    eprintln!("[菜单] {error}");
                }
                return;
            }
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                show_main_with_intent(
                    app,
                    MainIntent::Overview {
                        settings: app.state::<Cfg>().0.get(),
                    },
                );
            }
        })
        .build(app)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn finish_tray_refresh_after_cooldown(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));
        TRAY_REFRESHING.store(false, Ordering::Relaxed);
        refresh_tray_menu(&app);
    });
}

pub(crate) fn handle_tray_menu(
    app: &tauri::AppHandle,
    id: &str,
    island: Option<&str>,
) -> Result<(), String> {
    match id {
        "open_main" => show_main_with_intent(
            app,
            MainIntent::Overview {
                settings: app.state::<Cfg>().0.get(),
            },
        ),
        "island_refresh" => {
            if TRAY_REFRESHING.swap(true, Ordering::Relaxed) {
                return Ok(());
            }
            refresh_tray_menu(app);
            let _ = app.emit(crate::events::ISLAND_REFRESH, ());
            crate::events::refresh_requested(app, None);
            finish_tray_refresh_after_cooldown(app.clone());
        }
        "topmost" => {
            let _ = apply_island_topmost(app, !app.state::<Cfg>().0.get().always_on_top);
        }
        "pos_free" => apply_island_position(app, None, island),
        "pos_top" => apply_island_position(app, Some(dock::Edge::Top), island),
        "pos_bottom" => apply_island_position(app, Some(dock::Edge::Bottom), island),
        "pos_left" => apply_island_position(app, Some(dock::Edge::Left), island),
        "pos_right" => apply_island_position(app, Some(dock::Edge::Right), island),
        "clone_create" => create_island_clone(app, island)?,
        "clone_destroy" => destroy_island(app, island)?,
        "clone_promote" => {
            let id = island.ok_or("本体已经是主关注")?;
            app.state::<Cfg>().0.try_update(|settings| {
                island_clones::promote(settings, id);
            })?;
            crate::events::settings_changed(app);
        }
        "refresh" => {
            if TRAY_REFRESHING.swap(true, Ordering::Relaxed) {
                return Ok(());
            }
            refresh_tray_menu(app);
            if validation::isolated() {
                let _ = app.emit(crate::events::ISLAND_REFRESH, ());
                crate::events::refresh_requested(app, None);
                finish_tray_refresh_after_cooldown(app.clone());
                return Ok(());
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let result =
                    with_database(app.state::<Db>().0.clone(), |conn| Ok(collector::scan(conn)))
                        .await;
                match result {
                    Ok(r) => println!("[托盘] 立即刷新：新增 {} 条", r.records_inserted),
                    Err(e) => eprintln!("[托盘] 刷新失败: {e}"),
                }
                let _ = app.emit(crate::events::ISLAND_REFRESH, ());
                crate::events::refresh_requested(&app, None);
                finish_tray_refresh_after_cooldown(app);
            });
        }
        "toggle_island" => {
            if let Err(e) = toggle_island(app) {
                eprintln!("[托盘] 切换灵动岛失败: {e}");
            }
        }
        "dnd" => {
            if let Some(c) = app.try_state::<Cfg>() {
                c.0.update(|s| s.dnd = !s.dnd);
                crate::events::settings_changed(app);
            }
            refresh_tray_menu(app);
        }
        "reset_layout" => reset_window_layout(app),
        "about" => {
            show_main_with_intent(
                app,
                MainIntent::About {
                    version: env!("CARGO_PKG_VERSION").into(),
                },
            );
        }
        "open_source_settings" => {
            show_main_with_intent(
                app,
                MainIntent::Settings {
                    section: "island".into(),
                },
            );
        }
        "quit" => app.exit(0),
        other => {
            if let Some(p) = other.strip_prefix("plat_") {
                if let Err(error) = apply_island_platform(app, p.to_string()) {
                    eprintln!("[托盘] 平台切换失败：{error}");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quota(percentages: &[Option<f64>]) -> quota::QuotaState {
        quota::QuotaState::Ok {
            windows: percentages
                .iter()
                .enumerate()
                .map(|(index, used_percent)| quota::QuotaWindow {
                    key: format!("window-{index}"),
                    window_name: format!("窗口 {index}"),
                    used_percent: *used_percent,
                    amount_text: None,
                    remaining_text: None,
                    resets_at: None,
                })
                .collect(),
            plan: None,
            account: None,
        }
    }

    #[test]
    fn tray_uses_the_worst_available_quota_window() {
        assert_eq!(
            tray_quota_summary(&quota(&[Some(24.0), Some(78.0)])),
            (TrayQuotaTone::Warning, Some(78.0))
        );
        assert_eq!(
            tray_quota_summary(&quota(&[Some(91.0), Some(12.0)])),
            (TrayQuotaTone::Critical, Some(91.0))
        );
    }

    #[test]
    fn tray_keeps_unknown_distinct_from_zero_usage() {
        assert_eq!(
            tray_quota_summary(&quota(&[None])),
            (TrayQuotaTone::Unknown, None)
        );
        assert_eq!(
            tray_quota_summary(&quota(&[Some(0.0)])),
            (TrayQuotaTone::Normal, Some(0.0))
        );
    }

    #[test]
    fn api_key_tray_icon_stays_green_for_all_quota_tones() {
        let warning = quota(&[Some(78.0)]);
        let critical = quota(&[Some(95.0)]);
        assert_eq!(tray_quota_color(&warning, false), [6, 193, 103]);
        assert_eq!(tray_quota_color(&critical, false), [6, 193, 103]);
    }
}
