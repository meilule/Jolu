//! 纯设置数据结构与校验函数（零业务模块依赖，打破 `settings ↔ dock / island_clones / main_window / gateway::search` 循环）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 停靠边缘（§2.1.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Edge::Top => "top",
            Edge::Bottom => "bottom",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }
}

/// 持久化的停靠状态（§2.1.3）：显示器 + 边缘 + 沿边偏移。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DockState {
    pub edge: Option<Edge>,
    /// 沿边偏移：横边为 x，竖边为 y，单位为逻辑像素
    pub offset: f64,
    /// 显示器名称。显示器变更后据此判断能否恢复
    pub monitor: Option<String>,
}

/// 一个分身的持久化配置。字段与本体的 `island_*` 一一对应。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct IslandClone {
    pub id: String,
    pub platform: String,
    pub kind: String,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub source_id: Option<String>,
    pub dock: DockState,
    /// 自由态窗口左上角的物理像素位置；None 表示尚未记录，重建时错开本体。
    pub position: Option<(i32, i32)>,
}

/// 主面板上次的位置与尺寸。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    /// 客户区物理像素，与 WindowEvent::Resized 的尺寸一致。
    pub width: u32,
    pub height: u32,
}

impl Placement {
    pub fn valid(&self) -> bool {
        // Windows 最小化时可能把窗口临时移到 (-32000, -32000)，不能记成用户位置。
        self.x > -20_000
            && self.y > -20_000
            && self.x < 100_000
            && self.y < 100_000
            && (600..=20_000).contains(&self.width)
            && (400..=20_000).contains(&self.height)
    }
}

/// 搜索 Key 的序列化：默认掩码，[`secret_keys::reveal`] 作用域内输出原文。
pub(crate) mod secret_keys {
    use serde::{Serialize, Serializer};
    use std::cell::Cell;

    thread_local! {
        static REVEAL: Cell<bool> = const { Cell::new(false) };
    }

    /// 在当前线程、`f` 执行期间放行原文。只给落盘用；guard 保证 `f` panic 也会复位。
    pub(crate) fn reveal<T>(f: impl FnOnce() -> T) -> T {
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                REVEAL.with(|flag| flag.set(self.0));
            }
        }
        let _reset = Reset(REVEAL.with(|flag| flag.replace(true)));
        f()
    }

    pub(super) fn serialize<S: Serializer>(keys: &[String], serializer: S) -> Result<S::Ok, S::Error> {
        if REVEAL.with(Cell::get) {
            keys.serialize(serializer)
        } else {
            keys.iter().map(|key| mask(key)).collect::<Vec<_>>().serialize(serializer)
        }
    }

    /// 保留前 4 后 4，中间省略；短 Key（≤ 8 字符）全遮。
    pub(crate) fn mask(key: &str) -> String {
        let chars: Vec<char> = key.trim().chars().collect();
        if chars.len() <= 8 {
            return "••••••••".into();
        }
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}••••{tail}")
    }
}

