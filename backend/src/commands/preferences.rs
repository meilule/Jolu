//! 设置、数据目录、清理与开机自启。

#[allow(unused_imports)]
use crate::*;

pub(crate) fn data_paths(app: &tauri::AppHandle) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let directory = match validation::data_dir() {
        Some(path) => path,
        None => app.path().app_data_dir().map_err(|error| error.to_string())?,
    };
    Ok((directory.clone(), directory.join("usage.db")))
}

#[tauri::command]
pub(crate) fn data_info(app: tauri::AppHandle) -> Result<DataInfo, String> {
    let (_, database) = data_paths(&app)?;
    let size_bytes = [database.clone(), database.with_extension("db-wal"), database.with_extension("db-shm")]
        .into_iter()
        .filter_map(|path| std::fs::metadata(path).ok().map(|metadata| metadata.len()))
        .sum();
    Ok(DataInfo {
        path: database.to_string_lossy().into_owned(),
        size_bytes,
    })
}

#[tauri::command]
pub(crate) fn open_data_directory(app: tauri::AppHandle) -> Result<(), String> {
    let (directory, _) = data_paths(&app)?;
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    app.opener()
        .open_path(directory.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| error.to_string())
}

pub(crate) const AUTOSTART_VALUE_NAME: &str = "Jolu";

