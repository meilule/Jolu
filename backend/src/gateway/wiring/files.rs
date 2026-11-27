use std::path::{Path, PathBuf};

/// TOML 解析的公共入口：空文件按空文档处理（首次接线还没生成 config.toml）。
pub(crate) fn parse_toml_document(text: &str, what: &str) -> Result<toml_edit::DocumentMut, String> {
    if text.trim().is_empty() {
        return Ok(toml_edit::DocumentMut::new());
    }
    text.parse::<toml_edit::DocumentMut>().map_err(|error| format!("{what} 格式无效：{error}"))
}

/// 删掉一个已经空掉的顶层表（只删表头本身，不留 `[name]` 空壳）。
///
/// 为什么需要：`codex_wire` / `kimi_wire` 在文件原本没有目标表时会**新建**一张，
/// 断开时只删我们写进去的子项，表就空着了。用户看到的是一段凭空多出来的
/// `[model_providers]` / `[models]`——语义无害但明显是「没还原干净」。
/// 只处理顶层表：子表（`[a.b]`）由它自己的写入方负责。
pub(crate) fn prune_empty_table(document: &mut toml_edit::DocumentMut, key: &str) {
    let empty = document
        .get(key)
        .and_then(|item| item.as_table())
        .is_some_and(|table| table.is_empty());
    if empty {
        document.remove(key);
    }
}

/// dotenv 键值写入：保留其它行与注释，只替换目标键（不存在则追加）。
pub(crate) fn wire_dotenv(text: &str, key: &str, value: Option<&str>) -> String {
    let prefix = format!("{key}=");
    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in text.lines() {
        if line.trim_start().starts_with(&prefix) {
            if let Some(value) = value {
                out.push(format!("{prefix}{value}"));
            }
            replaced = true;
            continue;
        }
        out.push(line.to_string());
    }
    if !replaced {
        if let Some(value) = value {
            out.push(format!("{prefix}{value}"));
        }
    }
    let mut joined = out.join("\n");
    if !joined.is_empty() && !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

pub(crate) fn dotenv_get(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{key}=")))
        .map(|v| v.trim().trim_matches('"').to_string())
}

/// 判断当前配置值是否指向本机网关端口（用于「外部已改就不覆盖」）。
pub(crate) fn points_at_gateway(value: &str, port: u16) -> bool {
    let value = value.trim().trim_end_matches('/');
    value.starts_with(&format!("http://127.0.0.1:{port}"))
        || value.starts_with(&format!("http://localhost:{port}"))
}

/// `jolu-<agent>.<实例前 8 位>`：一键接线的归属标记，独立于可刷新的手动 Key。
pub(crate) fn token_for(agent: &str, instance_short: &str) -> String {
    format!("jolu-{agent}.{instance_short}")
}

/// 各 agent 的配置文件路径（探测与写入共用）。
/// library（批 5）与卸载钩子用的公开包装：同 [`config_path`]。
pub fn config_path_for(agent: &str) -> Option<PathBuf> {
    config_path(agent)
}

/// library（批 5）用的公开原子写：临时文件 + rename（同 [`write_atomic`]）。
pub fn write_json_atomic_public(path: &Path, bytes: Vec<u8>) -> Result<(), String> {
    write_atomic(path, bytes)
}

/// 配置路径：查注册表（一家一行声明在 `wiring_specs::SPECS`）。
pub(crate) fn config_path(agent: &str) -> Option<PathBuf> {
    let home = crate::creds::home()?;
    (super::specs::spec(agent)?.path)(&home)
}

pub(crate) fn write_atomic(path: &Path, bytes: Vec<u8>) -> Result<(), String> {
    use std::io::Write;
    let temporary = path.with_extension("jolu-wiring.tmp");
    let previous = crate::steady::read(path).ok();
    let write = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    };
    write().map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("写入 {} 失败：{error}", path.display())
    })?;
    // 2.10 写后校验：读回来要与写入的一致、且仍是合法的 JSON / TOML；不对就放回原内容并报错。
    // 为什么：rename 成功不代表 agent 读到的是我们写的（同时有别的程序改写、杀毒软件拦截等），
    // 写坏的配置会让工具整个起不来，必须当场发现
    if let Err(reason) = verify_written(path, &bytes) {
        match previous {
            Some(previous) => {
                let _ = std::fs::write(path, previous);
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
        return Err(format!("写入 {} 后校验失败，已放回原内容：{reason}", path.display()));
    }
    Ok(())
}

/// 读回刚写的文件：内容要一致，按扩展名重新解析一遍。
pub(crate) fn verify_written(path: &Path, expected: &[u8]) -> Result<(), String> {
    let actual = crate::steady::read(path).map_err(|error| format!("读回失败：{error}"))?;
    if actual != expected {
        return Err("读回的内容与写入的不一致（可能有别的程序同时改写）".into());
    }
    let text = String::from_utf8_lossy(&actual);
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") if !text.trim().is_empty() => {
            serde_json::from_str::<serde_json::Value>(&text).map_err(|error| format!("不是有效 JSON：{error}"))?;
        }
        Some("toml") => {
            text.parse::<toml_edit::DocumentMut>().map_err(|error| format!("不是有效 TOML：{error}"))?;
        }
        _ => {}
    }
    Ok(())
}

