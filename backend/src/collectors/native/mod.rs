//! 经核实的本机来源：只读第三方文件，记录写入本应用数据库。

pub(crate) mod parse;
pub(crate) mod zcode;
#[cfg(test)]
pub(crate) mod tests;

pub use zcode::{backfill_zcode_cancelled, backfill_zcode_cwd};
#[allow(unused_imports)]
pub(crate) use zcode::{zcode, zcode_boost_running, zcode_native_outcome, zcode_observed_state, zcode_session_state, ZCODE_RUNNING_GRACE_MS, ZCODE_TURN_GAP_MS};

use crate::{
    collector::ScanResult,
    db::Store,
    
    source_store,
};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize)]
pub struct Source {
    pub id: String,
    pub platform: String,
    pub name: String,
    pub available: bool,
    pub capabilities: Vec<String>,
    pub reason: Option<String>,
    #[serde(skip)]
    pub path: PathBuf,
}

pub fn home() -> Option<PathBuf> {
    crate::creds::home()
}
pub(crate) fn app_data() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub fn sources_at(home: &Path, app: &Path) -> Vec<Source> {
    let mut list: Vec<Source> = [
        ("opencode", "db", "OpenCode", crate::opencode::data_dir(home).join("opencode.db")),
        ("opencode", "json", "OpenCode JSON", crate::opencode::data_dir(home).join("storage/message")),
        ("gemini", "cli", "Gemini CLI", home.join(".gemini/tmp")),
        ("grok", "cli", "Grok CLI", home.join(".grok/auth.json")),
        (
            "zcode",
            "main",
            "Zcode",
            home.join(".zcode/cli/db/db.sqlite"),
        ),
        ("trae", "ide", "Trae IDE", app.join("Trae")),
        ("trae", "cn", "Trae 国内版", app.join("Trae CN")),
        // 登录凭据跟 dataBaseDir 走；上面的会话库 cli/db 留在原位（见 creds::zcode_data_dir）
        ("zcode", "account", "ZCode 账号", crate::creds::zcode_data_dir(home).join("v2").join("credentials.json")),
        (
            "qoder",
            "global",
            "Qoder 国际版",
            home.join(".qoder/projects"),
        ),
        (
            "qoder",
            "cn",
            "Qoder 国内版",
            home.join(".qoder-cn/projects"),
        ),
        (
            "workbuddy",
            "main",
            "WorkBuddy",
            home.join(".workbuddy/projects"),
        ),
        (
            "workbuddy",
            "ai",
            "WorkBuddy AI",
            home.join(".workbuddy-ai/projects"),
        ),
        (
            "kimi",
            "code",
            "Kimi Code",
            crate::kimi_sessions::kimi_home(home).join("sessions"),
        ),
        (
            "pi",
            "agent",
            "Pi",
            crate::pi_sessions::pi_agent_dir(home).join("sessions"),
        ),
        (
            "dsh",
            "main",
            "DeepSeek Harness",
            crate::dsh_sessions::dsh_home(home).join("sessions"),
        ),
        (
            "qwen",
            "cli",
            "Qwen Code",
            crate::qwen_sessions::qwen_home(home),
        ),
    ]
    .into_iter()
    .map(|(platform, instance, name, path)| {
        let implemented =
            crate::platforms::supports(platform, "local_sessions") || platform == "grok" || platform == "trae";
        // Qoder 有两套本地布局（2026-11-23 对账查出：本机只有用量库，没有
        // `~/.qoder/projects`；只按 projects 判定会让整个来源永不启用）。
        //
        // 但账本只有一份，属于国际版客户端（`~/.qoder/`）。国内版是 `~/.qoder-cn/`，
        // 它自己的数据目录没有账本 —— 不能让 cn 实例共用国际版账本，否则同一批记录
        // 会被两个 source 各写一遍（去重键含 source，会真的变成两行）。
        let available = if platform == "qoder" && instance == "global" {
            implemented && (path.exists() || crate::qoder_usage::ledger_path(home).is_file())
        } else if platform == "qwen" {
            implemented && crate::qwen_sessions::has_data(home)
        } else {
            path.exists() && implemented
        };
        Source {
            id: format!("native:{platform}:{instance}"),
            platform: platform.into(),
            name: name.into(),
            available,
            capabilities: crate::platforms::get(platform)
                .map(|p| p.capabilities.clone())
                .unwrap_or_default(),
            reason: if !implemented {
                Some(crate::platforms::limitation(platform))
            } else if !available {
                Some(format!(
                    "未发现 {name} 的本机数据，请先在对应应用中登录并产生记录"
                ))
            } else {
                Some(crate::platforms::limitation(platform))
            },
            path,
        }
    })
    .collect();

    // Antigravity CLI：自动发现官方目录与 CCS 托管多账号目录
    let ag_impl = crate::platforms::supports("antigravity", "local_sessions");
    let ag_caps = crate::platforms::get("antigravity")
        .map(|p| p.capabilities.clone())
        .unwrap_or_default();
    for root in crate::antigravity_usage::discover_roots(home) {
        let available = ag_impl && crate::antigravity_usage::root_has_data(&root.path);
        list.push(Source {
            id: root.source_id,
            platform: "antigravity".into(),
            name: root.name.clone(),
            available,
            capabilities: ag_caps.clone(),
            reason: if !available {
                Some(format!("未发现 {} 的本机数据，请先完成一次会话", root.name))
            } else {
                Some(crate::platforms::limitation("antigravity"))
            },
            path: root.path,
        });
    }

    list
}

