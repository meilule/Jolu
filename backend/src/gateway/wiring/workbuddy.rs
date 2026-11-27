//! WorkBuddy 接线（参照 magpie `internal/agent/workbuddy.go`）。
//!
//! WorkBuddy（腾讯 CodeBuddy 桌面版）的自定义模型在 `~/.workbuddy/models.json`
//! （`WORKBUDDY_CONFIG_DIR` 覆盖）。文件有**两种形状**：
//!
//! ```json
//! [ {"id":…, "name":…, "vendor":…, "apiKey":…, "url":…, …} ]          // 裸数组（WorkBuddy 自己写的）
//! {"models":[…], "availableModels":[…]},                              // CodeBuddy Code CLI 写的
//! ```
//!
//! 两种都要认：读的时候把 `models` 取出来，写的时候按原形状写回。裸数组写成裸数组，
//! 带包装的保留 `availableModels` 等其它键——用户手改的东西一行不动。
//!
//! WorkBuddy 在模型选择器里显示成 `custom-local:<id>`，请求时按 `url` 带 `Bearer apiKey`
//! 发。**模型是按任务在它的窗口里选的，不在文件里**，所以 Jolu 能做的只是「让我们的模型
//! 出现在那个选择器里」，不能替用户选中——这一点和 Qoder / Pencil 一样。
//!
//! magpie 还提醒：5.6 的 app.asar **只在启动时**监视这个文件，且监视的是文件本身，
//! 重写后可能要重启 WorkBuddy 才生效。
//!
//! 与 magpie 的差异：不实现跨机器的 `remote` 地址保留（Jolu 没有多机场景）；
//! 不写 `reasoning` 档位（网关目录暂无按工具的能力数据）。

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::ZcodeModel;

/// 我们在 `vendor` 字段里的标识：`models.json` 里 `vendor == "jolu"` 的条目是我们写的。
///
/// magpie 用 `magpie`。用 vendor 而不是 apiKey 判定，是因为 WorkBuddy 的模型设置面板
/// 会按供应商重写 `vendor`（编辑过的条目会变成 `"Custom"`），但**不会改 apiKey**——
/// 所以判定要两条都看，见 `is_ours`。
pub(crate) const WORKBUDDY_VENDOR: &str = "jolu";

/// 我们令牌的前缀：`token_for("workbuddy", instance)` = `jolu-workbuddy.<实例8位>`。
///
/// 单独抽出来是因为它**不等于** `"jolu-{WORKBUDDY_VENDOR}."`（那会拼成 `jolu-jolu.`，
/// 曾经的真 bug：所有判定都返回 false，接入后状态显示「未接入」）。
pub(crate) const WORKBUDDY_TOKEN_PREFIX: &str = "jolu-workbuddy.";

pub(crate) fn workbuddy_dir(home: &Path) -> PathBuf {
    std::env::var_os("WORKBUDDY_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".workbuddy"))
}

pub(crate) fn config_path(home: &Path) -> PathBuf {
    workbuddy_dir(home).join("models.json")
}

/// 读出来的 models.json：模型列表 + 可能存在的包装对象（裸数组时为 `None`）。
struct Document {
    models: Vec<Value>,
    /// `{"models":[…], …}` 形状时保留其余键（`availableModels` 等）；裸数组为 `None`
    rest: Option<Map<String, Value>>,
}

fn read(text: &str) -> Result<Document, String> {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok(Document { models: Vec::new(), rest: None });
    }
    let value: Value = serde_json::from_str(text).map_err(|error| format!("models.json 不是有效 JSON：{error}"))?;
    match value {
        Value::Array(models) => Ok(Document { models, rest: None }),
        Value::Object(mut object) => {
            let models = object
                .remove("models")
                .map(|value| match value {
                    Value::Array(models) => Ok(models),
                    _ => Err("models.json 的 models 不是数组".to_string()),
                })
                .transpose()?
                .unwrap_or_default();
            Ok(Document { models, rest: Some(object) })
        }
        _ => Err("models.json 顶层既不是数组也不是对象".into()),
    }
}

fn render(document: Document) -> Result<String, String> {
    let value = match document.rest {
        Some(mut rest) => {
            rest.insert("models".into(), Value::Array(document.models));
            Value::Object(rest)
        }
        None => Value::Array(document.models),
    };
    let mut text = serde_json::to_string_pretty(&value).map_err(|error| format!("models.json 序列化失败：{error}"))?;
    text.push('\n');
    Ok(text)
}

