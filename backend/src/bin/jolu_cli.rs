//! `jolu` 命令行骨架（批次 G §3.11）。
//!
//! 只读子命令：直接读桌面端的数据目录，**不启动网关监听、不写任何配置**。
//! 写操作留在桌面端——CLI 与桌面端并发写配置的风险计划里已标「中」，
//! 骨架阶段先只做只读，避免引入跨进程互斥的复杂度。
//!
//! 参数解析手写：项目其余 bin（`gateway_mcp_bridge` / `handoff_eval`）都手写，
//! 引入 clap 会让依赖树与 tauri 的构建时间一起变长，而这里只需要几个子命令。
//!
//! 用法：`jolu <子命令> [参数]`；`jolu help` 打印全部用法。

use jolu_lib::service::{gateway, paths};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("错误：{error}");
            1
        }
    };
    std::process::exit(code);
}

fn run(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        print_usage();
        return Ok(());
    };
    let rest = &args[1..];
    match command {
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        "status" => status(),
        "catalog" => catalog(rest),
        "group" => group(rest),
        "keys" => keys(rest),
        "quota" => quota(rest),
        other => Err(format!("未知子命令「{other}」，运行 `jolu help` 查看用法")),
    }
}

fn print_usage() {
    println!(
        "\
jolu — Jolu 命令行（只读）

用法：
  jolu status                     网关总览（模式、端口、接线状态）
  jolu catalog [--json]           模型目录（连接已接入的模型 + 路由分组）
  jolu group list                 来源分组与 Agent 分组
  jolu keys                       命名客户端 Key（只有掩码，没有原文）
  jolu quota <连接 id>            查一个连接的额度

环境变量：
  JOLU_DATA_DIR   指定数据目录（默认按平台从 %APPDATA% / Application Support 推断）

说明：
  所有子命令只读桌面端已有数据，不会启动网关监听，也不会写配置。"
    );
}

fn open_database() -> Result<rusqlite::Connection, String> {
    let (_, database) = paths::locate()?;
    let conn = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| format!("打开数据库失败：{error}"))?;
    Ok(conn)
}

fn status() -> Result<(), String> {
    let conn = open_database()?;
    // CLI 没有运行中的设置文件上下文：从库里的接线记录与默认值组装。
    // 端口等运行期设置属于桌面端，这里如实标出来源是「桌面端当前设置」还是不可用。
    let settings = settings_from_disk()?;
    let status = gateway::status::status(&settings, &conn);
    println!("模式：{}", status.mode);
    println!("监听：{}", if status.enabled { "已开启" } else { "未开启" });
    println!("端口：{}", status.port);
    println!(
        "默认来源：{}",
        status.default_connection.as_deref().unwrap_or("未设置（无前缀的模型名会被拒绝）")
    );
    println!("接线：");
    for wiring in &status.wiring {
        let state = if wiring.wired { "已接入" } else { "未接入" };
        println!("  - {:<12} {state}", wiring.agent);
    }
    Ok(())
}

/// 从桌面端的设置文件读 Settings。找不到时用默认值并提示——CLI 不猜运行期状态。
fn settings_from_disk() -> Result<jolu_lib::service::settings::Settings, String> {
    jolu_lib::service::settings::load()
}

fn catalog(args: &[String]) -> Result<(), String> {
    let conn = open_database()?;
    let json = args.iter().any(|arg| arg == "--json");
    // models.dev 元数据缓存由桌面端维护；CLI 只读已缓存的部分，缺失就按没有元数据处理
    let meta = jolu_lib::service::settings::models_meta();
    let models = gateway::catalog::list(&conn, &|model: &str| meta.as_ref().and_then(|cache| cache(model)))?;
    if json {
        let text = serde_json::to_string_pretty(&models).map_err(|error| error.to_string())?;
        println!("{text}");
        return Ok(());
    }
    if models.is_empty() {
        println!("模型目录为空：还没有来源被设为对外服务。");
        return Ok(());
    }
    for model in &models {
        let context = model
            .context_window
            .map(|value| format!(" · {} 上下文", value))
            .unwrap_or_default();
        println!("{:<40} {}{}", model.id, model.source, context);
    }
    Ok(())
}

fn group(args: &[String]) -> Result<(), String> {
    let action = args.first().map(String::as_str).unwrap_or("list");
    if action != "list" {
        return Err(format!(
            "分组只支持只读的 `jolu group list`（写操作请在桌面端做），收到「{action}」"
        ));
    }
    let conn = open_database()?;
    let all = gateway::groups::list(&conn)?;
    println!("来源分组：");
    for group in &all.sources {
        println!("  - {} ({})", group.name, group.members.len());
    }
    println!("Agent 分组：");
    for group in &all.agents {
        println!("  - {} ({})", group.name, group.members.len());
    }
    Ok(())
}

fn keys(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err("`jolu keys` 不接受参数；签发与撤销请在桌面端做".into());
    }
    let conn = open_database()?;
    let rows = gateway::client_keys::list(&conn)?;
    if rows.is_empty() {
        println!("还没有命名客户端 Key。");
        return Ok(());
    }
    for row in rows {
        let cap = row
            .requests_per_day
            .map(|value| format!("{value}/天"))
            .unwrap_or_else(|| "不限".into());
        println!("{:<16} {}  上限 {}  今日 {}", row.name, row.masked, cap, row.today_requests);
    }
    Ok(())
}

fn quota(args: &[String]) -> Result<(), String> {
    let Some(id) = args.first() else {
        return Err("用 `jolu quota <连接 id>` 指定要查的连接".into());
    };
    let conn = open_database()?;
    let Some(creds) = jolu_lib::service::settings::creds(&conn, id)? else {
        return Err(format!("没有 id 为「{id}」的连接"));
    };
    // 只报路由结论，不真的发请求：查询要经缓存、事件与托盘，属于桌面端职责
    let route = gateway::quota::route_for(id, &creds);
    println!("连接：{}（{} · {}）", creds.name, creds.platform, creds.kind);
    println!("查询链：{route:?}");
    println!("提示：实际额度查询请在桌面端触发（CLI 不发起上游请求）。");
    Ok(())
}