pub fn sources(platform: &str) -> Vec<Source> {
    home()
        .map(|h| {
            sources_at(&h, &app_data())
                .into_iter()
                .filter(|s| s.platform == platform)
                .collect()
        })
        .unwrap_or_default()
}
pub fn monitor_id(id: &str) -> Option<&str> {
    id.strip_prefix("monitor:")
}

/// 探测已在数据库锁外完成；这里仅登记所选来源，不执行外部程序或网络请求。
pub(crate) fn register_accounts(conn:&Connection,platform:&str,_name:Option<&str>,accounts:Vec<crate::login_detect::NativeAccount>) -> Result<(usize,usize,Vec<String>),String> {
        if !matches!(platform,"qoder"|"workbuddy"|"trae") { return Err("账号来源平台无效".into()); }
        if accounts.is_empty() || accounts.iter().any(|account| !matches!((platform,account.source_id),
            ("qoder","native:qoder:cn"|"native:qoder:global")|("workbuddy","native:workbuddy:main"|"native:workbuddy:ai")|("trae","native:trae:cn"|"native:trae:ide"))) {
            return Err("账号来源与供应商不匹配".into());
        }
        let mut added=0;
        let mut existing=0;
        for account in accounts {
            let official = account.account.as_deref().map(str::trim).filter(|value| !value.is_empty() && value.len() <= 320 && !value.chars().any(char::is_control));
            let display=crate::connections::validate_name(&official.unwrap_or(account.name).chars().take(80).collect::<String>())?;
            let id=format!("monitor:{}",account.source_id);
            let slug=crate::gateway::identity::allocate_slug(conn,&id,platform,&display,None).map_err(|_|"分配本机账号标识失败")?;
            let changed=conn.execute("INSERT OR IGNORE INTO connections(id,platform,kind,name,label,masked,status,created_ms,account_email,slug) VALUES(?1,?2,'auth',?3,'本机来源','','connected',?4,?5,?6)",
                params![id,platform,display,chrono::Utc::now().timestamp_millis(),official,slug]).map_err(|_|"保存本机账号失败")?;
            added+=changed;
            existing+=usize::from(changed==0);
        }
        Ok((added,existing,vec![]))
}

