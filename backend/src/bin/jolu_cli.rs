//! `jolu` 命令行骨架（批次 G §3.11）。
//!
//! 多数子命令只读：直接读桌面端的数据目录，不启动网关监听。
//! 写命令（5.5）：`settings set` 只改白名单里「用到时才读」的键，与桌面端共用文件锁
//! （`filelock`），桌面端 2 秒内读回；`claude run` 不改任何用户配置，只写 Jolu 自己的启动用设置。
//!
//! 参数解析手写：项目其余 bin（`gateway_mcp_bridge` / `handoff_eval`）都手写，
//! 引入 clap 会让依赖树与 tauri 的构建时间一起变长，而这里只需要几个子命令。
//!
//! 用法：`jolu <子命令> [参数]`；`jolu help` 打印全部用法。

use jolu_lib::service::{gateway, paths};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args).await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("错误：{error}");
            1
        }
    };
    std::process::exit(code);
}

async fn run(args: &[String]) -> Result<(), String> {
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
        "restore" => restore(),
        "search" => search(rest).await,
        "fetch" => fetch(rest).await,
        "library" => library(rest).await,
        "settings" => settings_cmd(rest),
        "claude" => claude(rest),
        "tune" => tune(rest),
        "provider" | "providers" => provider(rest).await,
        "save" => desktop_request("profile_save", rest.first().map(String::as_str), "用 `jolu save <名称>` 把当前接线存成 Profile"),
        "use" => desktop_request("profile_use", rest.first().map(String::as_str), "用 `jolu use <名称>` 切换到一个 Profile"),
        "serve" => desktop_request("serve", Some(""), ""),
        other => Err(format!("未知子命令「{other}」，运行 `jolu help` 查看用法")),
    }
}