/// 一个搜索后端的配置（来自设置文件）。Key 列表按用户排的顺序。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchApiConfig {
    /// 后端 id，见 `gateway::search::VENDORS`
    pub vendor: String,
    /// 一个或多个 Key，序列化默认输出掩码，只有 `settings::Store` 落盘时放行原文。
    #[serde(default, serialize_with = "secret_keys::serialize")]
    pub keys: Vec<String>,
    /// 自建地址（SearXNG 实例、私有 Firecrawl 等）；空 = 厂商默认
    #[serde(default)]
    pub url: Option<String>,
    /// 暂停：Key 与配置都保留，但不参与搜索
    #[serde(default)]
    pub off: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(default = "default_silent_startup")]
    pub silent_startup: bool,
    #[serde(default = "yes")]
    pub auto_check_updates: bool,
    #[serde(default = "default_cost_currency")]
    pub cost_currency: String,
    #[serde(default = "default_update_interval")]
    pub update_interval_minutes: u32,
    pub island_platform: String,
    #[serde(default = "default_kind")]
    pub island_kind: String,
    #[serde(default)]
    pub island_connection_id: Option<String>,
    #[serde(default)]
    pub island_connection_name: Option<String>,
    #[serde(default)]
    pub island_source_id: Option<String>,
    #[serde(default = "default_topmost")]
    pub always_on_top: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_opacity")]
    pub island_opacity: u8,
    #[serde(default = "default_scale")]
    pub island_scale: u16,
    #[serde(default = "default_shrink_scale")]
    pub island_shrink_scale: u16,
    #[serde(default = "default_island_style")]
    pub island_style: String,
    #[serde(default = "default_refresh")]
    pub refresh_minutes: u32,
    #[serde(default)]
    pub balance_alert_threshold: Option<f64>,
    #[serde(default = "default_currency")]
    pub balance_alert_currency: String,
    #[serde(default = "default_dock_enabled")]
    pub dock_enabled: bool,
    #[serde(default = "yes")]
    pub island_horizontal_dock_full: bool,
    #[serde(default)]
    pub retention_days: Option<u32>,
    pub dnd: bool,
    pub island_visible: bool,
    #[serde(default)]
    pub dock: DockState,
    #[serde(default)]
    pub island_clones: Vec<IslandClone>,
    #[serde(default)]
    pub island_linkage: IslandLinkage,
    #[serde(default)]
    pub main_window: Option<Placement>,
    #[serde(default)]
    pub proxy_enabled: bool,
    #[serde(default = "default_proxy_port")]
    pub proxy_port: u16,
    #[serde(default = "default_proxy_fallback")]
    pub proxy_fallback_direct: bool,
    #[serde(default)]
    pub proxy_claude_upstream: Option<String>,
    #[serde(default)]
    pub proxy_codex_upstream: Option<String>,
    #[serde(default = "default_proxy_mode")]
    pub proxy_mode: String,
    #[serde(default)]
    pub gateway_default_connection: Option<String>,
    #[serde(default = "default_gateway_circuit")]
    pub gateway_circuit_threshold: u32,
    #[serde(default = "yes")]
    pub gateway_auto_escape: bool,
    #[serde(default)]
    pub gateway_shared_lan: bool,
    #[serde(default)]
    pub gateway_allow_ips: Vec<String>,
    #[serde(default)]
    pub keep_awake: bool,
    #[serde(default)]
    pub warmup_enabled: bool,
    #[serde(default = "default_warmup_hour")]
    pub warmup_hour: u32,
    #[serde(default)]
    pub warmup_last_date: Option<String>,
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
    #[serde(default)]
    pub search_apis: Vec<SearchApiConfig>,
    #[serde(default)]
    pub instance_id: String,
    #[serde(default)]
    pub gateway_wiring: BTreeMap<String, String>,
    #[serde(default)]
    pub claude_long_context: String,
    #[serde(default = "default_default_page")]
    pub default_page: String,
    #[serde(default)]
    pub last_page: Option<String>,
    #[serde(default = "default_main_zoom")]
    pub main_zoom: u16,
    #[serde(default = "default_list_density")]
    pub list_density: String,
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default = "default_network_mode")]
    pub network_mode: String,
    #[serde(default)]
    pub network_proxy_url: Option<String>,
    #[serde(default = "yes")]
    pub notify_task_resumed: bool,
    #[serde(default = "yes")]
    pub notify_task_finished: bool,
    #[serde(default = "yes")]
    pub notify_quota_alerts: bool,
    #[serde(default)]
    pub privacy_masking: bool,
    #[serde(default)]
    pub privacy_actions: BTreeMap<String, String>,
    #[serde(default)]
    pub privacy_allow: Vec<String>,
    #[serde(default)]
    pub privacy_first_noticed: bool,
    #[serde(default)]
    pub zcode_free_first: bool,
    #[serde(default = "default_pricing_model_source")]
    pub pricing_model_source: String,
    #[serde(default = "yes")]
    pub models_dev_auto_sync: bool,
    #[serde(default = "default_fx_mode")]
    pub fx_mode: String,
    #[serde(default)]
    pub fx_fixed_usd_cny: Option<f64>,
    #[serde(default)]
    pub global_budget_enabled: bool,
    #[serde(default)]
    pub global_budget_daily_usd: Option<f64>,
    #[serde(default)]
    pub global_budget_monthly_usd: Option<f64>,
    #[serde(default = "default_global_budget_warn_percent")]
    pub global_budget_warn_percent: u8,
}