pub fn register(conn:&Connection,platform:&str,_name:Option<&str>) -> Result<(usize,usize,Vec<String>),String> {
    if platform=="ollama" {
        let user=crate::ollama_quota::local_user().map_err(|error|error.reason())?;
        let official=user.account();
        let display=_name.map(str::trim).filter(|value|!value.is_empty()).map(str::to_string).or(official.clone()).unwrap_or_else(||"Ollama 账号".into());
        let display=crate::connections::validate_name(&display.chars().take(80).collect::<String>())?;
        let id="monitor:native:ollama:account";
        let slug=crate::gateway::identity::allocate_slug(conn,id,platform,&display,None).map_err(|e|e.to_string())?;
        let label=user.plan().map(|plan|format!("Ollama · {plan}")).unwrap_or_else(||"本机登录".into());
        let n=conn.execute("INSERT OR IGNORE INTO connections(id,platform,kind,name,label,masked,status,created_ms,account_email,slug) VALUES(?1,'ollama','auth',?2,?3,'','connected',?4,?5,?6)",params![id,display,label,chrono::Utc::now().timestamp_millis(),official,slug]).map_err(|e|e.to_string())?;
        return Ok((n,usize::from(n==0),vec!["只读关联 Ollama 本机登录；不复制或刷新认证信息".into()]));
    }
    if matches!(platform,"qoder"|"workbuddy"|"trae") { return Err("请先在数据库锁外读取对应区域的本地登录".into()); }
    if platform=="opencode" {
        if !crate::opencode::openai_logged_in(){return Err("请先在 OpenCode 中登录 OpenAI".into())}
        let name=crate::connections::validate_name("OpenCode · OpenAI")?;
        let id="monitor:native:opencode:openai";
        let slug=crate::gateway::identity::allocate_slug(conn,id,"opencode",&name,None).map_err(|e|e.to_string())?;
        let n=conn.execute("INSERT OR IGNORE INTO connections(id,platform,kind,name,label,masked,status,created_ms,service,slug) VALUES(?1,'opencode','auth',?2,'OpenAI 订阅','','connected',?3,'openai',?4)",params![id,name,chrono::Utc::now().timestamp_millis(),slug]).map_err(|e|e.to_string())?;
        return Ok((n,usize::from(n==0),vec!["只读关联本机登录；不复制或刷新 OAuth 凭证".into()]));
    }
    if !crate::platforms::native(platform) {
        return Err("不是本机来源平台".into());
    }
    let found: Vec<_> = sources(platform)
        .into_iter()
        .filter(|s| s.available)
        .collect();
    if found.is_empty() {
        return Err(sources(platform)
            .first()
            .and_then(|s| s.reason.clone())
            .unwrap_or_else(|| "未发现可用本机来源".into()));
    }
    register_sources(conn, found, None)
}
pub(crate) fn register_sources(
    conn: &Connection,
    found: Vec<Source>,
    name: Option<&str>,
) -> Result<(usize, usize, Vec<String>), String> {
    let mut added = 0;
    let mut existing = 0;
    let mut warnings = Vec::new();
    for s in found {
        if let Err(reason) = validate_source(&s) {
            warnings.push(format!("{}：{reason}", s.name));
            continue;
        }
        let id = format!("monitor:{}", s.id);
        let display = name.map(str::trim).unwrap_or(&s.name);
        let slug = crate::gateway::identity::allocate_slug(conn, &id, &s.platform, display, None).map_err(|e| e.to_string())?;
        let changed=conn.execute("INSERT OR IGNORE INTO connections(id,platform,kind,name,label,secret,masked,status,created_ms,slug)
            VALUES(?1,?2,'auth',?3,'本机来源',NULL,'','paused',?4,?5)",
            params![id,s.platform,display,chrono::Utc::now().timestamp_millis(),slug]).map_err(|e|e.to_string())?;
        if changed > 0 {
            added += 1;
        } else {
            existing += 1;
        }
    }
    if added + existing == 0 {
        return Err(warnings.join("；"));
    }
    Ok((added, existing, warnings))
}

pub fn check(id: &str, platform: &str) -> Result<Source, String> {
    if platform=="ollama" && id=="monitor:native:ollama:account" {
        let user=crate::ollama_quota::local_user().map_err(|error|error.reason())?;
        let path=home().ok_or("无法定位用户目录")?.join(".ollama");
        return Ok(Source{id:"native:ollama:account".into(),platform:"ollama".into(),name:user.account().unwrap_or_else(||"Ollama 账号".into()),available:true,capabilities:crate::platforms::get("ollama").map(|p|p.capabilities.clone()).unwrap_or_default(),reason:None,path});
    }
    if id=="monitor:native:opencode:openai" && platform=="opencode" {
        if !crate::opencode::openai_logged_in(){return Err("OpenCode 中未检测到 OpenAI 登录".into())}
        return Ok(Source{id:"native:opencode:openai".into(),platform:"opencode".into(),name:"OpenAI".into(),available:true,capabilities:vec![],reason:None,path:crate::opencode::data_dir(&home().ok_or("无法定位用户目录")?).join("auth.json")});
    }
    let key = monitor_id(id).ok_or("该旧连接没有有效的本机来源标识，请重新获取")?;
    let source = sources(platform)
        .into_iter()
        .find(|s| s.id == key)
        .ok_or("本机来源标识无效")?;
    if matches!(platform,"qoder"|"workbuddy"|"trae") {
        let edition=match key {"native:qoder:cn"|"native:workbuddy:main"|"native:trae:cn"=>"cn","native:qoder:global"|"native:trae:ide"=>"global",_=>"ai"};
        if crate::login_detect::native_accounts_for(platform,Some(edition))?.iter().any(|a|a.source_id==key) {return Ok(Source {available:true,..source})}
        return Err("所选版本没有有效本地登录，请重新登录或选择浏览器授权".into());
    }
    if !source.available {
        return Err(source.reason.unwrap_or_else(|| "本机来源不可用".into()));
    }
    validate_source(&source)?;
    Ok(source)
}