pub(crate) const AUTOSTART_REGISTRY_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(target_os = "windows")]
pub(crate) fn windows_autostart_status() -> Result<bool, String> {
    let status = windows_command("reg.exe")
        .args(["QUERY", AUTOSTART_REGISTRY_KEY, "/v", AUTOSTART_VALUE_NAME])
        .status()
        .map_err(|error| format!("无法读取开机启动状态：{error}"))?;
    Ok(status.success())
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_command(program: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    let mut command = std::process::Command::new(program);
    command.creation_flags(0x0800_0000);
    // 注册项不存在是“未启用”的正常状态；不要让 reg.exe 的本地代码页错误文本污染日志。
    command.stdout(Stdio::null()).stderr(Stdio::null());
    command
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn windows_autostart_status() -> Result<bool, String> {
    Err("当前系统暂不支持开机启动配置".into())
}

#[tauri::command]
pub(crate) fn autostart_status() -> Result<bool, String> {
    windows_autostart_status()
}

#[tauri::command]
pub(crate) fn set_autostart(on: bool) -> Result<bool, String> {
    if validation::isolated() { return Err("隔离验收不修改开机启动".into()); }
    #[cfg(target_os = "windows")]
    {
        let status = if on {
            let executable = std::env::current_exe()
                .map_err(|error| format!("无法获取程序路径：{error}"))?;
            let command = startup::command(&executable);
            windows_command("reg.exe")
                .args([
                    "ADD",
                    AUTOSTART_REGISTRY_KEY,
                    "/v",
                    AUTOSTART_VALUE_NAME,
                    "/t",
                    "REG_SZ",
                    "/d",
                    &command,
                    "/f",
                ])
                .status()
        } else {
            windows_command("reg.exe")
                .args(["DELETE", AUTOSTART_REGISTRY_KEY, "/v", AUTOSTART_VALUE_NAME, "/f"])
                .status()
        }
        .map_err(|error| format!("无法更新开机启动：{error}"))?;
        if !status.success() && (on || windows_autostart_status()?) {
            return Err("系统拒绝更新开机启动注册项".into());
        }
        return windows_autostart_status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = on;
        Err("当前系统暂不支持开机启动配置".into())
    }
}

/// 每日 warm-up 设置（批次 G §3.12）：开关与触发小时。
/// 最近一次 warm-up 的结果（成功 / 跳过 / 失败及原因、完成时间）；本次启动后还没跑过为 None。
/// 设置页挂载时读一次，之后跟 `warmup-finished` 事件更新。
#[tauri::command]
pub(crate) fn warmup_last_outcome() -> Option<crate::warmup_run::LastWarmup> {
    crate::warmup_run::last()
}

/// 打开时清掉上次日期，让今天就能按新设置跑一次。
#[tauri::command]
pub(crate) fn set_warmup(app: tauri::AppHandle, enabled: bool, hour: u32) -> Result<settings::Settings, String> {
    let hour = hour.min(23);
    let next = app.state::<Cfg>().0.try_update(|settings| {
        let turning_on = enabled && !settings.warmup_enabled;
        settings.warmup_enabled = enabled;
        settings.warmup_hour = hour;
        if turning_on { settings.warmup_last_date = None; }
    })?;
    crate::events::settings_changed(&app);
    Ok(next)
}

/// OTLP 导出端点设置（批次 G §3.12）：留空即关闭导出。
/// 存之前先校验协议与格式，避免把明显写错的地址存下来。
#[tauri::command]
pub(crate) fn set_otlp_endpoint(app: tauri::AppHandle, endpoint: Option<String>) -> Result<settings::Settings, String> {
    let normalized = match endpoint.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Some(otlp::metrics_url(value)?),
        None => None,
    };
    let next = app.state::<Cfg>().0.try_update(|settings| settings.otlp_endpoint = normalized)?;
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) fn set_dock_enabled(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    if !on { FIRST_INSTALL_INTRO.store(false, Ordering::Release); }
    let prev = app.state::<Cfg>().0.get();
    let next = app.state::<Cfg>().0.try_update(|settings| {
        settings.dock_enabled = on;
        if !on {
            settings.dock = Default::default();
            for clone in &mut settings.island_clones { clone.dock = Default::default(); }
        }
    })?;
    if !on {
        let is_capsule = next.island_style == "capsule";
        for window in island_windows(&app) {
            if is_capsule {
                let id = island_clones::id_from_label(window.label()).flatten();
                dock::undock_stage(&window, island_dock(&prev, id.as_deref()).edge);
            } else {
                dock::undock(&window);
            }
        }
    }
    for id in island_ids(&next) { emit_dock_changed(&app, id.as_deref(), &island_dock(&next, id.as_deref())); }
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) fn set_island_horizontal_dock_full(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let next = app.state::<Cfg>().0.try_update(|settings| settings.island_horizontal_dock_full = on)?;
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) fn set_silent_startup(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    // 已启用的旧版注册项也在用户保存选择时补齐启动标记。
    if !validation::isolated() && windows_autostart_status()? {
        set_autostart(true)?;
    }
    let next = app.state::<Cfg>().0.try_update(|settings| settings.silent_startup = on)?;
    crate::events::settings_changed(&app);
    Ok(next)
}

/// 保持系统唤醒（批次 G §3.12）：开启后运行期间不让系统进入睡眠。
/// 切换立即生效；不支持的系统上打开会报错，不假装成功。
#[tauri::command]
pub(crate) fn set_keep_awake(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let applied = keep_awake::set(on)?;
    let next = app.state::<Cfg>().0.try_update(|settings| settings.keep_awake = applied)?;
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) fn set_theme(app: tauri::AppHandle, theme: String) -> Result<settings::Settings, String> {
    if !matches!(theme.as_str(), "light" | "dark" | "system") {
        return Err("主题必须是 light、dark 或 system".into());
    }
    let next = app.state::<Cfg>().0.try_update(|settings| settings.theme = theme)?;
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) fn set_display_preferences(app: tauri::AppHandle, opacity: u8, scale: u16, shrink_scale: u16, refresh_minutes: u32) -> Result<settings::Settings, String> {
    settings::validate_display_preferences(opacity, scale, shrink_scale, refresh_minutes)?;
    let next = app.state::<Cfg>().0.try_update(|settings| {
        settings.island_opacity = opacity;
        settings.island_scale = scale;
        settings.island_shrink_scale = shrink_scale;
        settings.refresh_minutes = refresh_minutes;
    })?;
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) fn set_balance_alert(app: tauri::AppHandle, threshold: Option<f64>, currency: String) -> Result<settings::Settings, String> {
    if threshold.is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 1_000_000_000.0)
        || currency.len() != 3 || !currency.bytes().all(|c| c.is_ascii_uppercase()) {
        return Err("请输入有效的三位大写币种代码，以及大于 0、不超过 10 亿的阈值".into());
    }
    let next = app.state::<Cfg>().0.try_update(|settings| {
        settings.balance_alert_threshold = threshold;
        settings.balance_alert_currency = currency;
    })?;
    crate::events::settings_changed(&app);
    Ok(next)
}

pub(crate) fn retention_cutoff(days: u32) -> Result<i64, String> {
    if !matches!(days, 30 | 90 | 180 | 365) {
        return Err("历史保留天数必须是 30、90、180 或 365".into());
    }
    Ok(chrono::Utc::now().timestamp_millis() - i64::from(days) * 86_400_000)
}

