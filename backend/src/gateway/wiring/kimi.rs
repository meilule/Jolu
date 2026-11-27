use std::path::{Path, PathBuf};
use super::files::{parse_toml_document, prune_empty_table};
use super::zcode::ZcodeModel;

/*
 * 参照 magpie `internal/agent/kimi.go`。Kimi 的 config.toml 里：
 *   - `providers.<名字>`：上游供应商（base_url / api_key / type 等）；
 *   - `models."<本地名>"`：模型表——`provider` 指到上面的供应商、`model` 是发给上游的模型名、
 *     `max_context_size` 告诉 Kimi 什么时候压缩上下文；`/model` 选择器与状态栏显示的是本地名。
 * 接线 = 写 `providers.jolu` + 一组 `models."jolu/<网关模型 id>"` 表 + `default_model`。
 * 本地名带 `jolu/` 前缀只是为了断开时能按前缀找到我们写的表（用户不会起这种名字），
 * 发给上游的 `model` 用的是表里的 `model` 字段（网关可路由的 `来源/模型`），前缀不出本机。
 */

/// Kimi 的 provider 段名与模型表前缀（同 magpie 的 magpieID）。
pub(crate) const KIMI_PROVIDER: &str = "jolu";
/// 上下文不知道时写的值：Kimi 必须要一个数，超了它会压缩会话（magpie kimiContext）。
pub(crate) const KIMI_DEFAULT_CONTEXT: i64 = 128_000;

/// Kimi 的配置目录，附带是否旧版：新版 Kimi Code（TypeScript 2.x）读 `~/.kimi-code`
/// （`KIMI_CODE_HOME` 覆盖），旧版 Python kimi-cli 读 `~/.kimi`（`KIMI_SHARE_DIR` 覆盖）；
/// 新版目录存在时旧的不再被读（magpie #290）。两处都不存在时默认写到 `~/.kimi-code`
/// （首次接线时代码会建目录）；探测侧 [`crate::agent_discovery`] 的同名逻辑保持一致。
/// 与 [`zcode_dir`] 同理：接线需要「legacy」这个探测规格表没有的信息（旧版 kimi-cli
/// 不认 `capabilities` 里的 `tool_use`，写了会拒绝启动），所以单独再算一遍目录。
pub(crate) fn kimi_dir_public(home: &Path) -> (PathBuf, bool) { kimi_dir(home) }

pub(crate) fn kimi_dir(home: &Path) -> (PathBuf, bool) {
    if let Some(dir) = std::env::var_os("KIMI_CODE_HOME").filter(|value| !value.is_empty()) {
        return (PathBuf::from(dir), false);
    }
    let code = home.join(".kimi-code");
    if code.is_dir() {
        return (code, false);
    }
    if let Some(dir) = std::env::var_os("KIMI_SHARE_DIR").filter(|value| !value.is_empty()) {
        return (PathBuf::from(dir), true);
    }
    (home.join(".kimi"), true)
}

/// 读 `providers.jolu` 表（只读；状态判定用）；不存在或不是表时返回 None。
pub(crate) fn kimi_provider_read<'a>(document: &'a toml_edit::DocumentMut) -> Option<&'a toml_edit::Table> {
    document.get("providers")?.as_table()?.get(KIMI_PROVIDER)?.as_table()
}

pub(crate) fn kimi_current_base(text: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    let base = kimi_provider_read(&document)?.get("base_url")?.as_str()?.trim().to_string();
    (!base.is_empty()).then_some(base)
}

