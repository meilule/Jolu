//! Antigravity CLI（`agy`）的专用接线。
//!
//! 参照 magpie `internal/agent/agy.go`：
//!   - 配置：`~/.gemini/antigravity-cli/settings.json`（没有环境变量覆盖目录）；
//!   - 选择自定义模型：`modelProvider: "gemini"` + `model: "<key>"`，
//!     `customModelsConfig.customModels[key] = { apiProvider: "API_PROVIDER_GOOGLE_GEMINI", modelName }`；
//!   - **地址与 Key 只从环境变量读**（`GOOGLE_GEMINI_BASE_URL` / `GEMINI_API_KEY`），
//!     settings.json 里没有这两个字段，也没有 `.env`。
//!
//! 因此接线分两部分：settings.json 写模型选择；同目录生成 `jolu-agy.cmd`，里面设置两个
//! 环境变量并用 `--model` 指定我们的模型。提示文案要求用户用这个脚本启动——直接运行
//! `agy` 会按它自己的 Google 后端走，不经过模型路由。
//!
//! 断开：把我们加的 `jolu/` 自定义模型删掉，`model` / `modelProvider` /
//! `customModelsConfig` 还原成接线前的快照；删除启动脚本。

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::wiring::adapters::{UnwireInput, WireEdits};
use super::wiring::WireContext;
use super::wiring_store::Applied;

/// 写进 `model` / `customModels` 的前缀。
const PREFIX: &str = "jolu/";
/// 启动脚本文件名（同配置目录）。
const LAUNCHER: &str = "jolu-agy.cmd";

pub(crate) fn config_path(home: &Path) -> PathBuf {
    home.join(".gemini").join("antigravity-cli").join("settings.json")
}

/// 启动脚本路径（测试与断开清理共用）。
pub(crate) fn launcher_path(config: &Path) -> PathBuf {
    config.parent().unwrap_or(Path::new(".")).join(LAUNCHER)
}

fn parse(text: &str) -> Result<Value, String> {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(text).map_err(|error| format!("settings.json 不是有效 JSON：{error}"))?;
    if !value.is_object() {
        return Err("settings.json 顶层不是对象".into());
    }
    Ok(value)
}

fn render(root: &Value) -> Result<String, String> {
    let mut text = serde_json::to_string_pretty(root).map_err(|error| format!("配置序列化失败：{error}"))?;
    text.push('\n');
    Ok(text)
}

/// 当前选中的模型（`model`）。
pub(crate) fn current_model(text: &str) -> Option<String> {
    parse(text).ok()?.get("model")?.as_str().map(str::to_string)
}

/// 一个键在接线前的存在性与值：`{present:true,value:…}` / `{present:false}`。
fn key_snapshot(root: &Value, key: &str) -> Value {
    match root.get(key) {
        Some(value) => json!({ "present": true, "value": value }),
        None => json!({ "present": false }),
    }
}

fn snapshot_of(root: &Value) -> Value {
    json!({
        "model": key_snapshot(root, "model"),
        "modelProvider": key_snapshot(root, "modelProvider"),
        "customModelsConfig": key_snapshot(root, "customModelsConfig"),
    })
}

fn restore_key(object: &mut Map<String, Value>, saved: &Value, key: &str) {
    let entry = saved.get(key);
    match entry.and_then(|value| value.get("present")).and_then(Value::as_bool) {
        Some(true) => {
            if let Some(value) = entry.and_then(|value| value.get("value")) {
                object.insert(key.into(), value.clone());
            }
        }
        _ => {
            object.remove(key);
        }
    }
}

fn launcher_script(base: &str, token: &str, model: &str) -> String {
    // `%*` 放在最后：用户在命令行再给 --model 时覆盖我们默认的模型（agy 取最后一个）。
    // 官方安装器默认把 agy.exe 放进 %LOCALAPPDATA%\agy\bin，补进本进程 PATH 避免未重启终端找不到命令。
    format!(
        "@echo off\r\nsetlocal\r\nif exist \"%LOCALAPPDATA%\\agy\\bin\\agy.exe\" set \"PATH=%LOCALAPPDATA%\\agy\\bin;%PATH%\"\r\nset \"GOOGLE_GEMINI_BASE_URL={base}\"\r\nset \"GEMINI_API_KEY={token}\"\r\nagy --model \"{model}\" %*\r\nendlocal\r\n"
    )
}