#[tauri::command]
pub(crate) fn set_retention_days(app: tauri::AppHandle, days: Option<u32>) -> Result<settings::Settings, String> {
    if let Some(days) = days { retention_cutoff(days)?; }
    let next = app.state::<Cfg>().0.try_update(|settings| settings.retention_days = days)?;
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) async fn cleanup_preview(db: State<'_, DbRead>, days: u32) -> Result<db::CleanupPreview, String> {
    let cutoff = retention_cutoff(days)?;
    with_reader(db.0.clone(), move |conn| {
        db::cleanup_preview(conn, cutoff).map_err(|error| error.to_string())
    }).await
}

#[tauri::command]
pub(crate) async fn cleanup_history(app: tauri::AppHandle, db: State<'_, Db>, days: u32) -> Result<db::CleanupPreview, String> {
    let cutoff = retention_cutoff(days)?;
    let result = with_database(db.0.clone(), move |conn| {
        db::cleanup_history(conn, cutoff).map_err(|error| error.to_string())
    }).await?;
    crate::events::refresh_requested(&app, None);
    Ok(result)
}

#[tauri::command]
pub(crate) fn get_settings(cfg: State<Cfg>) -> settings::Snapshot {
    // 搜索 Key 在序列化时自动掩码（gateway::search::secret_keys），明文不出后端；
    // 带修订号，前端以它为基准丢弃更旧的 settings-changed
    cfg.0.snapshot()
}

#[tauri::command]
pub(crate) fn set_dnd(app: tauri::AppHandle, cfg: State<Cfg>, on: bool) -> Result<settings::Settings, String> {
    let next = cfg.0.try_update(|s| s.dnd = on)?;
    crate::events::settings_changed(&app);
    refresh_tray_menu(&app);
    Ok(next)
}

/// 代理开关 / 端口 / 回退开关；切换即执行接管（或还原）并启动（或停止）监听。
/// 同步命令：接管与端口绑定都是毫秒级文件写与 bind，失败原因直接返回设置页。
#[tauri::command]
pub(crate) fn set_proxy(
    app: tauri::AppHandle,
    port: u16,
    enabled: bool,
    fallback_direct: bool,
) -> Result<settings::Settings, String> {
    proxy::set_proxy(&app, port, enabled, fallback_direct)
}

#[tauri::command]
pub(crate) fn proxy_status(app: tauri::AppHandle) -> proxy::ProxyStatus {
    proxy::status(&app)
}

/// 费用估算的汇率（USD → 目标币种）。带 24 小时缓存；失败时 fallback=true 用参考价。
#[derive(serde::Serialize)]
pub(crate) struct FxRateDto {
    currency: String,
    rate: f64,
    as_of: String,
    fallback: bool,
}

/// 设置费用估算的显示币种
#[tauri::command]
pub(crate) fn set_cost_currency(app: tauri::AppHandle, currency: String) -> Result<crate::settings::Settings, String> {
    if !matches!(currency.as_str(), "USD" | "CNY") {
        return Err("币种只支持 USD 或 CNY".into());
    }
    let next = app.state::<Cfg>().0.try_update(|settings| settings.cost_currency = currency.clone())?;
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) fn fx_rate(currency: String) -> Result<FxRateDto, String> {
    if currency != "CNY" {
        return Ok(FxRateDto { currency, rate: 1.0, as_of: "基准币种".into(), fallback: false });
    }
    let client = crate::net::blocking_builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("创建汇率客户端失败：{e}"))?;
    let rate = crate::fx::usd_cny(&client);
    Ok(FxRateDto { currency, rate: rate.rate, as_of: rate.as_of, fallback: rate.fallback })
}

// 任务 2.2 结束
// 任务 2.5 开始
/// 任务提醒（画板 94）：两个开关分别保存，传 None 的保持不变
#[tauri::command]
pub(crate) fn set_task_notify(app: tauri::AppHandle, resumed: Option<bool>, finished: Option<bool>, quota: Option<bool>) -> Result<settings::Settings, String> {
    let next = app.state::<Cfg>().0.try_update(|settings| {
        if let Some(on) = resumed { settings.notify_task_resumed = on; }
        if let Some(on) = finished { settings.notify_task_finished = on; }
        if let Some(on) = quota { settings.notify_quota_alerts = on; }
    })?;
    crate::events::settings_changed(&app);
    Ok(next)
}
