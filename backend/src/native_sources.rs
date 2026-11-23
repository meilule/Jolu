//! 经核实的本机来源：只读第三方文件，记录写入本应用数据库。
use crate::{
    collector::ScanResult,
    db::Store,
    native_parse as parse,
    source_store::{self, Record},
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
fn app_data() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub fn sources_at(home: &Path, app: &Path) -> Vec<Source> {
    [
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
    .collect()
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
fn register_sources(
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

fn validate_source(source: &Source) -> Result<(), String> {
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

fn readonly(path: &Path) -> Result<Connection, String> {
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

fn files(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
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
fn stamp(path: &Path) -> Result<(i64, i64), String> {
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
fn save_stamp(conn: &Connection, key: &str, size: i64, time: i64) -> rusqlite::Result<usize> {
    conn.prepare_cached("INSERT INTO scan_state(path,offset,mtime,parser_initialized) VALUES(?1,?2,?3,1)
        ON CONFLICT(path) DO UPDATE SET offset=?2,mtime=?3,parser_initialized=1")?.execute(params![key,size,time])
}

/// 第三方 SQLite（Zcode 用量库、Qoder / WorkBuddy 会话库）的内容戳记：主库与 WAL 任一变化即视为有新数据。
fn db_stamps(path: &Path) -> Result<[(i64, i64); 2], String> {
    let wal = PathBuf::from(format!("{}-wal", path.to_string_lossy()));
    Ok([stamp(path)?, if wal.exists() { stamp(&wal)? } else { (0, 0) }])
}

fn db_unchanged(conn: &Connection, key: &str, stamps: [(i64, i64); 2]) -> bool {
    crate::db::get_scan_state(conn, key) == Some(stamps[0])
        && crate::db::get_scan_state(conn, &format!("{key}-wal")) == Some(stamps[1])
}

fn save_db_stamps(conn: &Connection, key: &str, stamps: [(i64, i64); 2]) -> rusqlite::Result<()> {
    save_stamp(conn, key, stamps[0].0, stamps[0].1)?;
    save_stamp(conn, &format!("{key}-wal"), stamps[1].0, stamps[1].1)?;
    Ok(())
}

fn json_lines(path: &Path) -> Result<Vec<Value>, String> {
    json_lines_from(path, 0).map(|(lines, _)| lines)
}

/// 从 `start` 起读取完整的 JSONL 行，返回解析结果与读到的行边界位置；
/// 没有换行的尾行仍在写入，留给下次。`.json` 快照总是整份读取。
fn json_lines_from(path: &Path, start: i64) -> Result<(Vec<Value>, i64), String> {
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
fn resume_offset(path: &Path, saved: Option<&crate::db::ScanCheckpoint>, size: i64, time: i64) -> i64 {
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

fn qoder_availability(app: &Path, source: &str) -> HashMap<String, (bool, i64)> {
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

fn scan_files(
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
            db.write(|conn| -> Result<usize, String> {
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                let mut file_changed = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
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
pub(crate) const ZCODE_RUNNING_GRACE_MS: i64 = 10 * 60 * 1_000;

/// 展示已采集的状态时，旧的运行状态也会随时间过期。
pub(crate) fn zcode_observed_state(state: Option<&str>, updated: Option<i64>, now_ms: i64) -> &'static str {
    match (state, updated) {
        (Some("running"), Some(ts)) if ts <= now_ms + 60_000 && now_ms.saturating_sub(ts) <= ZCODE_RUNNING_GRACE_MS => "running",
        (Some("done"), Some(ts)) if ts <= now_ms + 60_000 => "done",
        (Some("failed"), Some(ts)) if ts <= now_ms + 60_000 => "failed",
        _ => "recent",
    }
}

/// `session.time_updated` 比 `model_usage` 最新一行新出多少才算「有新活动」。
///
/// ZCode 只在请求结束时写 model_usage，一轮对话进行中（下一个请求在流式跑）时，
/// 最后一行还是上一条请求的 completed——单看行状态永远显示「已完成」。
/// `session.time_updated` 随会话活动实时前进（新消息、流式 part 都会写），
/// 是行数据之外唯一的运行痕迹。实测（2026-11-14）：一轮结束后 usage 行总是最后
/// 落库，`time_updated − usage` 只有 13–33ms；正在执行的会话差值持续增长。
/// 2 秒能干净分开这两种形态，不会把「请求刚结束的收尾写库」误判成新一轮。
pub(crate) const ZCODE_TURN_GAP_MS: i64 = 2_000;

pub(crate) fn zcode_session_state(status: &str, ts: i64, now_ms: i64) -> &'static str {
    match status {
        "completed" => "done",
        "error" | "cancelled" => "failed",
        _ if ts <= now_ms + 60_000 && now_ms - ts <= ZCODE_RUNNING_GRACE_MS => "running",
        _ => "unknown",
    }
}

/// ZCode `model_usage.status` → `requests.native_outcome`。
///
/// 值域是四值 `success` / `failed` / `cancelled` / `unknown`，与备份导入校验
/// （`db::metric_valid`）和用量统计的失败口径保持一致。`cancelled` 单独成一档，
/// 而不是并进 `failed`：
///
/// - 取消是用户主动中止（或客户端收尾），不是上游错误；并进失败会让「失败次数」
///   和「成功率」同时失真（2026-11-21 本机 1599 条里只有 2 条 error，却因为 3 条
///   cancelled 显示成 5 次失败）。
/// - 也不并进 `success`：没有完成的请求不能算成功。
/// - `running` / `pending` 等还没有结论的行保持 `unknown`，不猜成功。
///
/// 会话状态仍按 [`zcode_session_state`] 折叠成 done / failed / running：
/// 会话卡片只说「这一轮结束得怎么样」，不区分取消与报错。
pub(crate) fn zcode_native_outcome(status: &str) -> &'static str {
    match status {
        "completed" => "success",
        "error" => "failed",
        "cancelled" => "cancelled",
        _ => "unknown",
    }
}

/// 在行状态之上叠加会话级信号：最新一行的状态不是 running、但 `time_updated`
/// 比它新出 [`ZCODE_TURN_GAP_MS`] 且仍在宽限期内，说明上一条请求结束后会话又有了
/// 新活动（新一轮已经开跑），判为 running。CLI 异常退出时 `time_updated` 一并
/// 停走，同样被宽限期拦住，不会长期假running。
fn zcode_boost_running(
    state: &str,
    latest_row_ts: i64,
    session_updated_ms: Option<i64>,
    now_ms: i64,
) -> &'static str {
    if state == "running" {
        return "running";
    }
    match session_updated_ms {
        Some(updated)
            if updated > latest_row_ts + ZCODE_TURN_GAP_MS
                && updated <= now_ms + 60_000
                && now_ms - updated <= ZCODE_RUNNING_GRACE_MS =>
        {
            "running"
        }
        _ => match state {
            "done" => "done",
            "failed" => "failed",
            _ => "unknown",
        },
    }
}

/// Qoder 桌面客户端用量账本的一轮扫描（C-3：主库与 WAL 都没变时直接跳过）。
///
/// 原先每轮都把整张 `chat_message` 读出、逐行解析、再整批写回，账本越大越慢，
/// 而且写回期间一直占着数据库锁。现在沿用 ZCode 的内容戳记：变了才整份重建。
/// 戳记与记录在同一个事务里提交，写入失败时戳记不前进，下一轮会重读。
///
/// 返回值第二项是逐行跳过的坏行说明（见 `qoder_usage::read_usage`），计入扫描错误。
fn qoder_ledger(db: &mut impl Store, home: &Path, source_id: &str) -> Result<(usize, Vec<String>), String> {
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

fn zcode(db: &mut impl Store, source: &Source) -> Result<usize, String> {
    let key = source.path.to_string_lossy().to_string();
    let timestamp_migration = format!("{key}#zcode-started-at-v1");
    // 升级后重扫一次旧库，补上会话级状态时间。
    let activity_migration = format!("{key}#zcode-activity-time-v2");
    let stamps = db_stamps(&source.path)?;
    if db.write(|conn| db_unchanged(conn, &key, stamps)
        && crate::db::get_scan_state(conn, &timestamp_migration) == Some((1, 1))
        && crate::db::get_scan_state(conn, &activity_migration) == Some((1, 1))) {
        return Ok(0);
    }
    // 读取第三方库不占本应用写锁。
    let input = readonly(&source.path)?;
    // 旧版 ZCode 的 session 表可能没有 directory 列：探测后决定是否取它，
    // 缺列时用 NULL 占位，保持列序不变（同一次查询内不能混用两种投影）。
    let has_directory: bool = input
        .prepare("SELECT COUNT(*) FROM pragma_table_info('session') WHERE name = 'directory'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .map(|count| count > 0)
        .unwrap_or(false);
    let directory_expr = if has_directory { "s.directory" } else { "NULL" };
    // provider_id 是归因的关键端点事实（本机 model_usage 100% 有值）。旧版 Zcode 库可能没有
    // 这一列：与 directory 同样的探测方式，缺列时用 NULL 占位，列序不变。
    let has_provider: bool = input
        .prepare("SELECT COUNT(*) FROM pragma_table_info('model_usage') WHERE name = 'provider_id'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .map(|count| count > 0)
        .unwrap_or(false);
    let provider_expr = if has_provider { "u.provider_id" } else { "NULL" };
    // time_updated 是会话级运行信号（见 ZCODE_TURN_GAP_MS 的说明）：旧版库缺列时
    // 用 NULL 占位，行为退回「只看行状态」，与 directory / provider 同一套探测方式。
    let has_time_updated: bool = input
        .prepare("SELECT COUNT(*) FROM pragma_table_info('session') WHERE name = 'time_updated'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .map(|count| count > 0)
        .unwrap_or(false);
    let time_updated_expr = if has_time_updated { "s.time_updated" } else { "NULL" };
    let mut stmt = input
        .prepare(&format!(
            "SELECT u.id,u.session_id,u.model_id,u.status,u.started_at,u.completed_at,
        u.input_tokens,u.output_tokens,u.cache_read_input_tokens,u.cache_creation_input_tokens,
        u.provider_total_tokens,u.computed_total_tokens,u.reasoning_tokens,u.raw_usage_json,s.title,{directory_expr},
        {provider_expr},{time_updated_expr}
        FROM model_usage u LEFT JOIN session s ON s.id=u.session_id",
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let session: String = row.get(1)?;
            let status: String = row.get(3)?;
            // 归日时间取 started_at（第 4 列），缺失时才回退 completed_at（第 5 列）。
            //
            // 为什么不用 completed_at（改前的做法）：ZCode 自己按 started_at 归日、
            // 也按它做 30 天清理（usage.ts 的保留期删除），用完成时间会让跨本地午夜的
            // 请求落到不同日期——本机实测有 6 条，按日对账时两边分布对不上。
            // ts 同时也是趋势桶、明细排序、价格时段估算与 CSV 导出的时间；
            // 改归日会一并影响这些，所以是有意的口径统一：以「请求何时发出」为准。
            let started_at = row.get::<_, Option<i64>>(4)?;
            let completed_at = row.get::<_, Option<i64>>(5)?;
            // 两列都为空说明这行的发生时间不可知：跳过而不是编一个当前时间冒充发生时刻
            let Some(ts) = started_at.or(completed_at) else {
                return Err(rusqlite::Error::InvalidColumnType(
                    4,
                    "started_at/completed_at 均为空，无法归日".into(),
                    rusqlite::types::Type::Null,
                ));
            };
            let raw: Option<String> = row.get(13)?;
            let raw: Value = raw
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(Value::Null);
            let known = raw.is_object() && raw.as_object().is_some_and(|v| !v.is_empty());
            let number = |i| -> rusqlite::Result<Option<i64>> {
                Ok(row
                    .get::<_, Option<i64>>(i)?
                    .filter(|n| *n >= 0 && (known || *n > 0)))
            };
            let input = number(6)?;
            let output = number(7)?;
            let cache_read = number(8)?;
            let cache_write = number(9)?;
            let provider_total = row.get::<_, Option<i64>>(10)?;
            let total = provider_total.or(if known { row.get(11)? } else { None });
            let semantics = if raw.get("prompt_tokens").is_some()
                || (raw.get("inputTokens").is_some()
                    && input
                        .zip(output)
                        .zip(total)
                        .is_some_and(|((i, o), t)| i + o == t))
            {
                "includes_cache"
            } else if raw.get("cache_creation_input_tokens").is_some()
                && input
                    .zip(output)
                    .zip(cache_read)
                    .zip(cache_write)
                    .zip(total)
                    .is_some_and(|((((i, o), r), w), t)| i + o + r + w == t)
            {
                "excludes_cache"
            } else {
                "unknown"
            };
            Ok((
                Record {
                    request: crate::db::RequestRecord {
                        platform: "zcode".into(),
                        source: source.id.clone(),
                        dedup_key: id,
                        session_id: Some(format!("{}:{session}", source.id)),
                        ts,
                        model: row.get(2)?,
                        input_tokens: input,
                        output_tokens: output,
                        cache_read_tokens: cache_read,
                        cache_write_tokens: cache_write,
                        total_tokens: total,
                        effort: None,
                    },
                    input_semantics: semantics,
                    credits: None,
                    original_credits: None,
                    billable: None,
                    context_ratio: None,
                    reasoning: number(12)?,
                    tool: None,
                },
                status,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, Option<String>>(16)?
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty()),
                row.get::<_, Option<i64>>(17)?,
                // 第 7 个元素：completed_at，**只给会话活跃判定用**。
                // 归日用 `ts`（started_at），但「这条会话最后一次真正写完是什么时候」必须是
                // 完成时间——否则 23:59 开始、00:01 完成的那条会被当成「一小时前就没动静」，
                // 活跃会话误判成空闲。两个时间各有用途，不能合并。
                completed_at,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    drop(input);
    // 同一会话只需按最新一条写活动状态与标题（活动更新本身按时间只进不退）。
    // 第四个元素是会话的项目目录（ZCode 的 session.directory）；
    // 库型来源没有源文件可读 cwd，只能在这一步带出去落库，否则全归「未识别项目」。
    // 第五个元素是 session.time_updated：会话级运行信号，同一会话的所有行共享同一个值。
    // 第二个元素（entry.1）是**完成时间**：活跃判定要问「这条会话最后一次写完是什么时候」，
    // 用开始时间会把长请求或跨午夜请求误判成空闲（见 ts 上面的注释）。
    let mut sessions: HashMap<String, (&'static str, i64, Option<String>, Option<String>, Option<i64>)> = HashMap::new();
    let now_ms = chrono::Utc::now().timestamp_millis();
    for (r, status, title, directory, _, session_updated, finished_at) in &rows {
        let Some(id) = &r.request.session_id else { continue };
        // 会话行状态也按完成时间算：一行刚写完才算 done，started_at 只说明它何时开始
        let row_finished = finished_at.unwrap_or(r.request.ts);
        let state = zcode_session_state(status, row_finished, now_ms);
        let entry = sessions.entry(id.clone()).or_insert((state, row_finished, None, None, None));
        if row_finished >= entry.1 {
            entry.0 = state;
            entry.1 = row_finished;
        }
        if entry.2.is_none() {
            entry.2 = title.clone();
        }
        if entry.3.is_none() {
            entry.3 = directory.clone().filter(|d| !d.trim().is_empty());
        }
        if entry.4.is_none() {
            entry.4 = *session_updated;
        }
    }
    // 行状态只能反映「最后一条请求」，一轮进行中时它仍是上一条的 completed。
    // 会话级 time_updated 在这之后又有新活动、且仍在宽限期内，就是新一轮已经开跑。
    for entry in sessions.values_mut() {
        entry.0 = zcode_boost_running(entry.0, entry.1, entry.4, now_ms);
    }
    db.write(|conn| -> Result<usize, String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let records: Vec<Record> = rows.iter().map(|(r, _, _, _, _, _, _)| r.clone()).collect();
        let changed = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
        let mut outcome = tx.prepare_cached("UPDATE requests SET native_outcome=?1 WHERE source=?2 AND dedup_key=?3 AND native_outcome IS NOT ?1")
            .map_err(|e| e.to_string())?;
        for (r, status, _, _, _, _, _) in &rows {
            outcome.execute(params![zcode_native_outcome(status),source.id,r.request.dedup_key])
                .map_err(|e| e.to_string())?;
        }
        drop(outcome);
        // provider 落独立列，不进 platform：归因层要拿它去查 provider_config 里的端点地址，
        // 平台字段只回答「哪套工具」，两者不能合并。旧行下一次扫描自然补齐。
        let mut provider = tx
            .prepare_cached("UPDATE requests SET provider_key=?3 WHERE source=?1 AND dedup_key=?2 AND provider_key IS NOT ?3")
            .map_err(|e| e.to_string())?;
        for (r, _, _, _, provider_key, _, _) in &rows {
            provider
                .execute(params![source.id, r.request.dedup_key, provider_key])
                .map_err(|e| e.to_string())?;
        }
        drop(provider);
        for (id, (state, ts, title, cwd, session_updated)) in &sessions {
            if let Some(title) = title {
                crate::db::upsert_session_title(&tx, "zcode", id, title).map_err(|e| e.to_string())?;
            }
            if let Some(cwd) = cwd {
                crate::db::set_session_cwd(&tx, "zcode", id, cwd).map_err(|e| e.to_string())?;
            }
            // 会话活动可能发生在上一条用量记录完成之后，保存可信的最新时间。
            // 后续状态过期时沿用这个时间，确保 set_session_activity 的时间顺序检查允许更新。
            let activity_ts = session_updated
                .filter(|updated| *updated <= now_ms + 60_000)
                .map_or(*ts, |updated| (*ts).max(updated));
            crate::db::set_session_activity(&tx, "zcode", id, state, activity_ts).map_err(|e| e.to_string())?;
        }
        save_db_stamps(&tx, &key, stamps).map_err(|e| e.to_string())?;
        save_stamp(&tx, &timestamp_migration, 1, 1).map_err(|e| e.to_string())?;
        save_stamp(&tx, &activity_migration, 1, 1).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(changed)
    })
}

/// 会话库路径：WorkBuddy 在项目目录旁的 workbuddy.db，Qoder 在应用数据目录的 main.sqlite。
fn metadata_db(source: &Source, app: &Path) -> Option<PathBuf> {
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

fn session_metadata(db: &mut impl Store, source: &Source, app: &Path) -> Result<usize, String> {
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
fn watch_root(source: &Source) -> PathBuf {
    if source.path.extension().is_some() && !source.path.is_dir() {
        source.path.parent().unwrap_or(&source.path).to_path_buf()
    } else {
        source.path.clone()
    }
}

/// 需要实时监听的本机来源根目录（含尚不存在的，监听器按存在与否自行绑定）。
fn collects_sessions(source: &Source) -> bool {
    source.id != "native:zcode:account" && crate::platforms::supports(&source.platform, "local_sessions")
}

pub fn watch_roots() -> Vec<PathBuf> {
    let Some(home) = home() else { return Vec::new() };
    sources_at(&home, &app_data())
        .into_iter()
        .filter(|source| collects_sessions(source))
        .map(|source| watch_root(&source))
        .collect()
}

/// 回填历史会话的项目目录：旧版本采集没有写 `sessions.cwd`，增量扫描也不会重扫
/// 未变化的库文件，所以启动时补一次。只填 cwd 仍为空的会话，不覆盖已有值。
pub fn backfill_zcode_cwd(db: &mut impl Store, home: &Path) -> usize {
    let path = home.join(".zcode/cli/db/db.sqlite");
    if !path.exists() {
        return 0;
    }
    let Ok(input) = readonly(&path) else { return 0 };
    let has_directory: bool = input
        .prepare("SELECT COUNT(*) FROM pragma_table_info('session') WHERE name = 'directory'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .map(|count| count > 0)
        .unwrap_or(false);
    if !has_directory {
        return 0;
    }
    let Ok(mut stmt) = input.prepare(
        "SELECT id, directory FROM session WHERE directory IS NOT NULL AND TRIM(directory) <> ''",
    ) else {
        return 0;
    };
    let rows: Vec<(String, String)> = match stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
        Ok(iter) => iter.flatten().collect(),
        Err(_) => return 0,
    };
    drop(stmt);
    drop(input);
    db.write(|conn| -> Result<usize, String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut updated = 0;
        for (session_id, directory) in &rows {
            let key = format!("native:zcode:main:{session_id}");
            // 只补空值：已有真实路径的会话不覆盖
            updated += tx
                .execute(
                    "UPDATE sessions SET cwd = ?2 WHERE platform = 'zcode' AND session_id = ?1 AND (cwd IS NULL OR cwd = '')",
                    params![key, directory],
                )
                .unwrap_or(0);
            let key_ai = format!("native:zcode:ai:{session_id}");
            updated += tx
                .execute(
                    "UPDATE sessions SET cwd = ?2 WHERE platform = 'zcode' AND session_id = ?1 AND (cwd IS NULL OR cwd = '')",
                    params![key_ai, directory],
                )
                .unwrap_or(0);
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(updated)
    })
    .unwrap_or(0)
}

/// 受控回填 ZCode 的 `native_outcome='cancelled'`。
///
/// 旧版本的映射把 ZCode 的 `cancelled` 与 `error` 一起写成 `failed`（2026-11-21 之前），
/// 而已入库的行不会再被扫描修正：`zcode()` 开头用 `db_unchanged` 比对主库与 WAL 的
/// 大小 / mtime，源库没变化就整段跳过；即使重扫，`source_store::write` 也不碰
/// `native_outcome` 这一列（它只写 Token / 时间等字段），状态单独靠后面的
/// `UPDATE requests SET native_outcome=...` 补写——而那个 UPDATE 只在 zcode() 真正
/// 跑起来时执行。
///
/// 这里读 ZCode 自己的 `model_usage` 拿原始状态，只把「原来存的是 failed、原始状态
/// 明确是 cancelled」的行改成 cancelled：
///
/// - 只更新状态列，**不删任何行**：ZCode 按 30 天清理旧 `model_usage`，Jolu 必须继续
///   保留这些历史账本（`docs/plans/2026-11-21-usage-data-accuracy-plan.md` 的 G5）。
///   ZCode 库里已经没有的行自然查不到，也就不在回填范围内，历史记录原样留存。
/// - 只动 `native_outcome='failed'` 的行：`success` / `unknown` 不动，避免把新扫描
///   已经写对的行再改一次；也不把 `error` 行误改成 cancelled。
/// - 只动 `source='native:zcode:main'` 且 `platform='zcode'`：账号来源
///   （`native:zcode:account`）只有凭据文件，没有请求行。曾经以为历史上有
///   `native:zcode:ai` 前缀，查证后那是 WorkBuddy AI 的实例名（`backfill_zcode_cwd`
///   里那两行属于历史笔误），ZCode 请求只写过 `main`，这里不扩大到不存在的来源。
/// - 幂等且只跑一次：成功后往 `migration_log` 写一条 `zcode_cancelled_backfill`，之后的
///   心跳扫描直接返回 0，不必反复读源库。标记只在**源库读取成功**后写，源库临时
///   打不开时留待下次心跳重试，不会因为一次失败永久放弃。
///   用 `migration_log` 而不是 `scan_state`：后者是逐文件采集位点，`session_threads`
///   与性能脚本会遍历它，塞一个非文件键没有意义。
///
/// 读取源库走只读连接，写本应用库走 `db.write` 事务；两步分开，源库打不开时直接返回 0，
/// 不影响启动流程。返回实际改动的行数。
pub fn backfill_zcode_cancelled(db: &mut impl Store, home: &Path) -> usize {
    // 带版本号：将来若还要修别的历史口径，换一个 kind 即可再跑一次。
    const KIND: &str = "zcode_cancelled_backfill";
    let already: bool = db.write(|conn| conn
        .query_row("SELECT EXISTS(SELECT 1 FROM migration_log WHERE kind=?1)", [KIND], |r| r.get::<_, bool>(0))
        .unwrap_or(false));
    if already {
        return 0;
    }
    let path = home.join(".zcode/cli/db/db.sqlite");
    if !path.exists() {
        return 0;
    }
    let Ok(input) = readonly(&path) else { return 0 };
    // 只有原始状态是 cancelled 的请求 id 才进入回填集合；表缺失或读取失败都当成
    // 「没有」并保留标记未写状态，不能因为这一步失败去猜状态。
    let Ok(mut stmt) = input.prepare("SELECT id FROM model_usage WHERE status = 'cancelled'") else {
        return 0;
    };
    let cancelled: HashSet<String> = match stmt.query_map([], |r| r.get::<_, String>(0))
        .and_then(|iter| iter.collect()) {
        Ok(rows) => rows,
        Err(_) => return 0,
    };
    drop(stmt);
    drop(input);
    db.write(|conn| -> usize {
        let Ok(tx) = conn.transaction() else { return 0 };
        let mut updated = 0;
        if !cancelled.is_empty() {
            // 先只取本库里仍是 failed 的 ZCode 行，逐条比对源库状态；命中才改。
            let Ok(mut pending) = tx.prepare(
                "SELECT id, dedup_key FROM requests
                 WHERE platform = 'zcode' AND source = 'native:zcode:main' AND native_outcome = 'failed'",
            ) else {
                return 0;
            };
            let rows: Vec<(i64, String)> = match pending.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .and_then(|iter| iter.collect()) {
                Ok(rows) => rows,
                Err(_) => return 0,
            };
            drop(pending);
            for (id, dedup_key) in rows {
                if !cancelled.contains(&dedup_key) {
                    continue;
                }
                // 再带一次 `native_outcome='failed'` 条件，避免与并发扫描抢写时把
                // 刚写好的 success / cancelled 覆盖掉。
                let Ok(count) = tx
                    .execute(
                        "UPDATE requests SET native_outcome = 'cancelled'
                         WHERE id = ?1 AND native_outcome = 'failed'",
                        params![id],
                    ) else { return 0 };
                updated += count;
            }
        }
        // 标记与回填同事务：要么都生效，要么都不生效，下次心跳重新尝试。
        let marked = tx.execute(
            "INSERT INTO migration_log(at_ms, kind, detail) VALUES(?1, ?2, ?3)",
            params![
                chrono::Utc::now().timestamp_millis(),
                KIND,
                format!("ZCode 取消状态回填：{updated} 行从 failed 改为 cancelled（只改状态，未删行）"),
            ],
        ).is_ok();
        if !marked || tx.commit().is_err() {
            return 0;
        }
        updated
    })
}

pub fn scan_at(db: &mut impl Store, home: &Path, app: &Path) -> ScanResult {
    scan_sources(db, home, app, None)
}

fn scan_sources(db: &mut impl Store, home: &Path, app: &Path, only: Option<&[PathBuf]>) -> ScanResult {
    let mut result = ScanResult::default();
    for source in sources_at(home, app) {
        if !source.available || !collects_sessions(&source) {
            continue;
        }
        if let Some(changed) = only {
            let root = watch_root(&source);
            if !changed.iter().any(|path| path.starts_with(&root)) {
                continue;
            }
        }
        // 显式暂停的本机连接停止该来源采集；尚未添加连接时保持本地统计自动发现。
        let paused: bool = db.write(|conn| conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND status='paused')",
                [format!("monitor:{}", source.id)],
                |r| r.get(0),
            )
            .unwrap_or(false));
        let paused = paused || (source.platform=="opencode" && db.write(|conn|conn.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id='monitor:native:opencode:openai' AND status='paused')",[],|r|r.get::<_,bool>(0)).unwrap_or(false)));
        if paused {
            continue;
        }
        let read = if source.platform == "opencode" {
            // 新旧格式共存时只读 SQLite，避免迁移副本重复统计。只读连接保留 WAL 可见性。
            if source.path.is_dir() && crate::opencode::data_dir(home).join("opencode.db").exists(){continue}
            (||->Result<(usize,Vec<String>),String>{let records=if source.path.is_dir(){crate::opencode_usage::read_json_files(&source.path)?}else{let input=readonly(&source.path)?;crate::opencode_usage::read_usage(&input,crate::opencode_usage::detect_layout(&input)?,0)?};db.write(|conn|source_store::write(conn,&records)).map(|n|(n,vec![])).map_err(|e|e.to_string())})()
        } else if source.platform == "zcode" {
            zcode(db, &source).map(|n| (n, Vec::new()))
        } else if source.platform == "qoder"
            && source.id == "native:qoder:global"
            && crate::qoder_usage::ledger_path(home).is_file()
        {
            // Qoder 桌面客户端的用量账本（2026-11-23 对账查出：本机真实数据在这里，
            // `~/.qoder/projects` 不存在）。整份重建：`chat_message` 可能被客户端改写/清理，
            // 按行追加的位点对它不适用；去重靠 `(source, dedup_key)` 兜底。
            qoder_ledger(db, home, &source.id)
        } else {
            scan_files(db, &source, app)
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
        // 归因决定三：ZCode 采集**成功**后顺手把 provider_config 里的端点地址与模型清单补进
        // 已存连接（不新增定时器，也不在连接目录读取 / 每帧的读路径上写库，理由见
        // `catalog_backfill` 模块头）。规则文件按 mtime + 长度缓存，只在变化时重读；
        // 隔离验收模式下该函数是空操作（既不读真实配置，也不写连接）。
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
    home()
        .map(|h| scan_at(db, &h, &app_data()))
        .unwrap_or_default()
}

/// 写事件只落在某些来源目录时，只扫这些来源。
pub fn scan_changed(db: &mut impl Store, changed: &[PathBuf]) -> ScanResult {
    home()
        .map(|h| scan_sources(db, &h, &app_data(), Some(changed)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_subscription_names_use_prepared_account_and_preserve_existing_name() {
        let db = crate::db::open(Path::new(":memory:")).unwrap();
        let account = || crate::login_detect::NativeAccount { source_id: "native:qoder:cn", name: "Qoder 国内版", account: Some("verified@example.test".into()) };
        register_accounts(&db, "qoder", Some("前端任意名称"), vec![account()]).unwrap();
        let row = crate::connections::list(&db).unwrap().remove(0);
        assert_eq!(row.name, "verified@example.test");
        assert_eq!(row.account_email.as_deref(), Some("verified@example.test"));
        db.execute("UPDATE connections SET name='保留旧名'", []).unwrap();
        register_accounts(&db, "qoder", Some("第二次任意名称"), vec![account()]).unwrap();
        assert_eq!(crate::connections::list(&db).unwrap()[0].name, "保留旧名");
    }
    #[test]
    fn prepared_native_accounts_register_without_local_probe_and_preserve_pause() {
        let db=crate::db::open(Path::new(":memory:")).unwrap();
        let account=||crate::login_detect::NativeAccount{source_id:"native:qoder:cn",name:"Qoder 国内版",account:None};
        assert!(register_accounts(&db,"workbuddy",None,vec![account()]).is_err());
        assert!(crate::connections::list(&db).unwrap().is_empty());
        let (added,existing,_)=register_accounts(&db,"qoder",None,vec![account()]).unwrap();
        assert_eq!((added,existing),(1,0));
        crate::connections::set_paused(&db,"monitor:native:qoder:cn",true).unwrap();
        let (added,existing,_)=register_accounts(&db,"qoder",None,vec![account()]).unwrap();
        assert_eq!((added,existing),(0,1));
        assert!(crate::connections::creds_of(&db,"monitor:native:qoder:cn").unwrap().unwrap().paused);
    }
    #[test]
    fn opencode_pause_stops_collection_and_resume_is_idempotent() {
        let root=fixture_dir("opencode-paused");let data=crate::opencode::data_dir(&root);std::fs::create_dir_all(&data).unwrap();
        let source=Connection::open(data.join("opencode.db")).unwrap();source.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT); INSERT INTO message VALUES('m','s',123,'{\"role\":\"assistant\",\"tokens\":{\"input\":100,\"output\":30}}');").unwrap();
        let mut db=crate::db::open(&root.join("jolu.db")).unwrap();db.execute("INSERT INTO connections(id,platform,kind,name,label,masked,status,created_ms) VALUES('monitor:native:opencode:openai','opencode','auth','test','','','paused',0)",[]).unwrap();
        assert_eq!(scan_at(&mut db,&root,&root).records_inserted,0);
        db.execute("UPDATE connections SET status='connected'",[]).unwrap();assert_eq!(scan_at(&mut db,&root,&root).records_inserted,1);assert_eq!(scan_at(&mut db,&root,&root).records_inserted,0);
    }

    /// P0-3 场景「扫描延迟」：源库在两次扫描之间新增数据，第二次扫描要补上，
    /// 且不能重复写已经采过的行。
    ///
    /// 这是「扫描延迟」的固定样本 —— 用「先扫、再往源库插一行、再扫」模拟
    /// 采集器落后于源端的真实情况。判据是：第一次 N 条、第二次 N+1 条、
    /// 没有重复行（`dedup_key` 唯一）、第二次不重复消耗已采数据。
    #[test]
    fn a_delayed_scan_picks_up_new_rows_without_duplicating() {
        let root = fixture_dir("zcode-delay");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let db = folder.join("db.sqlite");
        let now = chrono::Utc::now().timestamp_millis();
        let mk = || -> Connection {
            let c = Connection::open(&db).unwrap();
            c.execute_batch("CREATE TABLE IF NOT EXISTS session(id TEXT PRIMARY KEY, title TEXT);
                CREATE TABLE IF NOT EXISTS model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT,
                  status TEXT, started_at INTEGER, completed_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                  cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, provider_total_tokens INTEGER,
                  computed_total_tokens INTEGER, reasoning_tokens INTEGER, raw_usage_json TEXT);").unwrap();
            c
        };
        {
            let input = mk();
            input.execute("INSERT INTO session VALUES('s1','会话')", []).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u1','s1','glm-4.7','completed',?1,?2,10,5,0,0,15,15,0,NULL)",
                params![now - 2000, now - 1000]).unwrap();
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let first = scan_at(&mut conn, &root, &root);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let count = |c: &Connection| -> i64 { c.query_row("SELECT COUNT(*) FROM requests WHERE platform='zcode'", [], |r| r.get(0)).unwrap() };
        assert_eq!(count(&conn), 1, "第一次扫描采到 1 条");

        // 源端在两次扫描之间新增一行（真实的「采集延迟」）
        {
            let input = mk();
            input.execute("INSERT INTO model_usage VALUES('u2','s1','glm-4.7','completed',?1,?2,20,8,0,0,28,28,0,NULL)",
                params![now - 500, now]).unwrap();
        }
        let second = scan_at(&mut conn, &root, &root);
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert_eq!(count(&conn), 2, "第二次扫描补上新增的那行");
        let dup: i64 = conn
            .query_row("SELECT COUNT(*) FROM (SELECT dedup_key FROM requests GROUP BY dedup_key HAVING COUNT(*) > 1)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(dup, 0, "不能有重复行");
        std::fs::remove_dir_all(&root).ok();
    }

    /// P0-3 场景「多账号并发」：同一平台的两个账号在同一时刻产生请求，
    /// 各自独立记在 `connection_id` 下，不能互相覆盖或错配。
    ///
    /// 固定样本：两个连接 + 四条请求，两个连接各两条、时间完全交错。
    /// 判据是 `source_usage` 按连接分组后各自计数正确，且没有一条记录连到错的连接。
    #[test]
    fn concurrent_accounts_keep_their_own_rows() {
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        // 两个账号（连接）
        for (id, name) in [("c-a", "账号 A"), ("c-b", "账号 B")] {
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug) VALUES(?1,'claude','auth',?2,'x','****',1,?3)",
                params![id, name, id],
            )
            .unwrap();
        }
        // 四条请求，两个账号交替、时间交错
        let entries = [
            ("u-1", "c-a", 1000),
            ("u-2", "c-b", 1001),
            ("u-3", "c-a", 1002),
            ("u-4", "c-b", 1003),
        ];
        for (key, conn_id, ts) in entries {
            conn.execute(
                "INSERT INTO requests(platform,source,dedup_key,ts,connection_id) VALUES('claude','local-session',?1,?2,?3)",
                params![key, ts, conn_id],
            )
            .unwrap();
        }
        let breakdown = crate::usage_stats::source_breakdown(&conn, Some("claude"), 0, 10_000).unwrap();
        let a = breakdown.rows.iter().find(|r| r.connection_id == "c-a").expect("账号 A 的一行");
        let b = breakdown.rows.iter().find(|r| r.connection_id == "c-b").expect("账号 B 的一行");
        assert_eq!((a.requests, b.requests), (2, 2), "两个账号各自 2 条，不能错配");
        assert_eq!(breakdown.total_requests, 4);
        assert_eq!(breakdown.unattributed_requests, 0, "没有记录被丢进未归属");
    }

    /// Qoder 真实布局（2026-11-23 对账修复 P0-3-a）：只有用量库、没有 `~/.qoder/projects`
    /// 的机器上，来源也必须可用并采到数据。
    ///
    /// 修复前：`available = path.exists()`，`path` 是 `~/.qoder/projects`。本机没有该目录，
    /// 于是整个 Qoder 来源永不启用，999 条真实用量一条都采不到。
    #[test]
    fn qoder_source_is_available_from_the_ledger_alone() {
        let root = fixture_dir("qoder-ledger");
        let home = root.join("home");
        // 只建账本，不建 projects 目录
        let ledger_path = crate::qoder_usage::ledger_path(&home);
        std::fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
        {
            let ledger = Connection::open(&ledger_path).unwrap();
            ledger.execute_batch(
                "CREATE TABLE chat_message(id TEXT, session_id TEXT, role TEXT, token_info TEXT, gmt_create INTEGER);
                 INSERT INTO chat_message VALUES('m1','s1','assistant',
                   '{\"prompt_tokens\":1000,\"completion_tokens\":50,\"cached_tokens\":800}',1700000000000);
                 INSERT INTO chat_message VALUES('m2','s1','assistant',
                   '{\"prompt_tokens\":2000,\"completion_tokens\":60,\"cached_tokens\":1500}',1700000001000);",
            ).unwrap();
        }
        let sources: Vec<_> = sources_at(&home, &home)
            .into_iter()
            .filter(|s| s.platform == "qoder" && s.id == "native:qoder:global")
            .collect();
        assert_eq!(sources.len(), 1);
        assert!(sources[0].available, "有账本就应该可用，不要求 projects 目录");

        let mut conn = crate::db::open(&root.join("jolu.db")).unwrap();
        let result = scan_at(&mut conn, &home, &home);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let (rows, input, cache): (i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(cache_read_tokens),0)
             FROM requests WHERE platform='qoder'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(rows, 2, "两条 assistant 调用各记一行");
        // 存的是**原始 prompt**（含缓存），缓存单列。新鲜输入由 `input_semantics`
        // = includes_cache 的消费方扣减，不能在这里提前扣。
        assert_eq!(input, 3000, "原始 prompt 之和 = 1000 + 2000");
        assert_eq!(cache, 2300, "缓存读单独记 = 800 + 1500");

        // 重扫不重复
        let again = scan_at(&mut conn, &home, &home);
        assert!(again.errors.is_empty(), "{:?}", again.errors);
        let rows2: i64 = conn
            .query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows2, 2, "重扫不重复写");

        std::fs::remove_dir_all(&root).ok();
    }

    /// C-3：账本没变时不重读。用「改 Jolu 库里的值、看下一轮会不会被覆盖回去」判断有没有重读——
    /// 没重读，改过的值会留着；账本一变，整份重建会把它改回源端的值。
    #[test]
    fn an_unchanged_qoder_ledger_is_not_reread() {
        let root = fixture_dir("qoder-ledger-stamp");
        let home = root.join("home");
        let ledger_path = crate::qoder_usage::ledger_path(&home);
        std::fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
        {
            let ledger = Connection::open(&ledger_path).unwrap();
            ledger.execute_batch(
                "CREATE TABLE chat_message(id TEXT, session_id TEXT, role TEXT, token_info TEXT, gmt_create INTEGER);
                 INSERT INTO chat_message VALUES('m1','s1','assistant',
                   '{\"prompt_tokens\":1000,\"completion_tokens\":50,\"cached_tokens\":800}',1700000000000);",
            ).unwrap();
        }
        let mut conn = crate::db::open(&root.join("jolu.db")).unwrap();
        let output = |c: &Connection| -> i64 {
            c.query_row("SELECT output_tokens FROM requests WHERE platform='qoder' AND dedup_key='s1:m1'", [], |r| r.get(0)).unwrap()
        };
        assert!(scan_at(&mut conn, &home, &home).errors.is_empty());
        assert_eq!(output(&conn), 50);

        // 只改 Jolu 侧，账本不动：下一轮不该重读，改过的值留着
        conn.execute("UPDATE requests SET output_tokens = 999 WHERE platform='qoder'", []).unwrap();
        assert!(scan_at(&mut conn, &home, &home).errors.is_empty());
        assert_eq!(output(&conn), 999, "账本没变，不应重读重写");

        // 账本有变化（新增一条）：整份重建，旧行也按源端的值改回来
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            let ledger = Connection::open(&ledger_path).unwrap();
            ledger.execute(
                "INSERT INTO chat_message VALUES('m2','s1','assistant',
                   '{\"prompt_tokens\":10,\"completion_tokens\":5,\"cached_tokens\":0}',1700000001000)",
                [],
            ).unwrap();
        }
        assert!(scan_at(&mut conn, &home, &home).errors.is_empty());
        assert_eq!(output(&conn), 50, "账本变了要重读，旧行回到源端的值");
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 2);
        drop(conn);
        std::fs::remove_dir_all(&root).ok();
    }

    /// C-5：账本里的坏行进入扫描错误（按来源定界），好行照常入库。
    #[test]
    fn qoder_bad_rows_surface_as_scan_errors_without_blocking_good_rows() {
        let root = fixture_dir("qoder-ledger-bad");
        let home = root.join("home");
        let ledger_path = crate::qoder_usage::ledger_path(&home);
        std::fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
        {
            let ledger = Connection::open(&ledger_path).unwrap();
            ledger.execute_batch(
                "CREATE TABLE chat_message(id TEXT, session_id TEXT, role TEXT, token_info TEXT, gmt_create INTEGER);
                 INSERT INTO chat_message VALUES('m1','s1','assistant',
                   '{\"prompt_tokens\":10,\"completion_tokens\":5,\"cached_tokens\":0}',1700000000000);
                 INSERT INTO chat_message VALUES('m2','s1','assistant','{oops',1700000001000);",
            ).unwrap();
        }
        let mut conn = crate::db::open(&root.join("jolu.db")).unwrap();
        let result = scan_at(&mut conn, &home, &home);
        assert!(result.errors.iter().any(|e| e.contains("JSON")), "{:?}", result.errors);
        assert!(result.failed_sources.contains(&"qoder".to_string()));
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1, "好行照常入库");
        drop(conn);
        std::fs::remove_dir_all(&root).ok();
    }

    /// 真机只读验证（`#[ignore]`）：用真实 `~/.qoder` 账本扫进一个**临时** Jolu 库，
    /// 确认端到端能写出正确条数与 Token 合计，且不碰用户真实 `usage.db`。
    #[test]
    #[ignore = "真实账号只读验证：读 ~/.qoder 账本，写入临时库"]
    fn real_qoder_ledger_scans_into_a_temp_db() {
        let Some(home) = crate::creds::home() else { return };
        if !crate::qoder_usage::ledger_path(&home).is_file() {
            eprintln!("跳过：真实 Qoder 账本不存在");
            return;
        }
        // 临时目录 + 空库，绝不碰用户的 usage.db
        let root = std::env::temp_dir().join(format!(
            "jolu-qoder-real-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut conn = crate::db::open(&root.join("usage.db")).unwrap();
        let result = scan_at(&mut conn, &home, &root);
        eprintln!("真实扫描：插入 {} 条，错误 {:?}", result.records_inserted, result.errors);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let (rows, input, cache, output): (i64, i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(output_tokens),0) FROM requests WHERE platform='qoder'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).unwrap();
        eprintln!("真实扫描结果：{rows} 条，原始输入（含缓存）{input}，缓存读 {cache}，新鲜输入 {}，输出 {output}", input - cache);
        assert!(rows > 0, "真实账本应能扫出记录");
        assert!(cache > 0, "缓存读应为正");
        let dup: i64 = conn.query_row(
            "SELECT COUNT(*) FROM (SELECT dedup_key FROM requests WHERE platform='qoder'
             GROUP BY dedup_key HAVING COUNT(*) > 1)", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(dup, 0, "去重键不能重复");
        std::fs::remove_dir_all(&root).ok();
    }

    /// P1「账号切换」：切换账号后重扫历史日志，不重复写、不把旧记录改归到新账号。
    ///
    /// ## 真实场景
    ///
    /// 用户在同一个工具里退出 A 账号、登录 B 账号后继续用。旧日志文件还在
    /// （`~/.claude/projects/` 里的历史 jsonl 不会因为换账号被删），而 Jolu 的
    /// 采集器仍会扫到它们。两件事必须同时成立：
    ///
    /// 1. **重扫不重复**：旧日志的 `dedup_key` 已经存在，不能因为「换账号了」再写一遍；
    /// 2. **归因不错配**：旧记录不能因为连接表里多了 B 而被改判成 B。
    ///
    /// 这条固定样本把这两件事都锁住：先扫出 A 名下的记录，再插入 B 连接、再扫一遍，
    /// 断言记录数不变、且 `connection_id` 一个都没变。
    #[test]
    fn switching_accounts_rescans_without_duplication_or_reattribution() {
        let root = fixture_dir("account-switch");
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".qoder/projects/p")).unwrap();
        let log = home.join(".qoder/projects/p/s.jsonl");
        let mut body = String::new();
        for i in 0..2 {
            let line = serde_json::json!({
                "type": "assistant",
                "timestamp": 1_700_000_000_000i64 + i * 1000,
                "sessionId": "s",
                "message": { "id": format!("m{i}"), "usage": { "input_tokens": 100, "output_tokens": 20,
                            "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0 } }
            });
            body.push_str(&line.to_string());
            body.push('\n');
        }
        std::fs::write(&log, &body).unwrap();

        let db_path = root.join("jolu.db");
        let mut conn = crate::db::open(&db_path).unwrap();
        // A 账号：注册为一条 auth 连接，并把首次扫描的记录归到它下面
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug)
             VALUES('c-a','qoder','auth','账号 A','x','****',1,'c-a')",
            [],
        ).unwrap();
        let first = scan_at(&mut conn, &home, &home);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let count = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(count(&conn), 2, "A 账号下采到 2 条");
        // 把这两条归到 A（生产里由代理计时 / 网关记账写入 connection_id）
        conn.execute("UPDATE requests SET connection_id='c-a' WHERE platform='qoder'", []).unwrap();
        let attributed_to = |c: &Connection, id: &str| -> i64 {
            c.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder' AND connection_id=?1",
                [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(attributed_to(&conn, "c-a"), 2);

        // 换账号：B 连接出现（A 保留，用户只是切了个登录）
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug)
             VALUES('c-b','qoder','auth','账号 B','y','****',2,'c-b')",
            [],
        ).unwrap();
        // 再扫一遍（等价于切换账号后的下一次扫描）
        let second = scan_at(&mut conn, &home, &home);
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert_eq!(count(&conn), 2, "重扫不能重复写");
        assert_eq!(second.records_inserted, 0, "位点已推进，不该有新插入");
        assert_eq!(attributed_to(&conn, "c-a"), 2, "旧记录仍归 A，不能被改判成 B");
        assert_eq!(attributed_to(&conn, "c-b"), 0, "B 不该凭空得到历史记录");

        // 归因视图也不该把旧会话算到 B 头上：平台内两条 auth 连接时，
        // 无端点事实的请求落「平台」档而不是猜一个
        let snapshot = crate::attribution::Snapshot::load(&conn).unwrap();
        let view = snapshot.resolve("qoder", None, None, None);
        assert_eq!(view.confidence, crate::attribution::Confidence::Platform,
            "两条 auth 连接、没有端点事实时不能猜");

        std::fs::remove_dir_all(&root).ok();
    }

    /// P1「账号切换」第二种形态：旧账号**被删掉**，新账号接管。
    ///
    /// 与上一条不同：上一条是两个账号并存（旧记录仍能归到 A）。这条是 A 真的没了，
    /// 它的 `connection_id` 成了指向不存在连接的悬空值。正确行为：
    ///
    /// - 记录本身不能丢（历史账本必须保留，AGENTS.md 明确要求）；
    /// - 归因**不能**自动改到 B —— 那些请求确实是 A 产生的，改成 B 就是伪造归属；
    /// - 展示层遇到悬空 id 要落回「平台」档，而不是显示一个已删除的连接名。
    #[test]
    fn deleting_the_old_account_keeps_history_and_does_not_hand_it_to_the_new_one() {
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        let now = chrono::Local::now().timestamp_millis();
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug)
             VALUES('c-a','qoder','auth','账号 A','x','****',1,'c-a')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO requests(platform,source,dedup_key,ts,total_tokens,total_known,connection_id)
             VALUES('qoder','native:qoder:global','s:m0',?1,120,1,'c-a')",
            params![now],
        ).unwrap();
        // 删掉 A，插入 B
        conn.execute("DELETE FROM connections WHERE id='c-a'", []).unwrap();
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug)
             VALUES('c-b','qoder','auth','账号 B','y','****',2,'c-b')",
            [],
        ).unwrap();

        // 记录仍在，且 connection_id 仍是已消失的 A（不被改写成 B）
        let (rows, owner): (i64, Option<String>) = conn.query_row(
            "SELECT COUNT(*), MAX(connection_id) FROM requests WHERE platform='qoder'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(rows, 1, "删除账号不能删历史记录");
        assert_eq!(owner.as_deref(), Some("c-a"), "归属仍是产生它的账号，不能被改写");

        // 归因视图：悬空 id 落平台档，不显示已删除的连接、也不冒充 B
        let snapshot = crate::attribution::Snapshot::load(&conn).unwrap();
        let view = snapshot.resolve("qoder", Some("c-a"), None, None);
        assert_eq!(view.confidence, crate::attribution::Confidence::Platform,
            "指向已删除连接时要落平台档");
        assert_eq!(view.connection_id, None, "不能返回不存在的连接 id");
    }

    /// P1「中断恢复」：扫描中途被强制终止（kill -9 级别）后重扫，不丢记录、不重复。
    ///
    /// ## 为什么不能只测「正常扫两遍」
    ///
    /// 正常扫两遍走的是「位点已推进 → 提前返回」。真正的中断发生在**事务未提交**时：
    /// 记录已写进事务、位点还没落库，进程被杀，SQLite 回滚整个事务。此时
    /// 数据库里既没有记录也没有位点（正确），重扫必须从头补齐 —— 但去重键还在，
    /// 所以「补齐」不会变成「翻倍」。这三件事只有真中断才测得出来。
    ///
    /// ## 怎么模拟 kill -9
    ///
    /// 起一个子进程扫同一份文件、同一份库，在事务提交前 `std::process::abort()`。
    /// 不用 `panic`：panic 会走 unwinding 并 drop 事务（等于回滚），与 SIGKILL 不同；
    /// `abort` 直接结束进程，正是「位点与记录都没落盘」的状态。
    #[test]
    fn a_scan_killed_mid_transaction_is_recovered_without_loss_or_duplication() {
        if std::env::var_os("JOLU_SCAN_CRASH_CHILD").is_some() {
            return;
        }
        let root = fixture_dir("scan-crash");
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".qoder/projects/p")).unwrap();
        let log = home.join(".qoder/projects/p/s.jsonl");
        // 三条可解析的 assistant 记录
        let mut body = String::new();
        for i in 0..3 {
            let line = serde_json::json!({
                "type": "assistant",
                "timestamp": 1_700_000_000_000i64 + i * 1000,
                "sessionId": "s",
                "message": { "id": format!("m{i}"), "usage": { "input_tokens": 10, "output_tokens": 5,
                                                            "cache_read_input_tokens": 0,
                                                            "cache_creation_input_tokens": 0 } }
            });
            body.push_str(&line.to_string());
            body.push('\n');
        }
        std::fs::write(&log, &body).unwrap();
        let db_path = root.join("jolu.db");

        // 子进程：开库、扫一份「未提交」的事务，然后 abort
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native_sources::tests::scan_crash_child", "--nocapture"])
            .env("JOLU_SCAN_CRASH_CHILD", "1")
            .env("JOLU_SCAN_CRASH_HOME", &home)
            .env("JOLU_SCAN_CRASH_DB", &db_path)
            .output()
            .unwrap();
        // abort 的退出码是非零、无正常退出码；这里只断言「确实非正常结束」
        assert!(!output.status.success(), "子进程应被中断而非正常退出：{:?}", output.status);

        // 父进程接着扫：必须补齐全部 3 条，且一条不多
        let mut conn = crate::db::open(&db_path).unwrap();
        let first = scan_at(&mut conn, &home, &home);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let count = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(count(&conn), 3, "中断后重扫要补齐全部记录，一条都不能丢");

        // 再扫一次：位点已推进，不应再写入
        let second = scan_at(&mut conn, &home, &home);
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert_eq!(count(&conn), 3, "重复扫描不能增加记录");
        assert_eq!(second.records_inserted, 0, "位点已推进，第二次不应再插入");

        std::fs::remove_dir_all(&root).ok();
    }

    /// P1「中断恢复」第二种形态：源端写到一半被 kill（文件里留下没有换行的半行）。
    ///
    /// 与上一条不同：这里**进程没崩在事务里**，而是源端文件本身不完整。真实场景是
    /// Claude Code / Codex 正在追加一条 assistant 行，写到一半进程被杀，留半行。
    /// 正确行为是「只采完整行、半行留到下次」，且下一次文件写完后补齐那一条 ——
    /// 不能因为见到半行就整份重读（会重复），也不能把半行当坏行永久跳过（会丢）。
    #[test]
    fn a_half_written_tail_is_picked_up_after_the_source_finishes_it() {
        let root = fixture_dir("scan-tail");
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".qoder/projects/p")).unwrap();
        let log = home.join(".qoder/projects/p/s.jsonl");
        let line = |id: &str| {
            serde_json::json!({
                "type": "assistant", "timestamp": 1_700_000_000_000i64, "sessionId": "s",
                "message": { "id": id, "usage": { "input_tokens": 10, "output_tokens": 5,
                            "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0 } }
            }).to_string()
        };
        let complete = format!("{}
", line("m0"));
        let second = line("m1");
        // 前半行：只写一半，没有换行
        let half = &second[..second.len() / 2];
        std::fs::write(&log, format!("{complete}{half}")).unwrap();

        let db_path = root.join("jolu.db");
        let mut conn = crate::db::open(&db_path).unwrap();
        let first = scan_at(&mut conn, &home, &home);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let count = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(count(&conn), 1, "只采到完整的那一行");

        // 源端把剩下的半行写完
        std::fs::write(&log, format!("{complete}{second}
")).unwrap();
        let second_scan = scan_at(&mut conn, &home, &home);
        assert!(second_scan.errors.is_empty(), "{:?}", second_scan.errors);
        assert_eq!(count(&conn), 2, "补齐第二行，不丢");
        let dup: i64 = conn.query_row(
            "SELECT COUNT(*) FROM (SELECT dedup_key FROM requests GROUP BY dedup_key HAVING COUNT(*) > 1)",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(dup, 0, "不能重复");

        // 再扫：无新增
        assert_eq!(scan_at(&mut conn, &home, &home).records_inserted, 0);
        std::fs::remove_dir_all(&root).ok();
    }

    /// 上一条测试的子进程体：写一批记录但不提交，然后 abort。
    #[test]
    fn scan_crash_child() {
        if std::env::var_os("JOLU_SCAN_CRASH_CHILD").is_none() {
            return;
        }
        let db_path = PathBuf::from(std::env::var_os("JOLU_SCAN_CRASH_DB").unwrap());
        let mut conn = crate::db::open(&db_path).unwrap();
        // 扫到一个「记录写了、位点还没提交」的状态：这里直接开事务写入后 abort，
        // 不 commit —— 与进程在 commit 之前被 SIGKILL 完全等价。
        let tx_state = conn.transaction().unwrap();
        tx_state.execute(
            "INSERT INTO requests(platform,source,dedup_key,ts,total_tokens,total_known)
             VALUES('qoder','native:qoder:global','s:m-crash',1,15,1)",
            [],
        ).unwrap();
        // 故意不提交
        std::mem::forget(tx_state);
        std::process::abort();
    }

    fn fixture_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "jolu-native-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn one_unusable_instance_does_not_block_registration_of_another() {
        let root = fixture_dir("register");
        let empty = root.join(".qoder/projects/p");
        let valid = root.join(".qoder-cn/projects/p");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&valid).unwrap();
        std::fs::write(valid.join("s.jsonl"),format!("{}\n",serde_json::json!({"type":"assistant","timestamp":1000,"sessionId":"s","message":{"id":"m","usage":{"credits":1.0}}}))).unwrap();
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        let sources = sources_at(&root, &root)
            .into_iter()
            .filter(|s| s.platform == "qoder" && s.available)
            .collect();
        let (added, existing, warnings) = register_sources(&conn, sources, None).unwrap();
        assert_eq!((added, existing, warnings.len()), (1, 0, 1));
        assert_eq!(
            crate::connections::list(&conn).unwrap()[0].id,
            "monitor:native:qoder:cn"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn zcode_request_status_drives_session_state_with_running_grace() {
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(zcode_session_state("completed", now, now), "done");
        assert_eq!(zcode_session_state("error", now, now), "failed");
        assert_eq!(zcode_session_state("cancelled", now, now), "failed");
        assert_eq!(zcode_session_state("running", now - 5_000, now), "running");
        assert_eq!(zcode_session_state("pending", now, now), "running");
        assert_eq!(zcode_session_state("running", now - ZCODE_RUNNING_GRACE_MS - 1, now), "unknown");
        assert_eq!(zcode_session_state("running", now + 120_000, now), "unknown", "未来时间戳不可信");
    }

    /// 请求级状态：ZCode 原生的三种终态不能折叠成两档，其余状态保持 unknown。
    /// 会话状态（`zcode_session_state`）仍然只分 done / failed，两者是不同层级的事实。
    #[test]
    fn zcode_native_outcome_keeps_cancelled_apart_from_error() {
        assert_eq!(zcode_native_outcome("completed"), "success");
        assert_eq!(zcode_native_outcome("error"), "failed");
        assert_eq!(zcode_native_outcome("cancelled"), "cancelled", "取消不是错误");
        // 未知 / 缺失状态不猜：running、pending 以及任何没见过的值都留在 unknown
        for status in ["running", "pending", "", "aborted", "COMPLETED"] {
            assert_eq!(zcode_native_outcome(status), "unknown", "{status} 不该被猜成终态");
        }
        // 会话状态维持原样：会话卡片只说这一轮结束得怎么样
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(zcode_session_state("cancelled", now, now), "failed");
    }

    /// 旧库里的 cancelled 曾被写成 failed；回填只改状态列，不删行、不动其他状态，
    /// 且写过标记后不再重复回填。
    #[test]
    fn backfill_turns_legacy_failed_cancellations_into_cancelled_without_deleting_rows() {
        let root = fixture_dir("zcode-cancelled-backfill");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch(
                "CREATE TABLE model_usage(id TEXT PRIMARY KEY, status TEXT);
                 INSERT INTO model_usage VALUES('u-cancel','cancelled'),('u-error','error');",
            ).unwrap();
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        // 模拟旧版本扫描的结果：cancelled 与 error 都写成 failed
        for (key, outcome, ts) in [
            ("u-cancel", "failed", now - 2_000),
            ("u-error", "failed", now - 3_000),
            // 已成功 / 未知的历史行不能被顺手改掉
            ("u-cancel-done", "success", now - 4_000),
            ("u-mystery", "unknown", now - 5_000),
            // 源库已按 30 天清理、只剩在 Jolu 的历史行：必须原样保留
            ("u-purged", "failed", now - 40 * 86_400_000),
        ] {
            conn.execute(
                "INSERT INTO requests(platform,source,dedup_key,ts,native_outcome)
                 VALUES('zcode','native:zcode:main',?1,?2,?3)",
                params![key, ts, outcome],
            ).unwrap();
        }
        let before: Vec<(String, String)> = conn
            .prepare("SELECT dedup_key, native_outcome FROM requests ORDER BY dedup_key").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
            .flatten().collect();

        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 1, "只改原始状态确为 cancelled 的那一行");

        let outcome = |conn: &Connection, key: &str| -> Option<String> {
            conn.query_row("SELECT native_outcome FROM requests WHERE dedup_key=?1", [key], |r| r.get(0)).unwrap()
        };
        assert_eq!(outcome(&conn, "u-cancel").as_deref(), Some("cancelled"));
        assert_eq!(outcome(&conn, "u-error").as_deref(), Some("failed"), "error 仍是失败");
        assert_eq!(outcome(&conn, "u-cancel-done").as_deref(), Some("success"), "已正确的状态不动");
        assert_eq!(outcome(&conn, "u-mystery").as_deref(), Some("unknown"));
        assert_eq!(outcome(&conn, "u-purged").as_deref(), Some("failed"), "源库已清理的历史行原样保留");

        // 行数与非状态字段都没变：回填只 UPDATE 状态列
        let after: Vec<(String, String)> = conn
            .prepare("SELECT dedup_key, native_outcome FROM requests ORDER BY dedup_key").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
            .flatten().collect();
        assert_eq!(after.len(), before.len(), "不能删除任何历史记录");
        let ts: i64 = conn.query_row("SELECT ts FROM requests WHERE dedup_key='u-cancel'", [], |r| r.get(0)).unwrap();
        assert_eq!(ts, now - 2_000, "只改状态，不动时间等其他字段");

        // 标记已写：第二次调用不再读源库、也不再产生改动
        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 0, "带版本标记，跑过就不再重复");
        assert_eq!(outcome(&conn, "u-cancel").as_deref(), Some("cancelled"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cancelled_backfill_retries_after_write_failure() {
        let root = fixture_dir("zcode-cancelled-retry");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let input = Connection::open(folder.join("db.sqlite")).unwrap();
        input.execute_batch("CREATE TABLE model_usage(id TEXT, status TEXT); INSERT INTO model_usage VALUES('u-1','cancelled');").unwrap();
        drop(input);
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        conn.execute("INSERT INTO requests(platform,source,dedup_key,ts,native_outcome) VALUES('zcode','native:zcode:main','u-1',1,'failed')", []).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_backfill BEFORE UPDATE OF native_outcome ON requests BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 0);
        let marks: i64 = conn.query_row("SELECT COUNT(*) FROM migration_log WHERE kind='zcode_cancelled_backfill'", [], |r| r.get(0)).unwrap();
        assert_eq!(marks, 0);
        conn.execute_batch("DROP TRIGGER reject_backfill;").unwrap();
        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 1);
        let outcome: String = conn.query_row("SELECT native_outcome FROM requests WHERE dedup_key='u-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(outcome, "cancelled");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// **真机回填**（`#[ignore]`，只在显式指定时跑）：对用户真实的 `usage.db` 执行一次
    /// `backfill_zcode_cancelled`，把旧版本误写成 `failed` 的 ZCode 取消请求改成 `cancelled`。
    ///
    /// 为什么做成 ignored 测试而不是 CLI 子命令：它只跑一次、需要真实的源库与主库同时存在，
    /// 不是用户会反复用的功能。放在这里能被编译与类型检查覆盖，又不会误触发。
    ///
    /// 安全护栏（测试自己断言，不靠调用方）：
    /// - **行数不变**（只 UPDATE 状态，绝不删行）
    /// - 只动 `native_outcome='failed'` 且**源库状态确为 cancelled** 的行
    /// - `success` / `unknown` 一行不动
    /// - 跑完写 `migration_log` 标记，重复调用返回 0
    ///
    /// 运行：`cargo test --manifest-path backend/Cargo.toml --lib real_zcode_cancelled_backfill -- --ignored --nocapture`
    #[test]
    #[ignore = "写用户真实数据库；只在明确要求时手动运行"]
    fn real_zcode_cancelled_backfill() {
        let home = crate::creds::home().expect("需要用户主目录");
        // `locate()` 返回（数据目录, 配置目录）——数据目录里就是 usage.db
        let (data_dir, _) = crate::service::paths::locate().expect("定位数据目录");
        let db_path = data_dir.join("usage.db");
        assert!(db_path.is_file(), "usage.db 不存在：{}", db_path.display());
        let mut conn = crate::db::open(&db_path).expect("打开 usage.db");
        let count = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='zcode'", [], |r| r.get(0)).unwrap()
        };
        let failed_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM requests WHERE platform='zcode' AND native_outcome='failed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let rows_before = count(&conn);
        eprintln!("回填前：zcode 行 {rows_before}，其中 failed {failed_before}");

        let changed = backfill_zcode_cancelled(&mut conn, &home);
        let rows_after = count(&conn);
        let failed_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM requests WHERE platform='zcode' AND native_outcome='failed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let cancelled: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM requests WHERE platform='zcode' AND native_outcome='cancelled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        eprintln!("回填后：改动 {changed} 行，failed {failed_after}，cancelled {cancelled}，总行数 {rows_after}");

        assert_eq!(rows_after, rows_before, "回填绝不能删行");
        assert_eq!(changed as i64, cancelled, "改动数应等于现在的 cancelled 总数");
        assert_eq!(failed_after + cancelled, failed_before, "failed 减少的量全部变成了 cancelled");
        // 幂等：标记已写，再跑一次没有任何改动
        assert_eq!(backfill_zcode_cancelled(&mut conn, &home), 0, "带版本标记，跑过就不再重复");
    }

    /// 源库没有 cancelled 行时也要写标记：否则每次心跳都会重读一遍源库。
    #[test]
    fn backfill_marks_done_even_when_nothing_to_fix() {
        let root = fixture_dir("zcode-cancelled-nothing");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch("CREATE TABLE model_usage(id TEXT PRIMARY KEY, status TEXT);
                INSERT INTO model_usage VALUES('u-1','completed');").unwrap();
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 0);
        // 标记确实写了：这是「不必每次心跳重读源库」的依据，而不是「没东西可改就返回 0」
        let marks: i64 = conn
            .query_row("SELECT COUNT(*) FROM migration_log WHERE kind='zcode_cancelled_backfill'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(marks, 1, "没有 cancelled 行也要落标记，否则每个心跳都会重读源库");
        // 标记存在后，即使源库出现可回填的行也不再处理：一次性迁移
        let input = Connection::open(folder.join("db.sqlite")).unwrap();
        input.execute("UPDATE model_usage SET status='cancelled' WHERE id='u-1'", []).unwrap();
        drop(input);
        conn.execute(
            "INSERT INTO requests(platform,source,dedup_key,ts,native_outcome)
             VALUES('zcode','native:zcode:main','u-1',1,'failed')",
            [],
        ).unwrap();
        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 0, "带标记后不再重复回填");
        let outcome: Option<String> = conn
            .query_row("SELECT native_outcome FROM requests WHERE dedup_key='u-1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(outcome.as_deref(), Some("failed"), "标记后不再改，留给下一次换 kind 的迁移");
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn zcode_session_time_updated_boosts_recently_active_sessions_to_running() {
        let now = chrono::Utc::now().timestamp_millis();
        let row_ts = now - 60_000;
        // 一轮结束后 time_updated 只比 usage 行新几十毫秒：不抬成 running
        assert_eq!(zcode_boost_running("done", row_ts, Some(row_ts + 33), now), "done");
        // 新一轮开跑：time_updated 比 usage 行新出 3.6 秒（实测形态），抬成 running
        assert_eq!(zcode_boost_running("done", row_ts, Some(row_ts + 3_600), now), "running");
        // 失败行的会话继续跑同样抬升
        assert_eq!(zcode_boost_running("failed", row_ts, Some(row_ts + 3_600), now), "running");
        // 已经是 running 的行不动
        assert_eq!(zcode_boost_running("running", row_ts, None, now), "running");
        // 没有会话级时间（旧版库缺列）不抬
        assert_eq!(zcode_boost_running("done", row_ts, None, now), "done");
        // 超出宽限期的旧活动（CLI 异常退出残留）不抬：20 分钟前完成、19 分钟前有活动
        let stale_row_ts = now - 20 * 60_000;
        assert_eq!(
            zcode_boost_running("done", stale_row_ts, Some(stale_row_ts + 60_000), now),
            "done"
        );
        // 未来时间戳不可信
        assert_eq!(zcode_boost_running("done", row_ts, Some(now + 120_000), now), "done");
    }
    #[test]
    fn zcode_synthetic_database_yields_real_session_states_and_outcomes() {
        let root = fixture_dir("zcode");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT);
                CREATE TABLE model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT, status TEXT,
                  started_at INTEGER, completed_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                  cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, provider_total_tokens INTEGER,
                  computed_total_tokens INTEGER, reasoning_tokens INTEGER, raw_usage_json TEXT);").unwrap();
            for (sid, title) in [("s-run", "进行中"), ("s-done", "已完成"), ("s-fail", "失败"), ("s-stale", "残留")] {
                input.execute("INSERT INTO session VALUES(?1,?2)", params![sid, title]).unwrap();
            }
            let raw = r#"{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}"#;
            let rows: [(&str, &str, &str, i64, Option<i64>); 5] = [
                ("u-run", "s-run", "running", now - 2_000, None),
                ("u-done", "s-done", "completed", now - 5_000, Some(now - 3_000)),
                ("u-fail", "s-fail", "error", now - 5_000, Some(now - 4_000)),
                // 用户取消：状态必须独立成 cancelled，不能并进 failed（2026-11-21）
                ("u-cancel", "s-fail", "cancelled", now - 4_000, Some(now - 3_500)),
                ("u-stale", "s-stale", "running", now - 30 * 60_000, None),
            ];
            for (id, sid, status, started, completed) in rows {
                input.execute("INSERT INTO model_usage VALUES(?1,?2,'glm-4.7',?3,?4,?5,100,20,0,0,120,120,5,?6)",
                    params![id, sid, status, started, completed, raw]).unwrap();
            }
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        // 模拟升级前扫描戳未变化，但旧记录采用了完成时间。
        let key = root.join(".zcode/cli/db/db.sqlite").to_string_lossy().to_string();
        conn.execute("DELETE FROM scan_state WHERE path=?1", [format!("{key}#zcode-started-at-v1")]).unwrap();
        conn.execute("UPDATE requests SET ts=?1 WHERE dedup_key='u-done'", [now - 3_000]).unwrap();
        conn.execute("INSERT INTO requests(platform,source,dedup_key,ts) VALUES('zcode','native:zcode:main','already-cleaned',1)", []).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_timestamp BEFORE UPDATE OF ts ON requests WHEN NEW.dedup_key='u-done' BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert!(!result.errors.is_empty());
        assert_eq!(crate::db::get_scan_state(&conn, &format!("{key}#zcode-started-at-v1")), None);
        conn.execute_batch("DROP TRIGGER reject_timestamp;").unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let restored: i64 = conn.query_row("SELECT ts FROM requests WHERE dedup_key='u-done'", [], |r| r.get(0)).unwrap();
        assert_eq!(restored, now - 5_000);
        let retained: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE dedup_key='already-cleaned'", [], |r| r.get(0)).unwrap();
        assert_eq!(retained, 1);
        assert_eq!(crate::db::get_scan_state(&conn, &format!("{key}#zcode-started-at-v1")), Some((1, 1)));
        let state_of = |sid: &str| -> String {
            conn.query_row("SELECT activity_state FROM sessions WHERE platform='zcode' AND session_id LIKE ?1",
                [format!("%:{sid}")], |r| r.get(0)).unwrap()
        };
        assert_eq!(state_of("s-run"), "running");
        assert_eq!(state_of("s-done"), "done");
        assert_eq!(state_of("s-fail"), "failed");
        assert_eq!(state_of("s-stale"), "unknown", "超过宽限期的进行中行不算运行");
        let outcome = |key: &str| -> String {
            conn.query_row("SELECT native_outcome FROM requests WHERE dedup_key=?1", [key], |r| r.get(0)).unwrap()
        };
        assert_eq!(outcome("u-done"), "success", "completed → success");
        assert_eq!(outcome("u-fail"), "failed", "error → failed");
        assert_eq!(outcome("u-cancel"), "cancelled", "cancelled 必须单独一档，不能折叠成 failed");
        assert_eq!(outcome("u-run"), "unknown", "还没有结论的行不猜成功");
        let live = crate::db::live_usage(&conn, "zcode", None).unwrap();
        let mut states: Vec<(String, String)> = live.active_sessions.iter()
            .map(|s| (s.title.clone(), s.state.clone())).collect();
        states.sort();
        let mut expected = vec![
            ("已完成".to_string(), "done".to_string()),
            ("失败".to_string(), "failed".to_string()),
            ("进行中".to_string(), "running".to_string()),
        ];
        expected.sort();
        assert_eq!(states, expected, "残留行不在 90 秒窗口内，其余按真实状态输出");
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn zcode_time_updated_marks_active_turns_running() {
        let root = fixture_dir("zcode-time-updated");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            // 带time_updated列的库（3.18 实测 schema）：一轮进行中时，最后一行
            // model_usage 仍是上一条请求的 completed，会话级时间戳却一直在走。
            input.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT, time_updated INTEGER);
                CREATE TABLE model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT, status TEXT,
                  started_at INTEGER, completed_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                  cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, provider_total_tokens INTEGER,
                  computed_total_tokens INTEGER, reasoning_tokens INTEGER, raw_usage_json TEXT);").unwrap();
            let raw = r#"{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}"#;
            // s-live：10 秒前请求 completed，但会话 2 秒前还在写（新一轮在跑）
            input.execute("INSERT INTO session VALUES('s-live','学习中的会话',?1)", params![now - 2_000]).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u-live','s-live','glm-4.7','completed',?1,?2,100,20,0,0,120,120,5,?3)",
                params![now - 10_000, now - 9_000, raw]).unwrap();
            // 上一条请求已经很久，新一轮仍在写。状态时间应采用会话更新时间。
            input.execute("INSERT INTO session VALUES('s-long','长会话',?1)", params![now - 1_000]).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u-long','s-long','glm-4.7','completed',?1,?2,100,20,0,0,120,120,5,?3)",
                params![now - 1_200_000, now - 1_199_000, raw]).unwrap();
            // 请求开始已久，刚刚完成时仍应出现在最近活跃列表。
            input.execute("INSERT INTO session VALUES('s-finished','刚完成的长请求',?1)", params![now - 500]).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u-finished','s-finished','glm-4.7','completed',?1,?2,100,20,0,0,120,120,5,?3)",
                params![now - 1_200_000, now - 500, raw]).unwrap();
            // s-idle：time_updated 只比 usage 行新 33ms，就是刚收尾，不算运行
            input.execute("INSERT INTO session VALUES('s-idle','刚完成的会话',?1)", params![now - 3_000 + 33]).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u-idle','s-idle','glm-4.7','completed',?1,?2,100,20,0,0,120,120,5,?3)",
                params![now - 5_000, now - 3_033, raw]).unwrap();
            // s-gone：有新活动但已超出宽限期（CLI 异常退出），不假 running
            input.execute("INSERT INTO session VALUES('s-gone','残留的会话',?1)", params![now - ZCODE_RUNNING_GRACE_MS - 60_000]).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u-gone','s-gone','glm-4.7','completed',?1,?2,100,20,0,0,120,120,5,?3)",
                params![now - ZCODE_RUNNING_GRACE_MS - 70_000, now - ZCODE_RUNNING_GRACE_MS - 61_000, raw]).unwrap();
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let state_of = |sid: &str| -> String {
            conn.query_row("SELECT activity_state FROM sessions WHERE platform='zcode' AND session_id LIKE ?1",
                [format!("%:{sid}")], |r| r.get(0)).unwrap()
        };
        assert_eq!(state_of("s-live"), "running", "行状态 completed 但会话仍在写：新一轮在跑");
        assert_eq!(state_of("s-long"), "running");
        let updated: i64 = conn.query_row(
            "SELECT activity_updated_ms FROM sessions WHERE platform='zcode' AND session_id LIKE '%:s-long'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(updated, now - 1_000);
        let listed = crate::session_threads::list(&conn, &crate::session_threads::SessionFilter::default()).unwrap();
        let long = listed.sessions.iter().find(|s| s.session_id.ends_with(":s-long")).unwrap();
        assert_eq!(long.status, "running");
        let live = crate::db::live_usage(&conn, "zcode", None).unwrap();
        let long = live.active_sessions.iter().find(|s| s.session_id.ends_with(":s-long")).unwrap();
        assert_eq!(long.state, "running");
        assert_eq!(long.last_seen_ms, now - 1_000);
        assert_eq!(long.delta_tokens, 0, "历史用量不能冒充最近增量");
        let finished = live.active_sessions.iter().find(|s| s.session_id.ends_with(":s-finished")).unwrap();
        assert_eq!(finished.state, "done");
        assert_eq!(finished.last_seen_ms, now - 500);
        assert_eq!(finished.delta_tokens, 0);
        assert!(live.active_sessions.iter().position(|s| s.session_id.ends_with(":s-finished"))
            < live.active_sessions.iter().position(|s| s.session_id.ends_with(":s-long")));
        assert_eq!(zcode_observed_state(Some("done"), Some(now + 61_000), now), "recent");
        assert_eq!(zcode_observed_state(Some("failed"), Some(now + 61_000), now), "recent");
        assert_eq!(state_of("s-idle"), "done", "收尾写库的几十毫秒差不是新一轮");
        assert_eq!(state_of("s-gone"), "done", "超宽限期的旧活动不假 running");
        std::fs::remove_dir_all(&root).unwrap();
    }
    /// provider_id 必须以独立字段（`requests.provider_key`）落库：它是连接归因的端点事实。
    /// 塞进 platform 会把「哪套工具」和「哪个端点」合并，归因层就再也分不开了。
    #[test]
    fn zcode_provider_id_lands_in_its_own_column() {
        let root = fixture_dir("zcode-provider");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch(
                "CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT);
                CREATE TABLE model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT, status TEXT,
                  started_at INTEGER, completed_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                  cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, provider_total_tokens INTEGER,
                  computed_total_tokens INTEGER, reasoning_tokens INTEGER, raw_usage_json TEXT,
                  provider_id TEXT NOT NULL DEFAULT '');",
            ).unwrap();
            input.execute("INSERT INTO session VALUES('s1','会话')", []).unwrap();
            input.execute(
                "INSERT INTO model_usage VALUES('u1','s1','GLM-5.3','completed',?1,?2,100,20,0,0,120,120,5,'{}','new-provider')",
                params![now - 5_000, now - 4_000],
            ).unwrap();
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let (platform, provider): (String, Option<String>) = conn
            .query_row("SELECT platform, provider_key FROM requests WHERE dedup_key='u1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(platform, "zcode", "平台仍是工具，provider 不并入 platform");
        assert_eq!(provider.as_deref(), Some("new-provider"));
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn gemini_checkpoint_rewind_and_new_events_keep_monotonic_cursor() {
        let root = fixture_dir("rewind");
        let folder = root.join(".gemini/tmp/project/chats");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("session-test.jsonl");
        let message = |id: &str| serde_json::json!({"id":id,"type":"gemini","timestamp":1000,"tokens":{"input":100,"output":5,"cached":80,"total":105}});
        let save = |messages: Vec<Value>| {
            std::fs::write(
                &path,
                format!(
                    "{}\n",
                    serde_json::json!({"$set":{"sessionId":"s","messages":messages}})
                ),
            )
            .unwrap()
        };
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        save(vec![message("a"), message("b")]);
        assert!(scan_at(&mut conn, &root, &root).errors.is_empty());
        let first = crate::db::live_usage(&conn, "gemini", None).unwrap().cursor;
        save(vec![message("a")]);
        assert!(scan_at(&mut conn, &root, &root).errors.is_empty());
        let second = crate::db::live_usage(&conn, "gemini", Some(first))
            .unwrap()
            .cursor;
        assert!(second > first);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        save(vec![message("a"), message("new-long-id")]);
        scan_at(&mut conn, &root, &root);
        let next = crate::db::live_usage(&conn, "gemini", Some(second)).unwrap();
        assert!(next.cursor > second);
        assert_eq!(next.delta_tokens, 25);
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn damaged_and_partial_files_do_not_block_other_sessions_or_advance_bad_checkpoint() {
        let root = fixture_dir("isolation");
        let folder = root.join(".workbuddy/projects/p");
        std::fs::create_dir_all(&folder).unwrap();
        let bad = folder.join("a-bad.jsonl");
        std::fs::write(&bad, "{bad}\n").unwrap();
        let good = folder.join("b-good.jsonl");
        let line = serde_json::json!({"id":"m","sessionId":"s","timestamp":1000,"providerData":{"usage":{"inputTokens":10,"outputTokens":5,"totalTokens":15},"rawUsage":{"prompt_tokens":10,"prompt_tokens_details":{"cached_tokens":0},"credit":0.5}}});
        std::fs::write(&good, format!("{line}\n{{\"partial\":")).unwrap();
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.records_inserted, 1);
        assert_eq!(
            crate::db::get_scan_state(&conn, &bad.to_string_lossy()),
            None
        );
        assert_eq!(scan_at(&mut conn, &root, &root).records_inserted, 0);
        crate::db::cleanup_history(&mut conn, 2000).unwrap();
        std::fs::write(&good, format!("{line}\n\n")).unwrap();
        scan_at(&mut conn, &root, &root);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn appended_session_lines_are_read_incrementally_and_legacy_stamps_rebuild_once() {
        use std::io::Write;
        let root = fixture_dir("incremental");
        // 与目录枚举得到的路径字符串一致（位点以路径字符串为键）
        let folder = root.join(".workbuddy/projects").join("p");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("s.jsonl");
        let line = |id: &str| serde_json::json!({"id":id,"sessionId":"s","timestamp":1000,"providerData":{"usage":{"inputTokens":10,"outputTokens":5,"totalTokens":15},"rawUsage":{"prompt_tokens":10,"prompt_tokens_details":{"cached_tokens":0},"credit":0.5}}}).to_string();
        let count = |conn: &Connection| conn.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get::<_, i64>(0)).unwrap();
        let key = path.to_string_lossy().to_string();
        std::fs::write(&path, format!("{}\n", line("a"))).unwrap();
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        assert_eq!(scan_at(&mut conn, &root, &root).records_inserted, 1);

        // 追加一整行和半行：只读新增的完整行，位点停在行边界，半行留给下次。
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(file, "{}\n{}", line("b"), &line("c")[..20]).unwrap();
        drop(file);
        let result = scan_at(&mut conn, &root, &root);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.records_inserted, 1);
        assert_eq!(count(&conn), 2);
        let boundary = format!("{}\n{}\n", line("a"), line("b")).len() as i64;
        assert_eq!(crate::db::get_scan_checkpoint(&conn, &key).unwrap().offset, boundary);

        // 半行写完后接着读，不重复计入前两行。
        std::fs::write(&path, format!("{}\n{}\n{}\n", line("a"), line("b"), line("c"))).unwrap();
        assert_eq!(scan_at(&mut conn, &root, &root).records_inserted, 1);
        assert_eq!(count(&conn), 3);

        // 旧版本的整份戳记（无行边界标记）与落在行中间的位点都从头重读一次，结果不变。
        for (offset, initialized) in [(std::fs::metadata(&path).unwrap().len() as i64, 0), (boundary - 3, 1)] {
            conn.execute("UPDATE scan_state SET offset=?1, mtime=0, parser_initialized=?2 WHERE path=?3", params![offset, initialized, key]).unwrap();
            let result = scan_at(&mut conn, &root, &root);
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            assert_eq!(result.records_inserted, 0);
            assert_eq!(count(&conn), 3);
            let checkpoint = crate::db::get_scan_checkpoint(&conn, &key).unwrap();
            assert!(checkpoint.parser_initialized);
            assert_eq!(checkpoint.offset, std::fs::metadata(&path).unwrap().len() as i64);
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn changed_paths_only_scan_the_sources_that_contain_them() {
        let root = fixture_dir("targeted");
        let folder = root.join(".workbuddy/projects/p");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("s.jsonl");
        std::fs::write(&path, format!("{}\n", serde_json::json!({"id":"m","sessionId":"s","timestamp":1000,"providerData":{"usage":{"inputTokens":10,"outputTokens":5,"totalTokens":15},"rawUsage":{"prompt_tokens":10,"prompt_tokens_details":{"cached_tokens":0}}}}))).unwrap();
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let elsewhere = [root.join(".claude/projects/x/y.jsonl")];
        let skipped = scan_sources(&mut conn, &root, &root, Some(&elsewhere));
        assert_eq!((skipped.files_scanned, skipped.records_inserted), (0, 0));
        let hit = scan_sources(&mut conn, &root, &root, Some(std::slice::from_ref(&path)));
        assert_eq!((hit.files_scanned, hit.records_inserted), (1, 1));
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    #[ignore = "显式只读本机来源，应用写入仅在内存数据库"]
    fn read_only_native_smoke() {
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let result = scan_at(&mut conn, &home().unwrap(), &app_data());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let mut q=conn.prepare("SELECT platform,COUNT(*),SUM(total_tokens),SUM(cache_read_tokens),SUM(cache_write_tokens),SUM(total_known) FROM requests GROUP BY platform").unwrap();
        let counts: Vec<_> = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        eprintln!("仅输出统计汇总：{counts:?}");
        drop(q);
        assert_eq!(
            scan_at(&mut conn, &home().unwrap(), &app_data()).records_inserted,
            0
        );
        for (platform, _, _, _, _, _) in &counts {
            crate::db::usage_breakdown_at(
                &conn,
                platform,
                "total",
                None,
                None,
                chrono::Utc::now().timestamp_millis(),
            )
            .unwrap();
            crate::db::trend_at(
                &conn,
                platform,
                "total",
                None,
                None,
                chrono::Utc::now().timestamp_millis(),
            )
            .unwrap();
        }
        let json = crate::db::export_backup(&conn, None, None, None).unwrap();
        let mut restored = crate::db::open(Path::new(":memory:")).unwrap();
        crate::db::import_backup(&mut restored, &json).unwrap();
        for (platform, _, _, _, _, _) in counts {
            assert_eq!(
                source_store::metrics(&conn, &platform, 0, i64::MAX, None)
                    .unwrap()
                    .credits,
                source_store::metrics(&restored, &platform, 0, i64::MAX, None)
                    .unwrap()
                    .credits
            );
        }
    }
    #[test]
    fn missing_sources_are_not_created_or_reported_connected() {
        let path = std::env::temp_dir().join(format!("missing-native-{}", std::process::id()));
        let list = sources_at(&path, &path);
        assert_eq!(list.len(), 12);
        assert!(list.iter().all(|s| !s.available));
        assert!(!path.exists());
        assert!(readonly(&path.join("missing.db")).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn account_sources_have_distinct_identities_and_are_never_session_files() {
        let path = std::env::temp_dir().join(format!("jolu-account-sources-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(path.join("Trae")).unwrap();
        std::fs::create_dir_all(path.join("Trae CN")).unwrap();
        std::fs::create_dir_all(path.join(".zcode/v2")).unwrap();
        std::fs::write(path.join(".zcode/v2/credentials.json"), "{invalid credentials}").unwrap();
        let list = sources_at(&path, &path);
        let intl = list.iter().find(|s| s.id == "native:trae:ide").unwrap();
        let cn = list.iter().find(|s| s.id == "native:trae:cn").unwrap();
        let account = list.iter().find(|s| s.id == "native:zcode:account").unwrap();
        assert!(intl.available && cn.available && account.available);
        assert_ne!(intl.path, cn.path);
        assert!(!collects_sessions(intl) && !collects_sessions(cn) && !collects_sessions(account));
        assert!(validate_source(account).is_err(), "发现文件不等于登录有效");
        assert!(std::fs::read_to_string(&account.path).unwrap().contains("invalid credentials"));
        std::fs::remove_dir_all(path).unwrap();
    }

    /// ZCode 的 session.directory 必须落库成会话 cwd，否则全部归入「未识别项目」。
    #[test]
    fn zcode_directory_becomes_session_cwd() {
        let root = fixture_dir("zcode-cwd");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT, directory TEXT);
                CREATE TABLE model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT,
                  status TEXT, started_at INTEGER, completed_at INTEGER, input_tokens INTEGER,
                  output_tokens INTEGER, cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER,
                  provider_total_tokens INTEGER, computed_total_tokens INTEGER, reasoning_tokens INTEGER,
                  raw_usage_json TEXT);").unwrap();
            input.execute(
                "INSERT INTO session(id,title,directory) VALUES('s1','修复入口','D:/develop/Jolu')",
                [],
            ).unwrap();
            input.execute(
                "INSERT INTO model_usage(id,session_id,model_id,status,started_at,completed_at,input_tokens,output_tokens)
                 VALUES('u1','s1','glm-4.6','completed',?1,?1,10,5)",
                [now - 1000],
            ).unwrap();
        }
        let mut store = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let source = Source {
            id: "native:zcode:main".into(),
            platform: "zcode".into(),
            name: "ZCode".into(),
            available: true,
            capabilities: vec![],
            reason: None,
            path: folder.join("db.sqlite"),
        };
        zcode(&mut store, &source).unwrap();
        let cwd: Option<String> = store
            .query_row("SELECT cwd FROM sessions WHERE platform='zcode' AND session_id='native:zcode:main:s1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cwd.as_deref(), Some("D:/develop/Jolu"), "ZCode 的 directory 必须落库成会话 cwd");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 跨本地午夜的请求必须按 **开始时间** 归日，不是按完成时间。
    ///
    /// 现场：ZCode 自己按 `started_at` 归日、也按它做 30 天清理；Jolu 曾经优先用
    /// `completed_at`，于是 23:59 开始、00:01 完成的请求会被记到第二天——本机实测
    /// 有 6 条这样的请求，按日对账时两边分布对不上。
    ///
    /// 这条测试用一条**固定**的跨午夜请求（本地时区 2026-03-10 23:59:30 开始、
    /// 2026-03-11 00:00:30 完成），断言落库的 `ts` 落在 3 月 10 日。
    #[test]
    fn zcode_attributes_a_cross_midnight_request_to_its_start_day() {
        use chrono::TimeZone;
        let root = fixture_dir("zcode-midnight");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        // 本地时区的固定时刻（不是「现在」）：跨午夜测试必须可复现，不能依赖跑测试的钟点
        let started = chrono::Local
            .with_ymd_and_hms(2026, 3, 10, 23, 59, 30)
            .single()
            .expect("固定本地时刻")
            .timestamp_millis();
        let completed = started + 60_000; // 一分钟后：2026-03-11 00:00:30
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT, directory TEXT);
                CREATE TABLE model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT,
                  status TEXT, started_at INTEGER, completed_at INTEGER, input_tokens INTEGER,
                  output_tokens INTEGER, cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER,
                  provider_total_tokens INTEGER, computed_total_tokens INTEGER, reasoning_tokens INTEGER,
                  raw_usage_json TEXT);").unwrap();
            input.execute("INSERT INTO session(id,title) VALUES('s1','跨午夜')", []).unwrap();
            input.execute(
                "INSERT INTO model_usage(id,session_id,model_id,status,started_at,completed_at,input_tokens,output_tokens)
                 VALUES('u1','s1','glm-4.6','completed',?1,?2,10,5)",
                rusqlite::params![started, completed],
            ).unwrap();
        }
        let mut store = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let source = Source {
            id: "native:zcode:main".into(),
            platform: "zcode".into(),
            name: "ZCode".into(),
            available: true,
            capabilities: vec![],
            reason: None,
            path: folder.join("db.sqlite"),
        };
        zcode(&mut store, &source).unwrap();
        let ts: i64 = store
            .query_row("SELECT ts FROM requests WHERE platform='zcode' AND dedup_key='u1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ts, started, "落库时间必须是开始时间（23:59:30），不是完成时间（次日 00:00:30）");
        let day = chrono::Local.timestamp_millis_opt(ts).single().unwrap().format("%Y-%m-%d").to_string();
        assert_eq!(day, "2026-03-10", "这条请求属于 3 月 10 日；按完成时间会错记到 3 月 11 日");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