fn default_kind() -> String { "api".into() }
fn default_silent_startup() -> bool { true }
fn default_topmost() -> bool { true }
fn default_theme() -> String { "light".into() }
fn default_opacity() -> u8 { 100 }
fn default_scale() -> u16 { 100 }
fn default_shrink_scale() -> u16 { 100 }
fn default_island_style() -> String { "classic".into() }
fn default_refresh() -> u32 { 5 }
fn default_currency() -> String { "USD".into() }
fn default_dock_enabled() -> bool { true }
fn default_proxy_port() -> u16 { 12731 }
fn default_proxy_fallback() -> bool { true }
fn default_proxy_mode() -> String { "observe".into() }
fn default_update_interval() -> u32 { 30 }
fn default_cost_currency() -> String { "USD".into() }
fn default_pricing_model_source() -> String { "upstream".into() }
fn default_fx_mode() -> String { "auto".into() }
fn default_global_budget_warn_percent() -> u8 { 80 }
fn default_gateway_circuit() -> u32 { 5 }
fn default_warmup_hour() -> u32 { 9 }
fn default_default_page() -> String { "last".into() }
fn default_main_zoom() -> u16 { 100 }
fn default_list_density() -> String { "comfortable".into() }
fn default_network_mode() -> String { "system".into() }
fn yes() -> bool { true }
fn default_quota_warn_percent() -> u8 { 80 }

/// 灵动岛联动与趣味设置（R2-I-04）。旧配置文件缺字段时自动按「打扰最少」补齐默认值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IslandLinkage {
    pub strict_writing: bool,
    pub signals: bool,
    pub follow_main: bool,
    pub zcode_provider: Option<String>,
    pub auto_focus: bool,
    #[serde(default = "yes")]
    pub show_route_flow: bool,
    #[serde(default = "yes")]
    pub route_feedback: bool,
    #[serde(default = "yes")]
    pub multi_tool_stack: bool,
    #[serde(default = "default_quota_warn_percent")]
    pub quota_warn_percent: u8,
    #[serde(default = "yes")]
    pub quota_suggest_fallback: bool,
    #[serde(default = "yes")]
    pub peer_flow: bool,
    #[serde(default = "yes")]
    pub automation_bubble: bool,
    #[serde(default = "yes")]
    pub props_theater: bool,
    #[serde(default = "yes")]
    pub milestone_confetti: bool,
    #[serde(default = "yes")]
    pub poke_reaction: bool,
    #[serde(default = "yes")]
    pub daily_recap: bool,
    #[serde(default = "yes")]
    pub follow_active: bool,
    #[serde(default)]
    pub agent_hooks: bool,
}

impl Default for IslandLinkage {
    fn default() -> Self {
        Self {
            strict_writing: false,
            signals: true,
            follow_main: false,
            zcode_provider: None,
            auto_focus: false,
            show_route_flow: true,
            route_feedback: true,
            multi_tool_stack: true,
            quota_warn_percent: default_quota_warn_percent(),
            quota_suggest_fallback: true,
            peer_flow: true,
            automation_bubble: true,
            props_theater: true,
            milestone_confetti: true,
            poke_reaction: true,
            daily_recap: true,
            follow_active: true,
            agent_hooks: false,
        }
    }
}

