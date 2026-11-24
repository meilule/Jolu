//! 胶囊灵动岛的全局指针采样、轮廓命中检测与自适应鼠标穿透（island-rendering-tech.md §3）。
//!
//! 舞台窗口固定为最大形态尺寸、默认 `set_ignore_cursor_events(true)`；
//! 本线程用 `GetCursorPos` 直接读桌面物理坐标（不经事件循环）：
//! - **命中检测**：前端上报岛当前轮廓矩形（含四角圆角 `[tl, tr, br, bl]`）与强制交互态
//!   （如下拉菜单打开时）；鼠标进入轮廓即关闭穿透，离开立即恢复穿透，且离开 ≥ 200ms 时
//!   再兜底确认一次穿透，避免卡死挡住桌面点击。
//! - **两档自适应轮询**：鼠标落在任一岛轮廓外扩 40px 范围内（或正在交互 / 拖动）时 60Hz（16ms），
//!   远离时降到 4Hz（250ms），避免空转耗电。
//! - **眼睛视线跟随**：非免打扰、非拖动且鼠标移动 ≥ 0.5 物理像素时，以约 30Hz 广播 `island-pointer`。
//!
//! 只发坐标与命中判定，不记录、不落盘。

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use serde::Deserialize;
use tauri::{Emitter, Manager};

pub(crate) const EVENT: &str = crate::events::ISLAND_POINTER;
/// 鼠标在岛附近（轮廓外扩 40px）或交互中：60Hz
const NEAR_INTERVAL: Duration = Duration::from_millis(16);
/// 鼠标远离所有岛：4Hz
const FAR_INTERVAL: Duration = Duration::from_millis(250);
/// 眼睛跟随事件节流（约 30Hz）
const GAZE_MIN_INTERVAL: Duration = Duration::from_millis(32);
/// 非胶囊样式或灵动岛隐藏时的休眠间隔
const IDLE_INTERVAL: Duration = Duration::from_millis(500);
/// 设置复查周期
const SETTINGS_RECHECK: Duration = Duration::from_millis(300);
/// 鼠标离开轮廓 200ms 后再兜底复核一次穿透，防止穿透状态卡住（§7）
const FALLBACK_PASSTHROUGH_MS: Duration = Duration::from_millis(200);
/// 60Hz 高频唤醒的外扩边距（逻辑像素，§3.4）
const NEAR_MARGIN_LOGICAL: f64 = 40.0;

#[derive(Clone, Copy, serde::Serialize)]
struct Pointer { x: f64, y: f64 }

/// 窗口内逻辑坐标下的圆角矩形命中区。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct HitRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// `[top-left, top-right, bottom-right, bottom-left]` 逻辑像素圆角半径
    #[serde(default)]
    pub radii: [f64; 4],
}

#[derive(Debug, Clone, Default)]
struct WindowHitRegion {
    rects: Vec<HitRect>,
    /// 菜单/下拉等浮层展开时，强制关闭穿透，避免点不到弹层
    force_interactive: bool,
}

static HIT_REGIONS: LazyLock<Mutex<HashMap<String, WindowHitRegion>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn set_hit_region(label: &str, rects: Vec<HitRect>, force_interactive: bool) {
    let valid = rects
        .into_iter()
        .filter(|r| {
            r.x.is_finite()
                && r.y.is_finite()
                && r.width.is_finite()
                && r.height.is_finite()
                && r.width > 0.0
                && r.height > 0.0
        })
        .collect();
    if let Ok(mut map) = HIT_REGIONS.lock() {
        map.insert(label.to_string(), WindowHitRegion { rects: valid, force_interactive });
    }
}

pub(crate) fn clear_hit_region(label: &str) {
    if let Ok(mut map) = HIT_REGIONS.lock() {
        map.remove(label);
    }
}

/// 取岛的主轮廓矩形（首条命中区）供拖动贴边判定使用。
pub(crate) fn primary_contour(label: &str) -> Option<(f64, f64, f64, f64)> {
    let map = HIT_REGIONS.lock().ok()?;
    let first = map.get(label)?.rects.first()?;
    Some((first.x, first.y, first.width, first.height))
}

/// 精确判断点 `(px, py)` 是否落在含独立四角圆角的矩形内（支持 0px 贴边直角）。
pub(crate) fn point_in_rounded_rect(px: f64, py: f64, rect: &HitRect) -> bool {
    if px < rect.x || px > rect.x + rect.width || py < rect.y || py > rect.y + rect.height {
        return false;
    }
    let lx = px - rect.x;
    let ly = py - rect.y;
    let half = (rect.width.min(rect.height) / 2.0).max(0.0);
    let [tl, tr, br, bl] = rect.radii.map(|r| if r.is_finite() { r.clamp(0.0, half) } else { 0.0 });

    if tl > 0.0 && lx < tl && ly < tl {
        let dx = tl - lx;
        let dy = tl - ly;
        return dx * dx + dy * dy <= tl * tl;
    }
    if tr > 0.0 && lx > rect.width - tr && ly < tr {
        let dx = lx - (rect.width - tr);
        let dy = tr - ly;
        return dx * dx + dy * dy <= tr * tr;
    }
    if br > 0.0 && lx > rect.width - br && ly > rect.height - br {
        let dx = lx - (rect.width - br);
        let dy = ly - (rect.height - br);
        return dx * dx + dy * dy <= br * br;
    }
    if bl > 0.0 && lx < bl && ly > rect.height - bl {
        let dx = bl - lx;
        let dy = ly - (rect.height - bl);
        return dx * dx + dy * dy <= bl * bl;
    }
    true
}