pub(crate) fn validate_source(source: &Source) -> Result<(), String> {
    if source.platform == "trae" {
        return crate::trae_quota::validate_source(&source.path);
    }
    if source.id == "native:zcode:account" {
        let home = source.path.parent().and_then(Path::parent).and_then(Path::parent).ok_or("ZCode 账号路径无效")?;
        return crate::zcode_quota::validate_local_account_at(home);
    }
    if source.platform == "opencode" {
        if source.path.is_dir(){crate::opencode_usage::read_json_files(&source.path)?;}else{let db=readonly(&source.path)?;let layout=crate::opencode_usage::detect_layout(&db)?;crate::opencode_usage::read_usage(&db,layout,i64::MAX)?;}
        return Ok(());
    }
    if source.platform == "grok" {
        return crate::creds::local_grok_auth().map(|_| ());
    }
    if source.platform == "antigravity" {
        if crate::antigravity_usage::root_has_data(&source.path) {
            return Ok(());
        }
        return Err("目录已发现，但尚无 Antigravity 会话记录".into());
    }
    if matches!(source.platform.as_str(), "kimi" | "pi" | "dsh" | "qwen") {
        if source.path.is_dir() {
            return Ok(());
        }
        return Err("目录不存在，尚无会话记录".into());
    }
    if source.platform == "zcode" {
        readonly(&source.path)?.prepare("SELECT id,session_id,input_tokens,cache_read_input_tokens,cache_creation_input_tokens,raw_usage_json FROM model_usage LIMIT 0")
            .map_err(|_|"Zcode 数据库结构不受支持，尚不能启用采集".to_string())?;
        return Ok(());
    }
    let mut paths = Vec::new();
    files(&source.path, &mut paths)?;
    let mut malformed = 0;
    for path in paths {
        if source.platform == "gemini"
            && !path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("session-"))
        {
            continue;
        }
        if source.platform != "gemini" && path.extension().is_none_or(|s| s != "jsonl") {
            continue;
        }
        let lines = match json_lines(&path) {
            Ok(lines) => lines,
            Err(_) => {
                malformed += 1;
                continue;
            }
        };
        let valid = if source.platform == "gemini" {
            !parse::gemini_records(&lines, &source.id).is_empty()
        } else {
            lines.iter().any(|line| {
                if source.platform == "workbuddy" {
                    parse::workbuddy(line, &source.id, "probe").is_some()
                } else {
                    parse::qoder(line, &source.id, "probe", None).is_some()
                }
            })
        };
        if valid {
            return Ok(());
        }
    }
    Err(if malformed > 0 {
        "未找到可解析的用量记录，部分文件格式无效"
    } else {
        "目录已发现，但尚无可采集的用量记录；请先在对应应用完成一次会话"
    }
    .into())
}

pub(crate) fn readonly(path: &Path) -> Result<Connection, String> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .and_then(|c| {
        c.busy_timeout(std::time::Duration::from_millis(200))?;
        Ok(c)
    })
    .map_err(|e| format!("只读数据库无法打开：{e}"))
}