/// 从启动脚本里回读 `GEMINI_API_KEY`（供多实例接管检测 `foreign_marker` 使用）。
pub(crate) fn current_token(home: &Path) -> Option<String> {
    let script = std::fs::read_to_string(launcher_path(&config_path(home))).ok()?;
    for line in script.lines() {
        if let Some((_, rest)) = line.split_once("GEMINI_API_KEY=") {
            let value = rest.trim().trim_end_matches('"').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Antigravity CLI 是否已在本机安装或已有数据目录：
/// 与 Gemini `.env` 类似，首次使用（或经 CCS 托管运行）时 `~/.gemini/antigravity-cli/settings.json`
/// 可能尚未生成，只要检测到官方目录、本地数据目录或 `agy` 命令即允许直接接线创建。
pub(crate) fn installed(home: &Path) -> bool {
    let config = config_path(home);
    if config.exists() || config.parent().is_some_and(Path::is_dir) {
        return true;
    }
    if crate::antigravity_usage::discover_roots(home)
        .iter()
        .any(|root| crate::antigravity_usage::root_has_data(&root.path))
    {
        return true;
    }
    #[cfg(not(test))]
    if crate::agent_discovery::find_bin("agy", home).is_some() {
        return true;
    }
    false
}

/// 接线：写 model / modelProvider / customModels，并生成启动脚本。
pub(crate) fn wire(ctx: &WireContext<'_>, original: &str) -> Result<WireEdits, String> {
    let mut root = parse(original)?;
    let snapshot = ctx
        .prior_snapshot
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .unwrap_or_else(|| snapshot_of(&root));
    let chosen = ctx
        .chosen
        .model
        .filter(|model| !model.is_empty())
        .or_else(|| ctx.catalog.first().map(|model| model.id.as_str()))
        .ok_or("网关模型清单为空；先在「模型池」里启用要提供给 Antigravity 的模型")?
        .to_string();

    let object = root.as_object_mut().ok_or("settings.json 顶层不是对象")?;
    // customModelsConfig.customModels：保留用户自己的条目，只替换 jolu/ 前缀的。
    let config = object.entry("customModelsConfig").or_insert_with(|| json!({}));
    let config = config.as_object_mut().ok_or("customModelsConfig 不是对象，Jolu 不改写它")?;
    let models = config.entry("customModels").or_insert_with(|| json!({}));
    let models = models.as_object_mut().ok_or("customModels 不是对象，Jolu 不改写它")?;
    models.retain(|key, _| !key.starts_with(PREFIX));
    for model in ctx.catalog {
        models.insert(
            format!("{PREFIX}{}", model.id),
            json!({
                "apiProvider": "API_PROVIDER_GOOGLE_GEMINI",
                "modelName": model.id,
            }),
        );
    }
    object.insert("modelProvider".into(), json!("gemini"));
    object.insert("model".into(), json!(format!("{PREFIX}{chosen}")));

    let main = super::wiring::config_path_for("antigravity").ok_or("未找到用户目录")?;
    let script = launcher_script(&ctx.base, &ctx.token, &format!("{PREFIX}{chosen}"));
    let applied = Applied {
        base: ctx.base.clone(),
        token: Some(ctx.token.clone()),
        model: Some(format!("{PREFIX}{chosen}")),
        ..Default::default()
    };
    Ok(WireEdits {
        next: render(&root)?,
        snapshot: Some(snapshot),
        extra_files: vec![(launcher_path(&main), script.into_bytes())],
        applied,
    })
}

/// 断开：还原三个键，删除启动脚本由 `cleanup_launcher` 负责。
pub(crate) fn unwire(_agent: &str, input: &UnwireInput<'_>) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let root = parse(&input.original)?;
    let ours = root
        .get("model")
        .and_then(Value::as_str)
        .is_some_and(|model| model.starts_with(PREFIX));
    if !ours {
        return Ok(None);
    }
    let mut next = root;
    let object = next.as_object_mut().ok_or("settings.json 顶层不是对象")?;
    match input.saved.as_ref() {
        Some(saved) => {
            restore_key(object, saved, "model");
            restore_key(object, saved, "modelProvider");
            restore_key(object, saved, "customModelsConfig");
        }
        None => {
            // 没有快照的兜底：只清我们写进去的模型与选择键，用户其他键不动。
            object.remove("model");
            if object.get("modelProvider").and_then(Value::as_str) == Some("gemini") {
                object.remove("modelProvider");
            }
            if let Some(models) = object
                .get_mut("customModelsConfig")
                .and_then(Value::as_object_mut)
                .and_then(|config| config.get_mut("customModels"))
                .and_then(Value::as_object_mut)
            {
                models.retain(|key, _| !key.starts_with(PREFIX));
                if models.is_empty() {
                    object.remove("customModelsConfig");
                }
            }
        }
    }
    Ok(Some(vec![(input.path.clone(), render(&next)?.into_bytes())]))
}

/// 断开后的清理：删除同目录的启动脚本。`after_unwire` 传的是用户 home，不是配置文件路径。
pub(crate) fn cleanup_launcher(home: &Path) {
    let _ = std::fs::remove_file(launcher_path(&config_path(home)));
}

/// 状态判定（`WiredExtra::AntigravityModel` 专用）：模型选择 + 启动脚本里的地址/令牌都对。
pub(crate) fn wired(home: &Path, text: &str, port: u16, instance_short: &str, kept: Option<&str>) -> bool {
    let Ok(root) = parse(text) else { return false };
    let Some(model) = root.get("model").and_then(Value::as_str) else { return false };
    if !model.starts_with(PREFIX) || root.get("modelProvider").and_then(Value::as_str) != Some("gemini") {
        return false;
    }
    let Some(models) = root
        .get("customModelsConfig")
        .and_then(Value::as_object)
        .and_then(|config| config.get("customModels"))
        .and_then(Value::as_object)
    else {
        return false;
    };
    if !models.contains_key(model) {
        return false;
    }
    let Ok(script) = std::fs::read_to_string(launcher_path(&config_path(home))) else { return false };
    let base = format!("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:{port}");
    let own = format!("GEMINI_API_KEY={}", super::wiring::token_for("antigravity", instance_short));
    let kept_ok = kept.is_some_and(|key| !key.is_empty() && script.contains(&format!("GEMINI_API_KEY={key}")));
    script.contains(&base) && (script.contains(&own) || kept_ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::wiring::ZcodeModel;

    fn ctx(base: &str, token: &str) -> WireContext<'static> {
        // WireContext 借用了 catalog；调用方只在本测试函数内使用，生命周期足够。
        let catalog = Box::leak(vec![ZcodeModel {
            id: "zcode/glm-5.3".to_string(),
            name: "GLM-5.3".to_string(),
            context: 200_000,
            output: 32_768,
            image: true,
        }].into_boxed_slice());
        WireContext {
            agent: "antigravity",
            base: base.into(),
            token: token.into(),
            instance_short: "abcd1234",
            catalog,
            prior_snapshot: None,
            chosen: super::super::wiring::ModelChoice {
                model: None,
                label: None,
                description: None,
                prior_model: None,
                prior_label: None,
                prior_values: &[],
                subagent_model: None,
                tiers: None,
            },
            claude: None,
        }
    }

    #[test]
    fn wire_keeps_user_models_and_restores_them() {
        let original = r#"{"model":"mine","modelProvider":"vertex","customModelsConfig":{"customModels":{"mine":{"apiProvider":"API_PROVIDER_GOOGLE_GEMINI","modelName":"gemini-3-pro"}}},"keep":1}"#;
        let plan = wire(&ctx("http://127.0.0.1:12731", "jolu-antigravity.abcd1234"), original).unwrap();
        let root: Value = serde_json::from_str(&plan.next).unwrap();
        assert_eq!(root["model"], "jolu/zcode/glm-5.3");
        assert_eq!(root["modelProvider"], "gemini");
        assert_eq!(root["customModelsConfig"]["customModels"]["mine"]["modelName"], "gemini-3-pro");
        assert!(root["customModelsConfig"]["customModels"]["jolu/zcode/glm-5.3"].is_object());
        assert_eq!(root["keep"], 1);
        let script = String::from_utf8(plan.extra_files[0].1.clone()).unwrap();
        assert!(script.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:12731"));
        assert!(script.contains("agy --model \"jolu/zcode/glm-5.3\""));

        let input = UnwireInput {
            path: PathBuf::from("settings.json"),
            original: plan.next.clone(),
            saved: plan.snapshot.clone(),
            port: 12731,
            record: None,
        };
        let restored = String::from_utf8(unwire("antigravity", &input).unwrap().unwrap()[0].1.clone()).unwrap();
        let before: Value = serde_json::from_str(original).unwrap();
        let after: Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(before, after, "断开应还原用户原配置：{restored}");
    }

    #[test]
    fn wired_requires_launcher_and_model_selection() {
        let dir = std::env::temp_dir().join(format!("jolu-agy-wired-{}", std::process::id()));
        let config = config_path(&dir.join("home"));
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        let text = r#"{"model":"jolu/zcode/glm-5.3","modelProvider":"gemini","customModelsConfig":{"customModels":{"jolu/zcode/glm-5.3":{"apiProvider":"API_PROVIDER_GOOGLE_GEMINI","modelName":"zcode/glm-5.3"}}}}"#;
        assert!(!wired(&dir.join("home"), text, 12731, "abcd1234", None));
        std::fs::write(launcher_path(&config), "set \"GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:12731\"\r\nset \"GEMINI_API_KEY=jolu-antigravity.abcd1234\"\r\n").unwrap();
        assert!(wired(&dir.join("home"), text, 12731, "abcd1234", None));
        assert_eq!(current_token(&dir.join("home")).as_deref(), Some("jolu-antigravity.abcd1234"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installed_recognises_ccs_or_default_data_roots_without_settings_json() {
        let dir = std::env::temp_dir().join(format!(
            "jolu-agy-installed-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        assert!(!installed(&home), "空 home 不应判已安装");

        let ccs = home.join(".cc-switch").join("agy").join("p1").join("antigravity-cli").join("conversations");
        std::fs::create_dir_all(&ccs).unwrap();
        std::fs::write(ccs.join("c1.db"), b"").unwrap();
        assert!(installed(&home), "仅有 CCS 数据目录也应允许直接接线");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