fn point_in_region(px: f64, py: f64, region: &WindowHitRegion) -> bool {
    region.rects.iter().any(|r| point_in_rounded_rect(px, py, r))
}

fn point_near_region(px: f64, py: f64, region: &WindowHitRegion, margin: f64) -> bool {
    region.rects.iter().any(|r| {
        px >= r.x - margin
            && px <= r.x + r.width + margin
            && py >= r.y - margin
            && py <= r.y + r.height + margin
    })
}

/// 两次采样的位置差小于半个物理像素时不发事件。
fn moved(previous: Option<(f64, f64)>, next: (f64, f64)) -> bool {
    previous.map_or(true, |(x, y)| (x - next.0).abs() >= 0.5 || (y - next.1).abs() >= 0.5)
}

fn should_sample_gaze(settings: &crate::settings::Settings) -> bool {
    settings.island_style == "capsule" && settings.island_visible && !settings.dnd
}

fn should_hit_test(settings: &crate::settings::Settings) -> bool {
    settings.island_style == "capsule" && settings.island_visible
}

#[cfg(target_os = "windows")]
fn cursor(_app: &tauri::AppHandle) -> Option<(f64, f64)> {
    #[repr(C)]
    struct Point { x: i32, y: i32 }
    #[link(name = "user32")]
    extern "system" { fn GetCursorPos(point: *mut Point) -> i32; }
    let mut point = Point { x: 0, y: 0 };
    // 锁屏、安全桌面等情况下会失败，此时不发事件
    (unsafe { GetCursorPos(&mut point) } != 0).then(|| (f64::from(point.x), f64::from(point.y)))
}

#[cfg(not(target_os = "windows"))]
fn cursor(app: &tauri::AppHandle) -> Option<(f64, f64)> {
    app.cursor_position().ok().map(|p| (p.x, p.y))
}

struct WindowPassthroughState {
    ignoring: bool,
    outside_since: Option<Instant>,
    fallback_reasserted: bool,
}

fn apply_ignore_cursor(app: &tauri::AppHandle, label: String, ignore: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = handle.get_webview_window(&label) {
            let _ = win.set_ignore_cursor_events(ignore);
        }
    });
}