/// 一条模型是否 Jolu 写的。
///
/// **只看密钥**，不看 `vendor`：
/// - `apiKey` 精确等于本实例的令牌，或
/// - `apiKey` 以 `jolu-workbuddy.` 开头（`current_base` / `current_token` 拿不到精确
///   令牌时用这个前缀兜底；用户不会给密钥起这种名字）
///
/// 为什么不认 `vendor == "jolu"`：用户完全可能自己建一条同名 vendor 的模型（密钥是他
/// 自己的），按 vendor 认会把人家删掉。magpie 同样按密钥集合认（`keys[e.APIKey]`）。
/// 反过来，用户在 WorkBuddy 的模型设置里编辑过我们的条目后 `vendor` 会被改成 `"Custom"`
/// ——只有密钥这个判据在两边都成立。
fn is_ours(entry: &Value, token: &str) -> bool {
    let key = entry.get("apiKey").and_then(Value::as_str).unwrap_or_default();
    (!token.is_empty() && key == token) || key.starts_with(WORKBUDDY_TOKEN_PREFIX)
}

/// 当前配置里的网关地址（状态判定用）：我们条目的 `url` 去掉 `/chat/completions`。
///
/// 只看 vendor / 令牌前缀，不需要精确令牌——注册表的 `base` 字段签名是 `fn(&str)`，
/// 拿不到实例号。
pub(crate) fn current_base(text: &str) -> Option<String> {
    let document = read(text).ok()?;
    document
        .models
        .iter()
        .find(|entry| is_ours(entry, ""))
        .and_then(|entry| entry.get("url"))
        .and_then(Value::as_str)
        .map(|url| url.trim().trim_end_matches("/chat/completions").to_string())
}

/// 当前令牌：我们条目的 `apiKey`。
pub(crate) fn current_token(text: &str) -> Option<String> {
    let document = read(text).ok()?;
    document
        .models
        .iter()
        .find(|entry| is_ours(entry, ""))
        .and_then(|entry| entry.get("apiKey"))
        .and_then(Value::as_str)
        .map(|token| token.trim().to_string())
}

/// WorkBuddy 自己的模型给的「小窗口」：比它小的模型不给窗口选择菜单。
const SMALL_WINDOW: i64 = 200_000;

/// 一条模型条目。
fn entry(model: &ZcodeModel, base: &str, token: &str) -> Value {
    // 窗口不知道时用 200000（magpie 同款）：WorkBuddy 拿 0 会当成「没有窗口」，
    // 而 200K 是它自己模型的默认量级，行为安全。
    let context = if model.context > 0 { model.context } else { SMALL_WINDOW };
    let mut entry = json!({
        "id": model.id,
        "name": if model.name.trim().is_empty() { model.id.clone() } else { model.name.clone() },
        "vendor": WORKBUDDY_VENDOR,
        "apiKey": token,
        "url": format!("{base}/v1/chat/completions"),
        "maxInputTokens": context,
        "supportsToolCall": true,
        "supportsImages": model.image,
        "supportsReasoning": false,
    });
    // WorkBuddy 只在 contextWindow 列了两个以上尺寸时才给窗口选择菜单；
    // 不给的话会话预算按它的默认值走，所以这里把整个窗口设成默认
    if context > SMALL_WINDOW {
        entry["contextWindow"] = json!({
            "supportedLengths": [SMALL_WINDOW, context],
            "defaultLength": context,
        });
    }
    // 不给的话 WorkBuddy 用自己的默认输出上限；目录知道就写上
    if model.output > 0 {
        entry["maxOutputTokens"] = json!(model.output);
    }
    entry
}

/// 接线前的原值快照。
///
/// WorkBuddy 的模型条目里**没有「用户选中的是哪个」这种全局键**（选择在它的窗口里），
/// 所以断开只需要删掉我们的条目。要记两样：
/// - `theirs`：用户自己建的 `vendor = jolu` 条目（不能当我们的删掉）
/// - `availableModels`：CodeBuddy Code CLI 形状里那个展示列表的原值——我们追加过 id，
///   断开必须按原样恢复，否则用户的列表里会留下我们的模型名
pub(crate) fn snapshot(text: &str) -> Option<Value> {
    let document = read(text).ok()?;
    let theirs: Vec<Value> = document
        .models
        .iter()
        .filter(|entry| {
            // 用户自己建的同名 vendor，但 apiKey 不是 jolu- 开头 → 不是我们的
            entry.get("vendor").and_then(Value::as_str) == Some(WORKBUDDY_VENDOR)
                && !entry.get("apiKey").and_then(Value::as_str).is_some_and(|key| key.starts_with("jolu-"))
        })
        .cloned()
        .collect();
    let mut saved = serde_json::Map::new();
    saved.insert("theirs".into(), Value::Array(theirs));
    if let Some(available) = document.rest.as_ref().and_then(|rest| rest.get("availableModels")) {
        saved.insert("availableModels".into(), available.clone());
    }
    Some(Value::Object(saved))
}