fn print_usage() {
    println!(
        "\
jolu — Jolu 命令行

用法：
  jolu status                     网关总览（模式、端口、接线状态）
  jolu catalog [--json]           模型目录（连接已接入的模型 + 路由分组）
  jolu group list                 路由分组、来源分组与 Agent 分组
  jolu group add <分组> <策略> <来源/模型>…
                                  新建路由分组（策略 order|rotate|usage|smart|pace，立即生效）
  jolu group set <分组> <来源/模型>…  整体替换成员
  jolu group rm <分组>            删除路由分组
  jolu keys                       命名客户端 Key（只有掩码，没有原文）
  jolu quota <连接 id>            查一个连接的额度
  jolu restore                    把已接线 agent 的配置还原到接线前（卸载/排障用）
  jolu search <关键词>            用设置里第一个启用的搜索后端搜一次
  jolu fetch <网址>               读取一个公网网页的正文（与 MCP 的 web_fetch 相同，内网地址拒绝）
  jolu library install <agent>    把「Jolu 搜索」MCP 装进指定 agent
  jolu library remove <agent> <名> 卸载（只删 Jolu 装的）
  jolu library list               各 agent 的安装状态
  jolu settings list              命令行可改的设置与当前值
  jolu settings get <键>          看一个设置
  jolu settings set <键> <值>     改一个设置（on/off、数字或 none；桌面端 2 秒内生效）
  jolu provider list              连接列表（Key 只有掩码）
  jolu provider add <平台> <名称> [--base-url 地址] [--protocol openai|anthropic] [--model 模型] [--service 服务]
                                  加一个 API Key 连接；Key 从标准输入读，如 `echo sk-... | jolu provider add custom 我的网关 ...`
  jolu provider serve <连接 id> on|off  加入 / 移出模型池（对外服务）
  jolu provider rm <连接 id>      删除连接（历史统计保留）
  jolu save <名称>                把当前接线存成 Profile（由运行中的桌面端执行）
  jolu use <名称>                 切换到一个 Profile（由运行中的桌面端执行）
  jolu serve                      让运行中的桌面端开启模型路由
  jolu tune [--days 天数]         按最近的真实调用回放，建议压缩窗口与缓存时长（只给建议，不改配置）
  jolu claude run [--model <模型>] [claude 的参数…]
                                  让这一次的 Claude Code 走 Jolu 模型路由（不改 ~/.claude/settings.json）

环境变量：
  JOLU_DATA_DIR   指定数据目录（默认按平台从 %APPDATA% / Application Support 推断）

搜索后端与 Key 在桌面端 settings.json 的 search_apis 里配置。
说明：
  除 restore、library install/remove、group add/set/rm、provider add/rm、settings set、save、use、serve 与 claude run 外，子命令只读桌面端已有数据，不会启动网关监听。"
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
    use jolu_lib::service::gateway::route_groups;
    let action = args.first().map(String::as_str).unwrap_or("list");
    let write_db = || -> Result<rusqlite::Connection, String> {
        let (_, database) = paths::locate()?;
        jolu_lib::db::open(&database).map_err(|e| e.to_string())
    };
    let show = |group: &route_groups::RouteGroupView| println!("group/{:<20} {:<7} {}", group.slug, group.strategy, group.members.join(" → "));
    match action {
        "list" => {}
        "add" => {
            let (Some(slug), Some(strategy)) = (args.get(1), args.get(2)) else {
                return Err("用 `jolu group add <分组> <order|rotate|usage|smart|pace> <来源/模型>…`".into());
            };
            let group = route_groups::add(&write_db()?, slug, strategy, &args[3..])?;
            print!("已建：");
            show(&group);
            return Ok(());
        }
        "set" => {
            let Some(slug) = args.get(1) else { return Err("用 `jolu group set <分组> <来源/模型>…`".into()) };
            let group = route_groups::set_members(&write_db()?, slug, &args[2..])?;
            print!("已更新：");
            show(&group);
            return Ok(());
        }
        "rm" => {
            let Some(slug) = args.get(1) else { return Err("用 `jolu group rm <分组>`".into()) };
            route_groups::remove(&write_db()?, slug)?;
            println!("已删除 group/{slug}");
            return Ok(());
        }
        other => return Err(format!("未知 group 动作「{other}」：用 list / add / set / rm")),
    }
    let conn = open_database()?;
    println!("路由分组：");
    for group in route_groups::list(&conn)? {
        print!("  ");
        show(&group);
    }
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

/// 搜一次：用设置里第一个未暂停的后端。Key 轮转状态只活在本进程
/// （CLI 一次性执行，轮转记忆留在桌面端才有意义——这里主要是验证配置与后端通路）。
/// Library（批 5）：把「Jolu 搜索」装进 / 卸出各 agent，或查看状态。
/// 支持的 agent：claude-code / codex / gemini / opencode（zcode 格式待实测）。
async fn library(args: &[String]) -> Result<(), String> {
    let Some(action) = args.first() else {
        return Err("用 `jolu library install <agent>` / `remove <agent> <名>` / `list`".into());
    };
    let (_, database) = paths::locate()?;
    match action.as_str() {
        "list" => {
            let conn = open_database()?;
            for agent in jolu_lib::gateway::wiring::AGENTS {
                let names = jolu_lib::library::applied_names(&conn, agent);
                if names.is_empty() { continue; }
                println!("{agent}: {}", names.join("、"));
            }
            println!("（支持安装的 agent：claude-code / codex / gemini / opencode）");
            Ok(())
        }
        "install" => {
            let Some(agent) = args.get(1) else { return Err("指定 agent：claude-code / codex / gemini / opencode".into()) };
            let (command, cmd_args) = jolu_lib::library::search_entry_command_from_cli().map_err(|e| e.to_string())?;
            let item = jolu_lib::library::search_item_with(std::path::Path::new(&command));
            let mut conn = jolu_lib::db::open(&database).map_err(|e| e.to_string())?;
            let outcome = jolu_lib::library::install(&mut conn, agent, &item).map_err(|e| e.to_string())?;
            if outcome.created {
                println!("已装进 {agent}：{}（{} {}）", outcome.name, command, cmd_args.join(" "));
            } else {
                println!("{agent} 里已存在并更新：{}", outcome.name);
            }
            println!("重启该 agent 后在 /mcp 里可见 web_search 工具。");
            Ok(())
        }
        "remove" => {
            let (Some(agent), Some(name)) = (args.get(1), args.get(2)) else {
                return Err("用 `jolu library remove <agent> <名称>`".into());
            };
            let mut conn = jolu_lib::db::open(&database).map_err(|e| e.to_string())?;
            let removed = jolu_lib::library::remove(&mut conn, agent, name).map_err(|e| e.to_string())?;
            println!("{}", if removed { format!("已从 {agent} 移除 {name}") } else { format!("{agent} 里没有 Jolu 装的「{name}」") });
            Ok(())
        }
        other => Err(format!("未知 library 动作「{other}」")),
    }
}

async fn search(args: &[String]) -> Result<(), String> {
    let Some(query) = args.first() else {
        return Err("用 `jolu search <关键词>` 指定搜索词".into());
    };
    let directory = paths::config_directory()?;
    let settings = jolu_lib::service::settings::read_settings(&directory);
    let Some(config) = settings.search_apis.iter().find(|api| !api.off) else {
        return Err("还没有配置搜索后端：在桌面端 settings.json 的 search_apis 里加一份（vendor + keys）".into());
    };
    let mut refused_tail: Vec<(String, i64)> = Vec::new();
    let hits = jolu_lib::gateway::search::run_search(config, query, 5, &mut refused_tail, now_ms()).await.map_err(|e| e.to_string())?;
    if hits.is_empty() {
        println!("没有结果。");
        return Ok(());
    }
    for hit in &hits {
        println!("■ {}", hit.title);
        println!("  {}", hit.url);
        if !hit.snippet.is_empty() { println!("  {}", hit.snippet); }
    }
    Ok(())
}

async fn fetch(args: &[String]) -> Result<(), String> {
    let Some(url) = args.first() else {
        return Err("用 `jolu fetch <网址>` 指定要读的网页".into());
    };
    let page = jolu_lib::gateway::search::fetch::fetch(url, 4000).await?;
    println!("■ {}
  {}
", page.title, page.url);
    println!("{}", page.text);
    if page.truncated { println!("
（只显示前 4000 字）"); }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// 还原全部 agent 接线。**唯一的写命令**：卸载 Jolu 前手动执行，或网关故障时
/// 把 CLI 拉回直连。走与桌面端同一把跨进程互斥量（cli_switch），运行中的桌面端
/// 不会同时写同一批配置。
fn restore() -> Result<(), String> {
    let (directory, database) = paths::locate()?;
    let settings = jolu_lib::service::settings::read_settings(&directory);
    let port = if settings.proxy_port > 0 { settings.proxy_port } else { 12731 };
    let mut conn = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| format!("打开数据库失败：{error}"))?;
    let report = gateway::wiring_restore::restore_all(&mut conn, port);
    for agent in &report.restored { println!("已还原 {agent}"); }
    for agent in &report.skipped { println!("跳过 {agent}（配置已不在或不是 Jolu 写的）"); }
    for line in &report.failed { println!("失败 {line}"); }
    if report.failed.is_empty() { Ok(()) } else { Err("部分工具还原失败，详见上方输出".into()) }
}

fn settings_cmd(args: &[String]) -> Result<(), String> {
    use jolu_lib::service::settings_write;
    let directory = paths::config_directory()?;
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" => {
            for item in settings_write::SETTABLE {
                let value = settings_write::get(&directory, item.key)?;
                println!("{:<28} {:<14} {}", item.key, value, item.about);
            }
            Ok(())
        }
        "get" => {
            let key = args.get(1).ok_or("用 `jolu settings get <键>`")?;
            println!("{}", settings_write::get(&directory, key)?);
            Ok(())
        }
        "set" => {
            let (Some(key), Some(value)) = (args.get(1), args.get(2)) else {
                return Err("用 `jolu settings set <键> <值>`".into());
            };
            let (before, after) = settings_write::set(&directory, key, value)?;
            println!("{key}：{before} → {after}（桌面端 2 秒内读回生效）");
            Ok(())
        }
        other => Err(format!("未知 settings 动作「{other}」：用 list / get / set")),
    }
}

/// `jolu save` / `use` / `serve`：写请求文件，由运行中的桌面端执行（`cli_requests`）
fn desktop_request(action: &str, name: Option<&str>, usage: &str) -> Result<(), String> {
    let name = match name {
        Some(name) => name,
        None => return Err(usage.into()),
    };
    let directory = paths::config_directory()?;
    let name = (!name.is_empty()).then_some(name);
    let response = jolu_lib::cli_requests::send(&directory, action, name, std::time::Duration::from_secs(5), std::time::Duration::from_secs(90))?;
    if response.ok {
        println!("{}", response.message);
        Ok(())
    } else {
        Err(response.message)
    }
}

async fn provider(args: &[String]) -> Result<(), String> {
    use jolu_lib::service::providers;
    let (_, database) = paths::locate()?;
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" => {
            let conn = open_database()?;
            let rows = providers::list(&conn)?;
            if rows.is_empty() {
                println!("还没有连接。");
            }
            for row in rows {
                println!("{:<44} {:<10} {:<4} {:<20} {:<16} {}", row.id, row.platform, row.kind, row.name, row.masked, row.status);
            }
            Ok(())
        }
        "add" => {
            let (Some(platform), Some(name)) = (args.get(1), args.get(2)) else {
                return Err("用 `jolu provider add <平台> <名称> [--base-url 地址] [--protocol openai|anthropic] [--model 模型] [--service 服务]`，Key 从标准输入传入".into());
            };
            let mut flags = std::collections::HashMap::new();
            let mut rest = args[3..].iter();
            while let Some(flag) = rest.next() {
                if flag == "--key" || flag.starts_with("--key=") {
                    return Err("不要把 Key 写在命令行参数里（会留在 shell 历史）：从标准输入传入".into());
                }
                let key = flag.strip_prefix("--").filter(|k| ["base-url", "protocol", "model", "service"].contains(k)).ok_or_else(|| format!("不认识的参数「{flag}」"))?;
                let value = rest.next().ok_or_else(|| format!("{flag} 后面要写值"))?;
                flags.insert(key.to_string(), value.clone());
            }
            let mut secret = String::new();
            std::io::stdin().read_line(&mut secret).map_err(|e| format!("读标准输入失败：{e}"))?;
            let input = providers::ApiKeyInput {
                platform: platform.clone(),
                name: name.clone(),
                secret: secret.trim().to_string(),
                base_url: flags.remove("base-url"),
                protocol: flags.remove("protocol"),
                model: flags.remove("model"),
                service: flags.remove("service"),
            };
            // 校验会发阻塞的网络请求：放到独立线程，不占异步运行时
            let id = tokio::task::spawn_blocking(move || {
                let mut conn = jolu_lib::db::open(&database).map_err(|e| e.to_string())?;
                providers::add_api_key(&mut conn, input)
            })
            .await
            .map_err(|e| e.to_string())??;
            println!("已添加连接 {id}（Key 已验证；桌面端 2 秒内刷新）");
            Ok(())
        }
        "serve" => {
            let (Some(id), Some(state)) = (args.get(1), args.get(2)) else { return Err("用 `jolu provider serve <连接 id> on|off`".into()) };
            let on = match state.as_str() { "on" => true, "off" => false, _ => return Err("最后写 on 或 off".into()) };
            let conn = jolu_lib::db::open(&database).map_err(|e| e.to_string())?;
            providers::set_serve(&conn, id, on)?;
            println!("{id} 已{}模型池（立即生效）", if on { "加入" } else { "移出" });
            Ok(())
        }
        "rm" => {
            let id = args.get(1).ok_or("用 `jolu provider rm <连接 id>`")?;
            let conn = jolu_lib::db::open(&database).map_err(|e| e.to_string())?;
            providers::remove(&conn, id)?;
            println!("已删除连接 {id}（历史统计保留）");
            Ok(())
        }
        other => Err(format!("未知 provider 动作「{other}」：用 list / add / serve / rm")),
    }
}

/// `jolu claude run [--model <模型>] [claude 的参数…]`：本次启动走模型路由，退出码沿用 claude 的
fn claude(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) != Some("run") {
        return Err("用 `jolu claude run [--model <模型>] [claude 的参数…]`".into());
    }
    let mut rest: Vec<String> = args[1..].to_vec();
    let mut model = None;
    if rest.first().map(String::as_str) == Some("--model") {
        if rest.len() < 2 {
            return Err("--model 后面要写模型 id，如 kimi/k2".into());
        }
        model = Some(rest[1].clone());
        rest.drain(..2);
    }
    let directory = paths::config_directory()?;
    let launch = jolu_lib::service::launch::prepare_claude(&directory, model.as_deref())?;
    let status = std::process::Command::new(&launch.exe)
        .arg("--settings")
        .arg(&launch.settings_file)
        .args(&rest)
        .status()
        .map_err(|e| format!("启动 {} 失败：{e}", launch.exe.display()))?;
    std::process::exit(status.code().unwrap_or(1));
}