pub fn normalize_island_linkage(mut linkage: IslandLinkage) -> IslandLinkage {
    linkage.quota_warn_percent = linkage.quota_warn_percent.clamp(60, 95);
    linkage
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            silent_startup: true,
            auto_check_updates: true,
            update_interval_minutes: default_update_interval(),
            cost_currency: default_cost_currency(),
            island_platform: "claude".into(),
            island_kind: default_kind(),
            island_connection_id: None,
            island_connection_name: None,
            island_source_id: None,
            always_on_top: true,
            theme: default_theme(),
            island_opacity: default_opacity(),
            island_scale: default_scale(),
            island_shrink_scale: default_shrink_scale(),
            island_style: default_island_style(),
            refresh_minutes: default_refresh(),
            balance_alert_threshold: None,
            balance_alert_currency: default_currency(),
            dock_enabled: true,
            island_horizontal_dock_full: true,
            retention_days: None,
            dnd: false,
            island_visible: true,
            dock: Default::default(),
            island_clones: Vec::new(),
            island_linkage: IslandLinkage::default(),
            main_window: None,
            proxy_enabled: false,
            proxy_port: default_proxy_port(),
            proxy_fallback_direct: default_proxy_fallback(),
            proxy_claude_upstream: None,
            proxy_codex_upstream: None,
            proxy_mode: default_proxy_mode(),
            gateway_default_connection: None,
            gateway_circuit_threshold: default_gateway_circuit(),
            gateway_auto_escape: true,
            gateway_shared_lan: false,
            gateway_allow_ips: Vec::new(),
            keep_awake: false,
            warmup_enabled: false,
            warmup_hour: default_warmup_hour(),
            warmup_last_date: None,
            otlp_endpoint: None,
            search_apis: Vec::new(),
            instance_id: String::new(),
            gateway_wiring: BTreeMap::new(),
            claude_long_context: String::new(),
            default_page: default_default_page(),
            last_page: None,
            main_zoom: default_main_zoom(),
            list_density: default_list_density(),
            reduce_motion: false,
            network_mode: default_network_mode(),
            network_proxy_url: None,
            notify_task_resumed: true,
            notify_quota_alerts: true,
            notify_task_finished: true,
            privacy_masking: false,
            privacy_actions: BTreeMap::new(),
            privacy_allow: Vec::new(),
            privacy_first_noticed: false,
            zcode_free_first: false,
            pricing_model_source: default_pricing_model_source(),
            models_dev_auto_sync: true,
            fx_mode: default_fx_mode(),
            fx_fixed_usd_cny: None,
            global_budget_enabled: false,
            global_budget_daily_usd: None,
            global_budget_monthly_usd: None,
            global_budget_warn_percent: default_global_budget_warn_percent(),
        }
    }
}

/// 主窗口的十个页面，与前端 `MainPage` 一致。
const MAIN_PAGES: [&str; 10] = [
    "island",
    "providers",
    "checkin",
    "usage",
    "sessions",
    "automation",
    "skills",
    "agents",
    "gateway",
    "settings",
];

pub fn validate_default_page(page: &str) -> Result<(), String> {
    if page == "last" || MAIN_PAGES.contains(&page) {
        Ok(())
    } else {
        Err(format!("未知的默认打开页面：{page}"))
    }
}

pub fn validate_last_page(page: &str) -> Result<(), String> {
    if MAIN_PAGES.contains(&page) {
        Ok(())
    } else {
        Err(format!("未知的主窗口页面：{page}"))
    }
}

pub fn validate_island_style(style: &str) -> Result<(), String> {
    if matches!(style, "classic" | "capsule") {
        Ok(())
    } else {
        Err(format!("未知的灵动岛样式：{style}"))
    }
}

pub fn validate_display_preferences(
    opacity: u8,
    scale: u16,
    shrink_scale: u16,
    refresh_minutes: u32,
) -> Result<(), String> {
    if !(60..=100).contains(&opacity)
        || !matches!(scale, 85 | 100 | 115)
        || !matches!(shrink_scale, 75 | 100 | 125)
        || !matches!(refresh_minutes, 1 | 5 | 15 | 30)
    {
        return Err(
            "透明度须为 60–100%，大小须为 85/100/115%，缩小后大小须为 75/100/125%，刷新间隔须为 1/5/15/30 分钟"
                .into(),
        );
    }
    Ok(())
}

/// 主窗口界面缩放档位（画板 88 / D-21）。
pub const MAIN_ZOOM_STEPS: [u16; 6] = [80, 90, 100, 110, 125, 150];

/// 盘上的缩放值可能被手工改坏：不在档位里就按 100% 处理，与前端 `normalizeZoom` 一致。
pub fn normalize_zoom(zoom: u16) -> u16 {
    if MAIN_ZOOM_STEPS.contains(&zoom) {
        zoom
    } else {
        100
    }
}

pub fn validate_appearance(zoom: u16, density: &str) -> Result<(), String> {
    if !MAIN_ZOOM_STEPS.contains(&zoom) {
        return Err("界面缩放须为 80/90/100/110/125/150%".into());
    }
    if !matches!(density, "comfortable" | "compact") {
        return Err("列表密度须为 comfortable 或 compact".into());
    }
    Ok(())
}