/// 接入：先剔掉上次我们写的，再把用户的保留、我们的追加进去。
///
/// 保留两项用户状态（magpie 同款）：
/// - `disabled: true`（用户在 WorkBuddy 里关掉的模型）——同名模型的开关状态要留着
/// - `availableModels`（CodeBuddy Code CLI 形状里的展示列表）——替换成我们的模型 id
pub(crate) fn wire(text: &str, base: &str, token: &str, catalog: &[ZcodeModel]) -> Result<String, String> {
    if catalog.is_empty() {
        return Err("网关模型清单为空；先在「模型池」里启用要提供给这个工具的模型".into());
    }
    let document = read(text)?;
    // 用户在 WorkBuddy 里关掉的模型：按 id 记下来，重写后原样带回去
    let mut off = std::collections::BTreeSet::new();
    let mut kept = Vec::new();
    // 我们上一批条目插入的位置：保持它们在列表里的相对位置，不挪到最前也不挪到最后
    let mut at: Option<usize> = None;
    for entry in document.models {
        if is_ours(&entry, token) {
            if at.is_none() {
                at = Some(kept.len());
            }
            if entry.get("disabled").and_then(Value::as_bool) == Some(true) {
                if let Some(id) = entry.get("id").and_then(Value::as_str) {
                    off.insert(id.to_string());
                }
            }
            continue;
        }
        kept.push(entry);
    }
    let at = at.unwrap_or(kept.len());
    let mut add = Vec::new();
    let mut ids = Vec::new();
    for model in catalog {
        let mut model_entry = entry(model, base, token);
        if off.contains(&model.id) {
            model_entry["disabled"] = json!(true);
        }
        add.push(model_entry);
        ids.push(json!(model.id));
    }
    let mut models = Vec::with_capacity(kept.len() + add.len());
    models.extend(kept[..at].iter().cloned());
    models.extend(add);
    models.extend(kept[at..].iter().cloned());

    // `availableModels`：CodeBuddy Code CLI 形状里它是「选择器显示哪些」，把我们那批换掉
    let mut rest = document.rest;
    if let Some(rest) = rest.as_mut() {
        if let Some(available) = rest.get_mut("availableModels").and_then(Value::as_array_mut) {
            available.retain(|item| item.as_str().is_none_or(|id| !catalog.iter().any(|model| model.id == id)));
            available.extend(ids);
        }
    }
    render(Document { models, rest })
}