fn tune(args: &[String]) -> Result<(), String> {
    let days = match args {
        [] => 30,
        [flag, n] if flag == "--days" => n.parse::<i64>().map_err(|_| "--days 后面写天数".to_string())?,
        _ => return Err("用 `jolu tune [--days 天数]`".into()),
    };
    let conn = open_database()?;
    let advice = jolu_lib::service::tune::advise(&conn, days, now_ms())?;
    let k = |n: i64| format!("{}K", (n + 500) / 1000);
    let save = |cost: f64, low: f64| if cost > 0.0 { format!("{:.0}%", (cost - low) / cost * 100.0) } else { "0%".into() };
    if advice.is_empty() {
        println!("最近 {days} 天没有 Claude Code / Codex 的调用记录。");
    }
    for agent in &advice {
        let name = if agent.platform == "claude" { "Claude Code" } else { "Codex" };
        println!("■ {name}：最近 {days} 天 {} 次调用", agent.calls);
        if agent.compactions.is_empty() {
            println!("  压缩窗口：调用太少（每个模型至少 {} 次）或从没接近窗口，暂不建议", jolu_lib::tune::MIN_CALLS);
        }
        for c in &agent.compactions {
            let current = match agent.compact_setting { Some(_) => format!("现在 {}", k(c.current)), None => format!("现在约 {}（没设，按压缩点推断）", k(c.current)) };
            if c.best == c.current {
                println!("  {} 压缩窗口：{current}，保持即可（{} 次调用，{} 个会话，压缩 {} 次）", c.model, c.facts.calls, c.facts.streams, c.facts.compacts);
            } else {
                let window = if c.window > 0 { String::new() } else { "；模型窗口未知，建议值可能超过它，改前先核对".into() };
                println!("  {} 压缩窗口：{current} → 建议 {}，加权 Token 省 {}（{} 次调用，压缩 {} 次{window}）", c.model, k(c.best), save(c.cost, c.low), c.facts.calls, c.facts.compacts);
            }
        }
        if let Some(t) = &agent.cache {
            let inferred = if t.current_inferred { "（Jolu 记录里没有 1 小时写入的拆分，按 5 分钟推断）" } else { "" };
            let gaps = format!("间隔 5 分钟内 {:.0}%、5 分钟到 1 小时 {:.0}%", t.facts.quick * 100.0, t.facts.pause * 100.0);
            if t.best == t.current {
                println!("  提示词缓存：{}{inferred}，保持即可（{gaps}）", t.current);
            } else {
                println!("  提示词缓存：{}{inferred} → 建议 {}，加权 Token 省 {}（{gaps}）", t.current, t.best, save(t.cost, t.low));
            }
        }
    }
    println!("说明：结论只按你这台电脑这段时间的调用回放得出；改设置请在工具自己的配置里改。");
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