pub(crate) fn kimi_current_token(text: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    let token = kimi_provider_read(&document)?.get("api_key")?.as_str()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// 顶层 default_model 是否指向我们的模型表（`jolu/` 前缀）。
/// 文本版的「默认模型指向我们的表」判定（注册表用，避免调用方各自解析 TOML）。
pub(crate) fn kimi_default_model_ours(text: &str) -> bool {
    text.parse::<toml_edit::DocumentMut>().map(|document| kimi_default_ours(&document)).unwrap_or(false)
}

pub(crate) fn kimi_default_ours(document: &toml_edit::DocumentMut) -> bool {
    document
        .get("default_model")
        .and_then(|value| value.as_str())
        .is_some_and(|model| model.trim().starts_with(&format!("{KIMI_PROVIDER}/")))
}

/// 接线：写 providers.jolu（就地更新，保留用户自加的 custom_headers 等）+ 重建全部
/// `models."jolu/..."` 表 + 设 default_model。返回新文件内容；快照见 [`kimi_snapshot`]。
/// 参照 magpie `kimiIn` 的 writeMagpie / kimiModelTables / setDefault。
/// 与 magpie 的差异：magpie 用 stash 记住「上次写过的值」来区分用户手改（kimiOwn），
/// Jolu 第一版简化为「表里已有的 display_name / max_output_size 一律保留」——
/// 效果是用户手改不丢，代价是目录改名后旧显示名会留着，直到断开重接。
pub(crate) fn kimi_wire(
    text: &str,
    base: &str,
    token: &str,
    model: Option<&str>,
    models: &[ZcodeModel],
    legacy: bool,
) -> Result<String, String> {
    if models.is_empty() {
        return Err("网关模型清单为空；先在「模型池」里启用要提供给 Kimi 的模型".into());
    }
    let mut document = parse_toml_document(text, "config.toml")?;
    // 快照（断开时还原的原值）由 [`kimi_snapshot`] 单独算，这里只管写。
    // providers.jolu：段是表时就地更新（magpie kimiProvider：整表重写会把
    // 用户设的 custom_headers、env 以及挂在 provider 上的 default_model 冲掉，magpie #2030）；
    // jolu 键本身不是表（配置写坏）则换成表；providers 段整个不在（首次接线）才重建段。
    // 写坏的场景文件在磁盘上有备份（commit_plan 的 backup），不会丢数据
    if document.get("providers").map(|value| value.is_table()).unwrap_or(false) {
        let providers = document.get_mut("providers").and_then(|value| value.as_table_mut()).expect("上面判定过是表");
        if !providers.get(KIMI_PROVIDER).map(|value| value.is_table()).unwrap_or(false) {
            providers.insert(KIMI_PROVIDER, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let table = providers.get_mut(KIMI_PROVIDER).and_then(|value| value.as_table_mut()).expect("上一步刚换成表");
        table["type"] = toml_edit::value("kimi");
        table["base_url"] = toml_edit::value(base);
        table["api_key"] = toml_edit::value(token);
    } else {
        let mut table = toml_edit::Table::new();
        table["type"] = toml_edit::value("kimi");
        table["base_url"] = toml_edit::value(base);
        table["api_key"] = toml_edit::value(token);
        let mut providers = toml_edit::Table::new();
        providers.insert(KIMI_PROVIDER, toml_edit::Item::Table(table));
        document.insert("providers", toml_edit::Item::Table(providers));
    }
    let prefix = format!("{KIMI_PROVIDER}/");
    // 旧 `jolu/` 表里用户保留值（display_name / max_output_size）先收走——下面要整表删掉重写
    let mut kept: std::collections::HashMap<String, (Option<String>, Option<i64>)> = std::collections::HashMap::new();
    if let Some(models_table) = document.get_mut("models").and_then(|value| value.as_table_mut()) {
        for (name, item) in models_table.iter() {
            if !name.starts_with(&prefix) {
                continue;
            }
            let display = item.get("display_name").and_then(|value| value.as_str()).map(str::to_string).filter(|value| !value.is_empty());
            let output = item.get("max_output_size").and_then(|value| value.as_str()).and_then(|value| value.trim().replace('_', "").parse::<i64>().ok()).filter(|value| *value > 0);
            kept.insert(name.to_string(), (display, output));
        }
    }
    // 模型表：删掉全部旧 `jolu/` 表再重写——目录里模型改名或停用后，旧表不能残留
    if let Some(models_table) = document.get_mut("models").and_then(|value| value.as_table_mut()) {
        let ours: Vec<String> = models_table
            .iter()
            .map(|(name, _)| name.to_string())
            .filter(|name| name.starts_with(&prefix))
            .collect();
        for name in ours {
            models_table.remove(&name);
        }
    }
    // `models` 段不存在就建；是别的类型（配置写坏）时整体换成表
    if !document.get("models").map(|value| value.is_table()).unwrap_or(false) {
        document.insert("models", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let models_table = document.get_mut("models").and_then(|value| value.as_table_mut()).expect("上一步刚建成表");
    for entry in models {
        // 本地名 = `jolu/<网关模型 id>`；发给上游的 model 字段 = 网关模型 id 本身，
        // Kimi 按 model 字段发请求，网关按 `来源/模型` 选路。表键的引号交给 toml_edit 处理
        let name = format!("{prefix}{}", entry.id);
        let old = kept.get(&name).cloned().unwrap_or((None, None));
        let mut table = toml_edit::Table::new();
        table["provider"] = toml_edit::value(KIMI_PROVIDER);
        table["model"] = toml_edit::value(&entry.id);
        table["max_context_size"] = toml_edit::value(if entry.context > 0 { entry.context } else { KIMI_DEFAULT_CONTEXT });
        // /model 选择器与状态栏显示的名字：目录给了别称才写（等于 id 时 Kimi 自己会显示 id）；
        // 旧表里已有值（用户手改或上次接线写的）优先保留，见函数头注释的 magpie 差异
        if entry.name != entry.id {
            let display = old.0.unwrap_or_else(|| entry.name.clone());
            table["display_name"] = toml_edit::value(display);
        }
        // capabilities：旧版 kimi-cli 不认 tool_use（写了拒绝启动）；Jolu 目录暂无 thinking
        // 档数据，所以不写 thinking（magpie 会在模型有推理档位时加）
        let mut capabilities: Vec<String> = Vec::new();
        if entry.image {
            capabilities.push("image_in".into());
        }
        if !legacy {
            capabilities.push("tool_use".into());
        }
        if !capabilities.is_empty() {
            let mut array = toml_edit::Array::new();
            for item in &capabilities {
                array.push(item.as_str());
            }
            table["capabilities"] = toml_edit::Item::Value(toml_edit::Value::Array(array));
        }
        // 输出上限：新版 Kimi Code 会拿它发 max_completion_tokens（kimi-cli 没有这个键）。
        // 旧表里已有合法整数就保留；别的值（字符串等）按没设处理，写我们的（magpie kimiInt）
        if !legacy && entry.output > 0 {
            table["max_output_size"] = toml_edit::value(old.1.unwrap_or(entry.output));
        }
        models_table.insert(&name, toml_edit::Item::Table(table));
    }
    // default_model：UI 指定的优先（`jolu/<id>`）；其次保留仍在我们清单里的旧值；
    // 都没有（首次接线、或用户原来指着自己的供应商）就用第一个——接线就是把流量引到网关
    let chosen = model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(|model| format!("{prefix}{model}"))
        .or_else(|| {
            let current = document.get("default_model").and_then(|value| value.as_str()).map(str::trim).map(str::to_string);
            current.filter(|value| value.starts_with(&prefix) && models.iter().any(|entry| format!("{prefix}{}", entry.id) == *value))
        })
        .unwrap_or_else(|| format!("{prefix}{}", models[0].id));
    document["default_model"] = toml_edit::value(chosen);
    Ok(document.to_string())
}

/// 断开：删全部 `jolu/` 模型表与 providers.jolu；default_model 是我们的才还原成
/// 快照里的原值，且原模型表还在才写回（表没了会变成悬空引用，magpie ownModel 同款判断）。
/// 当前 default_model 已被用户改成别的（非 `jolu/`）时不动——外部意图优先。
pub(crate) fn kimi_restore(text: &str, saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut document = parse_toml_document(text, "config.toml")?;
    if let Some(models_table) = document.get_mut("models").and_then(|value| value.as_table_mut()) {
        let ours: Vec<String> = models_table
            .iter()
            .map(|(name, _)| name.to_string())
            .filter(|name| name.starts_with(&format!("{KIMI_PROVIDER}/")))
            .collect();
        for name in ours {
            models_table.remove(&name);
        }
    }
    if let Some(providers) = document.get_mut("providers").and_then(|value| value.as_table_mut()) {
        providers.remove(KIMI_PROVIDER);
    }
    if kimi_default_ours(&document) {
        let original = saved.and_then(|saved| saved.get("default_model")).and_then(|value| value.as_str()).map(str::trim).filter(|value| !value.is_empty());
        // 原来的默认模型得还有表可指（用户可能已把那张表删了）
        let exists = original
            .map(|model| document.get("models").and_then(|value| value.get(model)).is_some())
            .unwrap_or(false);
        match (original, exists) {
            (Some(model), true) => document["default_model"] = toml_edit::value(model),
            _ => {
                document.remove("default_model");
            }
        }
    }
    // 空表头一起清掉，理由同 `codex_restore`：kimi_wire 在文件原本没有这两张表时会新建。
    prune_empty_table(&mut document, "models");
    prune_empty_table(&mut document, "providers");
    Ok(document.to_string())
}

/// 快照：接线前用户自己的 default_model（是我们的值则不用存，断开时按「原来没有」处理）。
pub(crate) fn kimi_snapshot(text: &str) -> Option<serde_json::Value> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    if kimi_default_ours(&document) {
        return None;
    }
    let model = document.get("default_model").and_then(|value| value.as_str()).map(str::trim).filter(|value| !value.is_empty())?;
    Some(serde_json::json!({ "default_model": model }))
}