/// 断开：删掉我们的条目，其它一行不动。
pub(crate) fn restore(text: &str, token: &str, saved: Option<&Value>) -> Result<String, String> {
    let document = read(text)?;
    let mut models = Vec::with_capacity(document.models.len());
    let mut removed = false;
    for entry in document.models {
        if is_ours(&entry, token) {
            removed = true;
            continue;
        }
        models.push(entry);
    }
    if !removed {
        return Ok(text.to_string());
    }
    let mut rest = document.rest;
    if let Some(rest) = rest.as_mut() {
        // `availableModels`：按接线前的原值恢复（我们追加过的 id 必须拿掉）
        if let Some(original) = saved.and_then(|value| value.get("availableModels")) {
            rest.insert("availableModels".into(), original.clone());
        }
    }
    render(Document { models, rest })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<ZcodeModel> {
        vec![
            ZcodeModel { id: "kimi/k2".into(), name: "Kimi K2".into(), context: 256_000, output: 32_768, image: false },
            ZcodeModel { id: "group/main".into(), name: "group/main".into(), context: 0, output: 0, image: true },
        ]
    }
    const BASE: &str = "http://127.0.0.1:12731";
    const TOKEN: &str = "jolu-workbuddy.abcd1234";

    #[test]
    fn wire_bare_array_keeps_the_users_models() {
        let original = r#"[{"id":"my-own","name":"My Own","vendor":"Custom","apiKey":"sk-1","url":"https://mine/v1/chat/completions"}]"#;
        let wired = wire(original, BASE, TOKEN, &catalog()).unwrap();
        let list: Value = serde_json::from_str(&wired).unwrap();
        let list = list.as_array().unwrap();

        assert_eq!(list.len(), 3, "用户 1 条 + 我们 2 条");
        assert_eq!(list[0]["id"], "my-own", "用户的条目保留");
        assert_eq!(list[0]["apiKey"], "sk-1");
        assert_eq!(list[1]["id"], "kimi/k2");
        assert_eq!(list[1]["vendor"], WORKBUDDY_VENDOR);
        assert_eq!(list[1]["apiKey"], TOKEN);
        assert_eq!(list[1]["url"], format!("{BASE}/v1/chat/completions"));
        assert_eq!(list[1]["maxInputTokens"], 256_000);
        assert_eq!(list[1]["contextWindow"]["defaultLength"], 256_000, "大于 200K 的窗口给选择菜单");
        assert_eq!(list[1]["maxOutputTokens"], 32_768);
        assert_eq!(list[2]["maxInputTokens"], SMALL_WINDOW, "不知道窗口时用 200K");
        assert!(list[2].get("contextWindow").is_none(), "等于 200K 不给选择菜单");
        assert_eq!(list[2]["supportsImages"], true);
    }

    #[test]
    fn wire_object_shape_keeps_other_keys_and_available_models() {
        let original = r#"{"models":[{"id":"my-own","vendor":"Custom","apiKey":"sk-1","url":"https://mine"}],"availableModels":["my-own"],"other":1}"#;
        let wired = wire(original, BASE, TOKEN, &catalog()).unwrap();
        let root: Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(root["other"], 1, "包装对象的其它键保留");
        let available = root["availableModels"].as_array().unwrap();
        assert_eq!(available.len(), 3, "用户的 id 保留 + 我们的 2 个追加");
        assert_eq!(available[0], "my-own");
        assert_eq!(root["models"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn round_trip_returns_the_original_shape() {
        for original in [
            r#"[{"id":"my-own","name":"My Own","vendor":"Custom","apiKey":"sk-1","url":"https://mine"}]"#,
            r#"{"models":[{"id":"my-own","vendor":"Custom","apiKey":"sk-1","url":"https://mine"}],"availableModels":["my-own"],"other":1}"#,
        ] {
            let saved = snapshot(original);
            let wired = wire(original, BASE, TOKEN, &catalog()).unwrap();
            assert!(current_base(&wired).is_some_and(|base| base.starts_with(BASE)));
            assert_eq!(current_token(&wired).as_deref(), Some(TOKEN));
            let restored: Value = serde_json::from_str(&restore(&wired, TOKEN, saved.as_ref()).unwrap()).unwrap();
            assert_eq!(restored, serde_json::from_str::<Value>(original).unwrap(), "还原后与原始一致（形状也要对）");
        }
    }

    #[test]
    fn a_model_turned_off_in_workbuddy_stays_off() {
        let wired = wire("[]", BASE, TOKEN, &catalog()).unwrap();
        // 用户在 WorkBuddy 里关掉一个
        let off = wired.replace(r#""id": "kimi/k2","#, r#""id": "kimi/k2", "disabled": true,"#);
        let rewired = wire(&off, BASE, TOKEN, &catalog()).unwrap();
        let list: Value = serde_json::from_str(&rewired).unwrap();
        let entry = list.as_array().unwrap().iter().find(|entry| entry["id"] == "kimi/k2").unwrap();
        assert_eq!(entry["disabled"], true, "用户关掉的模型重写后仍保持关闭");
    }

    #[test]
    fn rewire_does_not_grow_our_entries() {
        let once = wire("[]", BASE, TOKEN, &catalog()).unwrap();
        let twice = wire(&once, BASE, TOKEN, &catalog()).unwrap();
        let list: Value = serde_json::from_str(&twice).unwrap();
        let ours = list.as_array().unwrap().iter().filter(|entry| entry["vendor"] == WORKBUDDY_VENDOR).count();
        assert_eq!(ours, 2, "重新接线不能重复追加");
    }

    #[test]
    fn empty_file_returns_to_an_empty_array() {
        let wired = wire("", BASE, TOKEN, &catalog()).unwrap();
        let restored: Value = serde_json::from_str(&restore(&wired, TOKEN, snapshot("").as_ref()).unwrap()).unwrap();
        assert_eq!(restored, json!([]), "空文件接完再断开要回到空数组");
    }

    #[test]
    fn refuses_empty_catalog_and_bad_shapes() {
        assert!(wire("[]", BASE, TOKEN, &[]).is_err());
        assert!(wire("{oops", BASE, TOKEN, &catalog()).is_err());
        assert!(wire("\"text\"", BASE, TOKEN, &catalog()).is_err(), "顶层是字符串要报错");
        assert!(wire(r#"{"models":{}}"#, BASE, TOKEN, &catalog()).is_err(), "models 不是数组要报错");
    }

    #[test]
    fn a_users_own_jolu_vendor_entry_is_not_deleted() {
        // 用户自己建了一条 vendor=jolu 但 key 不是我们的
        let original = r#"[{"id":"mine","vendor":"jolu","apiKey":"sk-own","url":"https://mine"}]"#;
        let saved = snapshot(original);
        let wired = wire(original, BASE, TOKEN, &catalog()).unwrap();
        let restored: Value = serde_json::from_str(&restore(&wired, TOKEN, saved.as_ref()).unwrap()).unwrap();
        let list = restored.as_array().unwrap();
        let mine = list.iter().find(|entry| entry["id"] == "mine").unwrap();
        assert_eq!(mine["apiKey"], "sk-own", "用户自己建的 jolu 条目不能被删");
    }
}