/// 写前备份到应用数据目录（沿用 cli_apply 的纪律：备份失败则不写）。
pub(crate) fn backup(path: &Path, backup_dir: &Path) -> Result<(), String> {
    if !crate::steady::exists(&path) {
        return Ok(());
    }
    std::fs::create_dir_all(backup_dir).map_err(|error| format!("创建备份目录失败：{error}"))?;
    let name = format!(
        "{}-{}.bak",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("config"),
        chrono::Local::now().format("%Y%m%d%H%M%S")
    );
    std::fs::copy(path, backup_dir.join(name)).map_err(|error| format!("备份 {path:?} 失败：{error}"))?;
    Ok(())
}

/// 别的 Jolu 实例写下的标记：token 的实例后缀与当前实例不同。
pub(crate) fn foreign_marker(agent: &str, text: &str, instance_short: &str) -> Option<String> {
    let spec = super::specs::spec(agent)?;
    let home = crate::creds::home()?;
    let token = (spec.token)(&home, text)?;
    if !token.starts_with("jolu-") {
        return None;
    }
    let (_, suffix) = token.rsplit_once('.')?;
    (suffix != instance_short).then(|| suffix.to_string())
}

#[cfg(test)]
#[cfg(test)]
#[cfg(test)]
/// 当前配置是否由本实例写入（状态判断用）。
pub(crate) fn ours(agent: &str, text: &str, port: u16, instance_short: &str) -> bool {
    ours_with(agent, text, port, instance_short, None)
}

/// `kept` 是接线时保留下来的调用方 Key（keyHolds）：文件里是它也算我们的接线。
pub(crate) fn ours_with(agent: &str, text: &str, port: u16, instance_short: &str, kept: Option<&str>) -> bool {
    let Some(spec) = super::specs::spec(agent) else { return false };
    let Some(home) = crate::creds::home() else { return false };
    spec.is_wired(&home, text, port, instance_short, kept)
}

/// 探测某 agent 的当前接入状态（只看文件）。

pub(crate) fn write_all_with_rollback(files: &[(PathBuf, Vec<u8>)]) -> Result<(), String> {
    let originals: Vec<Option<Vec<u8>>> = files.iter().map(|(path, _)| crate::steady::read(path).ok()).collect();
    for (index, (path, bytes)) in files.iter().enumerate() {
        if let Err(error) = write_atomic(path, bytes.clone()) {
            for (done, (done_path, _)) in files[..index].iter().enumerate() {
                match &originals[done] {
                    Some(previous) => {
                        let _ = write_atomic(done_path, previous.clone());
                    }
                    None => {
                        let _ = std::fs::remove_file(done_path);
                    }
                }
            }
            return Err(error);
        }
    }
    Ok(())
}