pub(crate) fn files(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(path).map_err(|e| format!("无法读取来源目录：{e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            continue;
        }
        let p = entry.path();
        if kind.is_dir() {
            files(&p, out)?;
        } else if p.extension().is_some_and(|s| s == "jsonl" || s == "json") {
            out.push(p);
        }
    }
    Ok(())
}
pub(crate) fn stamp(path: &Path) -> Result<(i64, i64), String> {
    let m = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let time = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok((m.len() as i64, time))
}
/// 戳记里的 `parser_initialized=1` 表示 offset 落在完整行边界上，可以据此续读；
/// 旧版本按整份文件大小记录的戳记没有这个标记，首次升级时整份重读一次。
pub(crate) fn save_stamp(conn: &Connection, key: &str, size: i64, time: i64) -> rusqlite::Result<usize> {
    conn.prepare_cached("INSERT INTO scan_state(path,offset,mtime,parser_initialized) VALUES(?1,?2,?3,1)
        ON CONFLICT(path) DO UPDATE SET offset=?2,mtime=?3,parser_initialized=1")?.execute(params![key,size,time])
}

/// 第三方 SQLite（Zcode 用量库、Qoder / WorkBuddy 会话库）的内容戳记：主库与 WAL 任一变化即视为有新数据。
pub(crate) fn db_stamps(path: &Path) -> Result<[(i64, i64); 2], String> {
    let wal = PathBuf::from(format!("{}-wal", path.to_string_lossy()));
    Ok([stamp(path)?, if wal.exists() { stamp(&wal)? } else { (0, 0) }])
}

pub(crate) fn db_unchanged(conn: &Connection, key: &str, stamps: [(i64, i64); 2]) -> bool {
    crate::db::get_scan_state(conn, key) == Some(stamps[0])
        && crate::db::get_scan_state(conn, &format!("{key}-wal")) == Some(stamps[1])
}

pub(crate) fn save_db_stamps(conn: &Connection, key: &str, stamps: [(i64, i64); 2]) -> rusqlite::Result<()> {
    save_stamp(conn, key, stamps[0].0, stamps[0].1)?;
    save_stamp(conn, &format!("{key}-wal"), stamps[1].0, stamps[1].1)?;
    Ok(())
}

pub(crate) fn json_lines(path: &Path) -> Result<Vec<Value>, String> {
    json_lines_from(path, 0).map(|(lines, _)| lines)
}

/// 从 `start` 起读取完整的 JSONL 行，返回解析结果与读到的行边界位置；
/// 没有换行的尾行仍在写入，留给下次。`.json` 快照总是整份读取。
pub(crate) fn json_lines_from(path: &Path, start: i64) -> Result<(Vec<Value>, i64), String> {
    use std::io::{Seek, SeekFrom};
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size.saturating_sub(start.max(0) as u64) > 64 * 1024 * 1024 {
        return Err("会话文件超过 64 MiB 读取上限".into());
    }
    if path.extension().is_some_and(|s| s == "json") {
        if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
            return Err("会话快照超过 64 MiB 读取上限".into());
        }
        return serde_json::from_reader(std::fs::File::open(path).map_err(|e| e.to_string())?)
            .map(|v| (vec![v], size as i64))
            .map_err(|_| "会话 JSON 格式无效".into());
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(start.max(0) as u64)).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut out = Vec::new();
    let mut bytes = Vec::new();
    let mut consumed = start.max(0);
    loop {
        bytes.clear();
        let read = reader
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        if bytes.last() != Some(&b'\n') {
            break;
        } // 尚在追加的尾行留给下次
        consumed += read as i64;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        out.push(
            serde_json::from_slice(&bytes)
                .map_err(|_| "会话含无效 JSON 行，未推进读取位点".to_string())?,
        );
    }
    Ok((out, consumed))
}

/// Qoder / WorkBuddy 的会话日志只追加：已处理到的行边界之后才有新记录。
/// 位点缺失、来自旧版本、文件被截断或等长重写、或位点不在行边界上时从头读。
pub(crate) fn resume_offset(path: &Path, saved: Option<&crate::db::ScanCheckpoint>, size: i64, time: i64) -> i64 {
    use std::io::{Read, Seek, SeekFrom};
    let Some(saved) = saved.filter(|saved| saved.parser_initialized) else { return 0 };
    if saved.offset <= 0 || saved.offset > size || (saved.offset == size && saved.mtime != time) {
        return 0;
    }
    let boundary = std::fs::File::open(path).ok().and_then(|mut file| {
        file.seek(SeekFrom::Start(saved.offset as u64 - 1)).ok()?;
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte).ok()?;
        Some(byte[0] == b'\n')
    });
    if boundary == Some(true) { saved.offset } else { 0 }
}

pub(crate) fn qoder_availability(app: &Path, source: &str) -> HashMap<String, (bool, i64)> {
    let product = if source.ends_with(":cn") {
        "com.qodercn.app.stable"
    } else {
        "com.qoder.app.stable"
    };
    let Ok(db) = readonly(&app.join(product).join("main.sqlite")) else {
        return HashMap::new();
    };
    let Ok(mut q) =
        db.prepare("SELECT session_id,snapshot_json,updated_at FROM chat_session_context_usage")
    else {
        return HashMap::new();
    };
    let Ok(rows) = q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    }) else {
        return HashMap::new();
    };
    rows.flatten()
        .filter_map(|(id, json, ts)| {
            let v: Value = serde_json::from_str(&json).ok()?;
            Some((id, (v.get("tokenCountsAvailable")?.as_bool()?, ts)))
        })
        .collect()
}