pub(crate) fn start(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("island-pointer".into())
        .spawn(move || {
            let mut last_gaze: Option<(f64, f64)> = None;
            let mut last_gaze_at: Option<Instant> = None;
            let mut hit_active = false;
            let mut gaze_active = false;
            let mut checked: Option<Instant> = None;
            let mut states: HashMap<String, WindowPassthroughState> = HashMap::new();

            loop {
                if checked.map_or(true, |at| at.elapsed() >= SETTINGS_RECHECK) {
                    if let Some(cfg) = app.try_state::<crate::Cfg>() {
                        let s = cfg.0.get();
                        hit_active = should_hit_test(&s);
                        gaze_active = should_sample_gaze(&s);
                    } else {
                        hit_active = false;
                        gaze_active = false;
                    }
                    checked = Some(Instant::now());
                }

                if !hit_active {
                    last_gaze = None;
                    states.clear();
                    std::thread::sleep(IDLE_INTERVAL);
                    continue;
                }

                let dragging = crate::dock::DRAGGING.load(Ordering::Relaxed);
                let mut any_near = dragging;
                let now = Instant::now();

                if let Some(pos) = cursor(&app) {
                    // 1. 眼睛跟随推送（约 30Hz）
                    if gaze_active
                        && !dragging
                        && moved(last_gaze, pos)
                        && last_gaze_at.map_or(true, |at| at.elapsed() >= GAZE_MIN_INTERVAL)
                    {
                        last_gaze = Some(pos);
                        last_gaze_at = Some(now);
                        let _ = app.emit(EVENT, Pointer { x: pos.0, y: pos.1 });
                    }

                    // 2. 逐个灵动岛窗口做轮廓命中检测与穿透切换
                    let snapshot: HashMap<String, WindowHitRegion> = HIT_REGIONS
                        .lock()
                        .map(|m| m.clone())
                        .unwrap_or_default();
                    let windows = crate::commands::island_window::island_windows(&app);
                    let mut alive_labels = Vec::with_capacity(windows.len());

                    for win in windows {
                        let label = win.label().to_string();
                        alive_labels.push(label.clone());
                        let (Ok(origin), Ok(scale)) = (win.outer_position(), win.scale_factor()) else { continue };
                        if scale <= 0.0 { continue; }
                        let lx = (pos.0 - f64::from(origin.x)) / scale;
                        let ly = (pos.1 - f64::from(origin.y)) / scale;

                        let region = snapshot.get(&label);
                        let inside = dragging
                            || region.is_some_and(|r| r.force_interactive || point_in_region(lx, ly, r));
                        let near = inside
                            || region.is_some_and(|r| point_near_region(lx, ly, r, NEAR_MARGIN_LOGICAL));
                        if near { any_near = true; }

                        let want_ignore = !inside;
                        let entry = states.entry(label.clone()).or_insert(WindowPassthroughState {
                            ignoring: true,
                            outside_since: None,
                            fallback_reasserted: false,
                        });

                        if entry.ignoring != want_ignore {
                            entry.ignoring = want_ignore;
                            entry.outside_since = if want_ignore { Some(now) } else { None };
                            entry.fallback_reasserted = false;
                            apply_ignore_cursor(&app, label, want_ignore);
                        } else if want_ignore {
                            // 离开轮廓 ≥ 200ms 兜底再发一次穿透，防止极端竞态下穿透状态卡住
                            if let Some(since) = entry.outside_since {
                                if !entry.fallback_reasserted && now.duration_since(since) >= FALLBACK_PASSTHROUGH_MS {
                                    entry.fallback_reasserted = true;
                                    apply_ignore_cursor(&app, label, true);
                                }
                            }
                        }
                    }

                    states.retain(|label, _| alive_labels.contains(label));
                }

                std::thread::sleep(if any_near { NEAR_INTERVAL } else { FAR_INTERVAL });
            }
        })
        .map(|_| ())
        .unwrap_or_else(|error| eprintln!("[灵动岛] 指针与命中线程启动失败：{error}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_emits_when_position_changes() {
        assert!(moved(None, (10.0, 10.0)));
        assert!(!moved(Some((10.0, 10.0)), (10.2, 10.3)));
        assert!(moved(Some((10.0, 10.0)), (10.6, 10.0)));
        assert!(moved(Some((10.0, 10.0)), (10.0, 9.0)));
    }

    #[test]
    fn gaze_and_hit_test_policies() {
        let mut settings = crate::settings::Settings::default();
        assert!(!should_sample_gaze(&settings), "默认经典样式不采样");
        assert!(!should_hit_test(&settings), "经典样式不启舞台命中");
        settings.island_style = "capsule".into();
        assert!(should_sample_gaze(&settings));
        assert!(should_hit_test(&settings));
        settings.dnd = true;
        assert!(!should_sample_gaze(&settings), "免打扰暂停眼睛采样");
        assert!(should_hit_test(&settings), "免打扰下仍保持穿透命中，用户能点击交互");
        settings.dnd = false;
        settings.island_visible = false;
        assert!(!should_sample_gaze(&settings));
        assert!(!should_hit_test(&settings));
    }

    #[test]
    fn rounded_rect_hit_test_respects_all_four_independent_radii() {
        // 顶部贴边胶囊：上两角 0px 直角，下两角 20px 圆角
        let docked_top = HitRect { x: 100.0, y: 0.0, width: 200.0, height: 40.0, radii: [0.0, 0.0, 20.0, 20.0] };
        // 上左直角：紧贴 (100.5, 0.5) 应命中
        assert!(point_in_rounded_rect(100.5, 0.5, &docked_top));
        // 上右直角：紧贴 (299.5, 0.5) 应命中
        assert!(point_in_rounded_rect(299.5, 0.5, &docked_top));
        // 下左 20px 圆角外侧角尖 (101, 39) 不应命中
        assert!(!point_in_rounded_rect(101.0, 39.0, &docked_top));
        // 下右 20px 圆角外侧角尖 (299, 39) 不应命中
        assert!(!point_in_rounded_rect(299.0, 39.0, &docked_top));
        // 中心命中
        assert!(point_in_rounded_rect(200.0, 20.0, &docked_top));
        // 矩形外不命中
        assert!(!point_in_rounded_rect(99.0, 20.0, &docked_top));
        assert!(!point_in_rounded_rect(200.0, 41.0, &docked_top));
    }

    #[test]
    fn near_margin_expands_hit_region_by_40px() {
        let region = WindowHitRegion {
            rects: vec![HitRect { x: 100.0, y: 20.0, width: 120.0, height: 40.0, radii: [20.0; 4] }],
            force_interactive: false,
        };
        assert!(point_near_region(60.0, 20.0, &region, NEAR_MARGIN_LOGICAL));
        assert!(!point_near_region(59.0, 20.0, &region, NEAR_MARGIN_LOGICAL));
        assert!(point_near_region(160.0, 100.0, &region, NEAR_MARGIN_LOGICAL));
        assert!(!point_near_region(160.0, 101.0, &region, NEAR_MARGIN_LOGICAL));
    }
}