pub(crate) fn scan_files(
    db: &mut impl Store,
    source: &Source,
    app: &Path,
) -> Result<(usize, Vec<String>), String> {
    let mut paths = Vec::new();
    files(&source.path, &mut paths)?;
    paths.sort();
    // Qoder 上下文快照只用于解释新记录：没有文件需要解析时不去读它的会话库。
    let mut availability: Option<HashMap<String, (bool, i64)>> = None;
    let mut changed = 0;
    let mut errors = Vec::new();
    for path in paths {
        let result = (|| -> Result<usize, String> {
            if source.platform == "gemini" {
                if !path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with("session-"))
                {
                    return Ok(0);
                }
                if path.extension().is_some_and(|s| s == "json")
                    && path.with_extension("jsonl").exists()
                {
                    return Ok(0);
                }
            } else if !path.extension().is_some_and(|s| s == "jsonl") {
                return Ok(0);
            }
            let (size, time) = stamp(&path)?;
            let key = path.to_string_lossy().to_string();
            let saved = db.write(|conn| crate::db::get_scan_checkpoint(conn, &key));
            // Gemini 是整份留存快照（rewind 会删消息），只能整份重建；其余来源按行追加续读。
            let snapshot = source.platform == "gemini";
            let start = if snapshot {
                if saved.as_ref().is_some_and(|saved| (saved.offset, saved.mtime) == (size, time)) {
                    return Ok(0);
                }
                0
            } else {
                let start = resume_offset(&path, saved.as_ref(), size, time);
                if start == size && saved.as_ref().is_some_and(|saved| saved.mtime == time) {
                    return Ok(0);
                }
                start
            };
            let (lines, consumed) = json_lines_from(&path, start)?;
            let fallback = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let records = if snapshot {
                parse::gemini_records(&lines, &source.id)
            } else {
                if source.platform == "qoder" && availability.is_none() && !lines.is_empty() {
                    availability = Some(qoder_availability(app, &source.id));
                }
                let empty = HashMap::new();
                let availability = availability.as_ref().unwrap_or(&empty);
                let mut latest = HashMap::new();
                for line in &lines {
                    let r = if source.platform == "workbuddy" {
                        parse::workbuddy(line, &source.id, fallback)
                    } else {
                        let session = parse::text(line, "sessionId").unwrap_or(fallback);
                        let available = availability.get(session).and_then(|(b, t)| {
                            // 当前上下文快照只能解释其附近同一轮记录，不能覆盖旧历史。
                            let ts = line.get("timestamp").and_then(parse::timestamp)?;
                            ((ts - *t).abs() <= 60_000).then_some(*b)
                        });
                        parse::qoder(line, &source.id, fallback, available)
                    };
                    if let Some(r) = r {
                        latest.insert(r.request.dedup_key.clone(), r);
                    }
                }
                latest.into_values().collect()
            };
            let gem_act = if snapshot {
                parse::gemini_activity(&lines, &source.id, fallback)
            } else {
                None
            };
            db.write(|conn| -> Result<usize, String> {
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                let mut file_changed = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
                if let Some((ref full_sid, st, ts, turn_start)) = gem_act {
                    crate::db::set_session_activity(&tx, &source.platform, full_sid, st, ts)
                        .map_err(|e| e.to_string())?;
                    if let Some(start_ms) = turn_start {
                        crate::db::set_session_turn_start(&tx, &source.platform, full_sid, start_ms)
                            .map_err(|e| e.to_string())?;
                    }
                } else {
                    for r in &records {
                        if let Some(id) = &r.request.session_id {
                            crate::db::set_session_activity(
                                &tx,
                                &source.platform,
                                id,
                                "unknown",
                                r.request.ts,
                            )
                            .map_err(|e| e.to_string())?;
                        }
                    }
                }
                if snapshot {
                    // 原文件是留存快照；官方 rewind 后移除的消息也从该快照统计中移除。
                    if let Some(sid) = lines.iter().rev().find_map(|v| {
                        parse::text(v, "sessionId")
                            .or_else(|| v.get("$set").and_then(|v| parse::text(v, "sessionId")))
                    }) {
                        let id = format!("{}:{sid}", source.id);
                        let keys: HashSet<_> = records
                            .iter()
                            .map(|r| r.request.dedup_key.as_str())
                            .collect();
                        let old: Vec<(i64, String)> = {
                            let mut q=tx.prepare("SELECT id,dedup_key FROM requests WHERE source=?1 AND session_id=?2").map_err(|e|e.to_string())?;
                            let rows = q
                                .query_map(params![source.id, id], |r| Ok((r.get(0)?, r.get(1)?)))
                                .map_err(|e| e.to_string())?;
                            rows.collect::<rusqlite::Result<_>>()
                                .map_err(|e| e.to_string())?
                        };
                        for (id, key) in old {
                            if !keys.contains(key.as_str()) {
                                tx.execute("DELETE FROM request_metrics WHERE request_id=?1", [id])
                                    .map_err(|e| e.to_string())?;
                                tx.execute("DELETE FROM usage_events WHERE request_id=?1", [id])
                                    .map_err(|e| e.to_string())?;
                                file_changed += tx
                                    .execute("DELETE FROM requests WHERE id=?1", [id])
                                    .map_err(|e| e.to_string())?;
                            }
                        }
                    }
                }
                // 快照记整份大小；追加日志只记到最后一个完整行，尾部半行下次接着读。
                let offset = if snapshot { size } else { consumed };
                save_stamp(&tx, &key, offset, time).map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())?;
                Ok(file_changed)
            })
        })();
        match result {
            Ok(n) => changed += n,
            Err(error) => errors.push(format!(
                "{}：{error}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
    Ok((changed, errors))
}

/// Zcode 每条请求自带状态：completed → done，error / cancelled → failed。
/// 其余视为进行中，但只在宽限期内算运行中；CLI 异常退出留下的旧行不能长期显示运行。

pub(crate) fn qoder_ledger(db: &mut impl Store, home: &Path, source_id: &str) -> Result<(usize, Vec<String>), String> {
    let path = crate::qoder_usage::ledger_path(home);
    let key = path.to_string_lossy().to_string();
    let stamps = db_stamps(&path)?;
    if db.write(|conn| db_unchanged(conn, &key, stamps)) {
        return Ok((0, Vec::new()));
    }
    // 读取第三方库不占本应用写锁
    let ledger = readonly(&path)?;
    let read = crate::qoder_usage::read_usage(&ledger, source_id, 0)?;
    drop(ledger);
    let changed = db.write(|conn| -> rusqlite::Result<usize> {
        let tx = conn.transaction()?;
        let changed = source_store::write(&tx, &read.records)?;
        save_db_stamps(&tx, &key, stamps)?;
        tx.commit()?;
        Ok(changed)
    })
    .map_err(|e| e.to_string())?;
    Ok((changed, read.skipped))
}


/// 会话库路径：WorkBuddy 在项目目录旁的 workbuddy.db，Qoder 在应用数据目录的 main.sqlite。
pub(crate) fn metadata_db(source: &Source, app: &Path) -> Option<PathBuf> {
    match source.platform.as_str() {
        "workbuddy" => Some(source.path.parent().unwrap_or(&source.path).join("workbuddy.db")),
        "qoder" => Some(app.join(if source.id.ends_with(":cn") {
            "com.qodercn.app.stable"
        } else {
            "com.qoder.app.stable"
        }).join("main.sqlite")),
        _ => None,
    }
}

pub(crate) fn session_metadata(db: &mut impl Store, source: &Source, app: &Path) -> Result<usize, String> {
    let Some(path) = metadata_db(source, app) else { return Ok(0) };
    if !path.exists() {
        return Ok(0);
    }
    // 会话库没变就不重读、不逐条 upsert；戳记键加前缀，避免与日志文件位点混用。
    let key = format!("meta:{}", path.to_string_lossy());
    let stamps = db_stamps(&path)?;
    if db.write(|conn| db_unchanged(conn, &key, stamps)) {
        return Ok(0);
    }
    let input = readonly(&path)?;
    let sql = if source.platform == "workbuddy" {
        "SELECT s.id,COALESCE(NULLIF(s.custom_title,''),s.title),s.model,u.used,u.size,u.updated_at,NULL
         FROM sessions s LEFT JOIN session_usage u ON u.session_id=s.id WHERE s.deleted_at IS NULL"
    } else {
        "SELECT s.session_id,s.title,s.model,NULL,NULL,u.updated_at,u.snapshot_json
         FROM chat_sessions s LEFT JOIN chat_session_context_usage u ON u.session_id=s.session_id WHERE s.deleted_at IS NULL"
    };
    let mut q = input.prepare(sql).map_err(|e| e.to_string())?;
    let rows = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(q);
    drop(input);
    db.write(|conn| -> Result<usize, String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut changed = 0;
        for (id, title, model, used, size, ts, snapshot) in rows {
            let session = format!("{}:{id}", source.id);
            if let Some(title) = title.filter(|s| !s.trim().is_empty()) {
                crate::db::upsert_session_title(&tx, &source.platform, &session, &title)
                    .map_err(|e| e.to_string())?;
            }
            let ratio = used
                .zip(size)
                .filter(|(u, s)| *u >= 0 && *s > 0 && *u <= *s)
                .map(|(u, s)| u as f64 / s as f64)
                .or_else(|| {
                    snapshot
                        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                        .and_then(|v| parse::amount(&v, "percentage"))
                });
            if let Some((ratio, ts)) = ratio.zip(ts) {
                changed += source_store::context(
                    &tx,
                    &source_store::ContextSnapshot {
                        platform: source.platform.clone(),
                        source: source.id.clone(),
                        session_id: session,
                        model,
                        ratio,
                        ts,
                    },
                )
                .map_err(|e| e.to_string())?;
            }
        }
        save_db_stamps(&tx, &key, stamps).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(changed)
    })
}

/// 来源的监听根目录：目录型来源就是目录本身，Zcode 这类单文件库取所在目录（WAL 也在里面）。
pub(crate) fn watch_root(source: &Source) -> PathBuf {
    if source.path.extension().is_some() && !source.path.is_dir() {
        source.path.parent().unwrap_or(&source.path).to_path_buf()
    } else {
        source.path.clone()
    }
}

/// 需要实时监听的本机来源根目录（含尚不存在的，监听器按存在与否自行绑定）。
pub(crate) fn collects_sessions(source: &Source) -> bool {
    source.id != "native:zcode:account" && crate::platforms::supports(&source.platform, "local_sessions")
}

pub fn watch_roots() -> Vec<PathBuf> {
    let Some(home) = home() else { return Vec::new() };
    let mut roots: Vec<PathBuf> = sources_at(&home, &app_data())
        .into_iter()
        .filter(|source| collects_sessions(source))
        .map(|source| watch_root(&source))
        .collect();
    for dir in crate::antigravity_usage::watch_dirs(&home) {
        if !roots.contains(&dir) {
            roots.push(dir);
        }
    }
    roots
}

/// 回填历史会话的项目目录：旧版本采集没有写 `sessions.cwd`，增量扫描也不会重扫
/// 未变化的库文件，所以启动时补一次。只填 cwd 仍为空的会话，不覆盖已有值。

pub fn scan_at(db: &mut impl Store, home: &Path, app: &Path) -> ScanResult {
    scan_sources(db, home, app, None)
}

pub(crate) fn path_under(path: &Path, root: &Path) -> bool {
    if path.starts_with(root) {
        return true;
    }
    if !cfg!(windows) {
        return false;
    }
    let norm = |p: &Path| {
        let s = p.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
        s.strip_prefix(r"\\?\").unwrap_or(&s).trim_end_matches('\\').to_string()
    };
    let p = norm(path);
    let r = norm(root);
    !r.is_empty() && (p == r || p.strip_prefix(&r).is_some_and(|rest| rest.starts_with('\\')))
}

pub(crate) fn scan_sources(
    db: &mut impl Store,
    home: &Path,
    app: &Path,
    only: Option<&[PathBuf]>,
) -> ScanResult {
    let mut result = ScanResult::default();
    for source in sources_at(home, app) {
        if !source.available || !collects_sessions(&source) {
            continue;
        }
        if let Some(changed) = only {
            let root = watch_root(&source);
            if !changed.iter().any(|path| path_under(path, &root)) {
                continue;
            }
        }
        let paused: bool = db.write(|conn| {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND status='paused')",
                [format!("monitor:{}", source.id)],
                |r| r.get(0),
            )
            .unwrap_or(false)
        });
        let paused = paused
            || (source.platform == "opencode"
                && db.write(|conn| {
                    conn.query_row(
                        "SELECT EXISTS(SELECT 1 FROM connections WHERE id='monitor:native:opencode:openai' AND status='paused')",
                        [],
                        |r| r.get::<_, bool>(0),
                    )
                    .unwrap_or(false)
                }));
        if paused {
            continue;
        }
        let Some(read) = crate::collectors::dispatch_native_scan(db, &source, home, app) else {
            continue;
        };
        result.files_scanned += 1;
        let mut collected = false;
        match read {
            Ok((n, errors)) => {
                collected = true;
                result.records_inserted += n;
                if !errors.is_empty() {
                    result.failed_sources.push(source.platform.clone());
                    result
                        .errors
                        .push(format!("{}：{}", source.name, errors.join("；")));
                }
            }
            Err(reason) => {
                result.failed_sources.push(source.platform.clone());
                result.errors.push(format!("{}：{reason}", source.name));
            }
        }
        if collected && source.id == "native:zcode:main" {
            let _ = db.write(|conn| crate::catalog_backfill::backfill_after_zcode_scan(conn));
        }
        match session_metadata(db, &source, app) {
            Ok(n) => result.records_inserted += n,
            Err(reason) => {
                result.failed_sources.push(source.platform.clone());
                result
                    .errors
                    .push(format!("{} 会话元数据：{reason}", source.name));
            }
        }
    }
    result
}

pub fn scan(db: &mut impl Store) -> ScanResult {
    home().map(|h| scan_at(db, &h, &app_data())).unwrap_or_default()
}

pub fn scan_changed(db: &mut impl Store, changed: &[PathBuf]) -> ScanResult {
    home()
        .map(|h| scan_sources(db, &h, &app_data(), Some(changed)))
        .unwrap_or_default()
}
