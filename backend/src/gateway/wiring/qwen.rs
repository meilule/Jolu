//! Qwen Code 接线（参照 magpie `internal/agent/qwen.go`）。
//!
//! Qwen Code 是 Gemini CLI 的 fork，设置全在 `~/.qwen/settings.json`（`QWEN_HOME` 覆盖）。
//! 用户自己的模型是 `modelProviders.<协议>` 数组，每项 `{baseUrl, envKey, id, name}`：
//! `id` 是请求时用的模型名、`name` 是 /model 选择器里显示的、`envKey` 指向同文件 `env`
//! 对象（或 `~/.qwen/.env`）里的密钥变量名。会话启动时用哪个模型在 `model {name, baseUrl}`，
//! 同一 id 可能有多条（靠 baseUrl 区分）；`security.auth.selectedType = "openai"` 跳过
//! 启动时的 /auth 询问。
//!
//! Jolu 往 `modelProviders.openai` 末尾追加每个网关模型一条，`envKey` 用 `JOLU_QWEN_API_KEY`
//! ——**这个特定的变量名就是「这条是我们写的」的判据**（magpie 用 `MAGPIE_QWEN_API_KEY`）。
//! 用户自己的条目保持在前、原样不动，这样他们自己的模型 id 不会错位。
//!
//! 与 magpie 的差异：不实现 `Sync`（magpie 在网关地址变化时重写条目，Jolu 的地址由接线
//! 时的端口决定，重新接线会整段覆盖）；不写 `generationConfig` 之外的推理档位
//! （网关目录暂无按工具的能力数据）。

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::ZcodeModel;

/// 我们写进 `modelProviders.openai` 的条目共用的 `envKey`。
///
/// 用 Qwen 专属的名字而不是通用名：`~/.qwen/.env` 或 shell 导出的同名变量会**压过**
/// settings.json 里的值，用专属名字才不会被用户自己的同名变量悄悄顶掉。
pub(crate) const QWEN_ENV_KEY: &str = "JOLU_QWEN_API_KEY";

pub(crate) fn qwen_dir(home: &Path) -> PathBuf {
    std::env::var_os("QWEN_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".qwen"))
}

pub(crate) fn config_path(home: &Path) -> PathBuf {
    qwen_dir(home).join("settings.json")
}

fn parse(text: &str) -> Result<Value, String> {
    // Qwen 的 settings.json 允许 JSONC（注释、尾逗号），但 Jolu 只写不改用户结构；
    // 解析失败就当无效配置报错，不做「猜着修」——修坏了比报错更糟。
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
    let mut text = serde_json::to_string_pretty(root).map_err(|error| format!("settings.json 序列化失败：{error}"))?;
    text.push('\n');
    Ok(text)
}

/// 一条 `modelProviders.openai` 条目是否 Jolu 写的。
fn is_ours(entry: &Value) -> bool {
    entry.get("envKey").and_then(Value::as_str) == Some(QWEN_ENV_KEY)
}

/// 当前 `modelProviders.openai` 里我们那批条目的网关地址（取第一条即可，都相同）。
fn ours_base(list: &[Value]) -> Option<String> {
    list.iter()
        .find(|entry| is_ours(entry))
        .and_then(|entry| entry.get("baseUrl"))
        .and_then(Value::as_str)
        .map(|base| base.trim().to_string())
}

/// 当前配置里的网关地址（状态判定用）：`modelProviders` 里我们的条目，或 `env` 里的 Key
/// 存在但数组被外部清掉时退回 `model.baseUrl`。
pub(crate) fn current_base(text: &str) -> Option<String> {
    let root = parse(text).ok()?;
    let list = root.pointer("/modelProviders/openai")?.as_array()?;
    ours_base(list)
}

/// 当前令牌：写在 `env.JOLU_QWEN_API_KEY`。
pub(crate) fn current_token(text: &str) -> Option<String> {
    let root = parse(text).ok()?;
    root.pointer("/env/JOLU_QWEN_API_KEY")
        .and_then(Value::as_str)
        .map(|token| token.trim().to_string())
}

fn entry(model: &ZcodeModel, base: &str) -> Value {
    let mut entry = json!({
        "baseUrl": format!("{base}/v1"),
        "envKey": QWEN_ENV_KEY,
        "id": model.id,
        // 显示名：Qwen 的 /model 用它。目录没有显示名时用 id 本身
        "name": if model.name.trim().is_empty() { model.id.clone() } else { model.name.clone() },
    });
    // 上下文窗口：目录里不知道就不写（Qwen 会用它的默认值），不编造
    if model.context > 0 {
        entry["generationConfig"] = json!({ "contextWindowSize": model.context });
    }
    entry
}

/// 选中的模型：给了就用，否则取目录第一个。
fn pick<'a>(model: Option<&'a str>, catalog: &'a [ZcodeModel]) -> Option<&'a str> {
    model.map(str::trim).filter(|value| !value.is_empty()).or_else(|| catalog.first().map(|m| m.id.as_str()))
}

/// 接线前的原值快照。
///
/// 只记 Jolu 会改的四个键，且**排除我们自己写下的值**——否则重新接线会把上次的网关地址
/// 当成用户原值，断开时就还原不回去（magpie 用 `keyAuthSeen` 标记解决，
/// 这里靠「快照里不存网关值」等价处理）。
pub(crate) fn snapshot(text: &str) -> Option<Value> {
    let root = parse(text).ok()?;
    let mut saved = Map::new();
    // model.name / model.baseUrl：只有当前指向的不是我们的条目时才当用户原值
    let points_at_ours = root
        .pointer("/modelProviders/openai")
        .and_then(Value::as_array)
        .is_some_and(|list| {
            let name = root.pointer("/model/name").and_then(Value::as_str);
            list.iter().any(|entry| is_ours(entry) && entry.get("id").and_then(Value::as_str) == name)
        });
    if !points_at_ours {
        if let Some(name) = root.pointer("/model/name").and_then(Value::as_str) {
            saved.insert("model.name".into(), json!(name));
        }
        if let Some(base) = root.pointer("/model/baseUrl").and_then(Value::as_str) {
            saved.insert("model.baseUrl".into(), json!(base));
        }
    }
    // security.auth.selectedType：不是我们写的 "openai" 才存
    if let Some(selected) = root.pointer("/security/auth/selectedType").and_then(Value::as_str) {
        if selected != "openai" || root.pointer("/modelProviders/openai").and_then(Value::as_array).is_none_or(|list| !list.iter().any(is_ours)) {
            saved.insert("security.auth.selectedType".into(), json!(selected));
        }
    }
    // env.JOLU_QWEN_API_KEY：用户的同名值（如果有）要存下来，断开时放回
    if let Some(token) = root.pointer("/env/JOLU_QWEN_API_KEY").and_then(Value::as_str) {
        if !token.starts_with("jolu-") {
            saved.insert("env.JOLU_QWEN_API_KEY".into(), json!(token));
        }
    }
    Some(Value::Object(saved))
}

/// 接入：追加我们的条目、写密钥、切 auth、把默认模型指向我们的。
pub(crate) fn wire(text: &str, base: &str, token: &str, model: Option<&str>, catalog: &[ZcodeModel]) -> Result<String, String> {
    if catalog.is_empty() {
        return Err("网关模型清单为空；先在「模型池」里启用要提供给这个工具的模型".into());
    }
    let selected = pick(model, catalog).ok_or("没有可选的模型")?.to_string();
    let mut root = parse(text)?;

    // 1. modelProviders.openai：先剔掉上次我们写的，再把用户的保留在前、我们的追加在后
    {
        let root_object = root.as_object_mut().ok_or("settings.json 顶层不是对象")?;
        let providers = root_object
            .entry("modelProviders")
            .or_insert_with(|| json!({}));
        let providers = providers.as_object_mut().ok_or("modelProviders 不是对象，Jolu 不改写它")?;
        let list = providers.entry("openai").or_insert_with(|| json!([]));
        let list = list.as_array_mut().ok_or("modelProviders.openai 不是数组，Jolu 不改写它")?;
        list.retain(|entry| !is_ours(entry));
        for model in catalog {
            list.push(entry(model, base));
        }
    }

    // 2. env.JOLU_QWEN_API_KEY：密钥放这里，数组项只引用变量名
    {
        let root_object = root.as_object_mut().ok_or("settings.json 顶层不是对象")?;
        let env = root_object.entry("env").or_insert_with(|| json!({}));
        let env = env.as_object_mut().ok_or("env 不是对象，Jolu 不改写它")?;
        env.insert(QWEN_ENV_KEY.into(), json!(token));
    }

    // 3. security.auth.selectedType = openai：跳过启动时的 /auth 询问
    {
        let root_object = root.as_object_mut().ok_or("settings.json 顶层不是对象")?;
        let security = root_object.entry("security").or_insert_with(|| json!({}));
        let security = security.as_object_mut().ok_or("security 不是对象，Jolu 不改写它")?;
        let auth = security.entry("auth").or_insert_with(|| json!({}));
        let auth = auth.as_object_mut().ok_or("security.auth 不是对象，Jolu 不改写它")?;
        auth.insert("selectedType".into(), json!("openai"));
    }

    // 4. model.name / model.baseUrl：默认模型指向我们写的那条
    {
        let root_object = root.as_object_mut().ok_or("settings.json 顶层不是对象")?;
        let model_object = root_object.entry("model").or_insert_with(|| json!({}));
        let model_object = model_object.as_object_mut().ok_or("model 不是对象，Jolu 不改写它")?;
        model_object.insert("name".into(), json!(selected));
        model_object.insert("baseUrl".into(), json!(format!("{base}/v1")));
    }

    render(&root)
}

/// 断开：删掉我们的条目与密钥，选择键仍指向我们时还原或删除。
pub(crate) fn restore(text: &str, saved: Option<&Value>) -> Result<String, String> {
    let mut root = parse(text)?;
    let saved_at = |key: &str| saved.and_then(|value| value.get(key)).cloned();

    let mut had_ours = false;
    // 我们那批条目的 id（删之前记下来：删完就查不到了，第 4 步要用来判「当前选的是不是我们写的」）
    let ours_ids: Vec<String> = root
        .pointer("/modelProviders/openai")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|entry| is_ours(entry))
                .filter_map(|entry| entry.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // 1. modelProviders.openai：只剔掉 envKey 是我们的
    if let Some(list) = root.pointer_mut("/modelProviders/openai").and_then(Value::as_array_mut) {
        let before = list.len();
        list.retain(|entry| !is_ours(entry));
        had_ours = list.len() != before;
    }
    prune_empty_array(&mut root, &["modelProviders", "openai"]);
    prune_empty_object(&mut root, &["modelProviders"]);

    // 2. env.JOLU_QWEN_API_KEY：用户有原值就放回，否则删掉；env 空了也删
    {
        let ours = root.pointer("/env/JOLU_QWEN_API_KEY").and_then(Value::as_str).is_some();
        if ours || had_ours {
            match saved_at("env.JOLU_QWEN_API_KEY").and_then(|value| value.as_str().map(str::to_string)) {
                Some(original) => {
                    if let Some(env) = root.pointer_mut("/env").and_then(Value::as_object_mut) {
                        env.insert(QWEN_ENV_KEY.into(), json!(original));
                    }
                }
                None => {
                    if let Some(env) = root.pointer_mut("/env").and_then(Value::as_object_mut) {
                        env.remove(QWEN_ENV_KEY);
                    }
                }
            }
        }
        prune_empty_object(&mut root, &["env"]);
    }

    // 3. security.auth.selectedType：仍是我们写的 "openai" 时才动
    let selected_ours = root.pointer("/security/auth/selectedType").and_then(Value::as_str) == Some("openai");
    if selected_ours && (had_ours || saved_at("security.auth.selectedType").is_some()) {
        match saved_at("security.auth.selectedType").and_then(|value| value.as_str().map(str::to_string)) {
            Some(original) => {
                if let Some(auth) = root.pointer_mut("/security/auth").and_then(Value::as_object_mut) {
                    auth.insert("selectedType".into(), json!(original));
                }
            }
            None => {
                if let Some(auth) = root.pointer_mut("/security/auth").and_then(Value::as_object_mut) {
                    auth.remove("selectedType");
                }
            }
        }
    }
    prune_empty_object(&mut root, &["security", "auth"]);
    prune_empty_object(&mut root, &["security"]);

    // 4. model.name / model.baseUrl：仍指向我们写的模型才还原（用户改选了别的就不动）
    //
    // 要区分三种情况（`had_ours` 记的是「这个文件里曾经有我们的条目」）：
    //   a. 快照里存过 model.name → 接线前用户选的是别的模型，现在这个名是我们写的 → 还原
    //   b. 快照里没有 + 当前 model.name 在我们的条目名单里 → 我们写的，删干净
    //   c. 快照里没有 + 当前 model.name 不在我们的名单里 → 用户改选了别的，一行不动
    //
    // `ours_ids` 必须在**删除条目之前**记下来，删完就查不到了。
    if had_ours {
        let current = root.pointer("/model/name").and_then(Value::as_str).map(str::to_string);
        let current_is_ours = current.as_deref().is_some_and(|name| ours_ids.iter().any(|id| id == name));
        match saved_at("model.name").and_then(|value| value.as_str().map(str::to_string)) {
            Some(original) => {
                if let Some(model_object) = root.pointer_mut("/model").and_then(Value::as_object_mut) {
                    model_object.insert("name".into(), json!(original));
                    match saved_at("model.baseUrl").and_then(|value| value.as_str().map(str::to_string)) {
                        Some(base) => {
                            model_object.insert("baseUrl".into(), json!(base));
                        }
                        None => {
                            model_object.remove("baseUrl");
                        }
                    }
                }
            }
            None if current_is_ours => {
                if let Some(model_object) = root.pointer_mut("/model").and_then(Value::as_object_mut) {
                    model_object.remove("name");
                    model_object.remove("baseUrl");
                }
            }
            // 用户改选了别的模型：不动
            None => {}
        }
    }
    prune_empty_object(&mut root, &["model"]);

    render(&root)
}

/// 删掉一个空数组键（路径不存在或不是空数组则不动）。
fn prune_empty_array(root: &mut Value, path: &[&str]) {
    let Some((last, parents)) = path.split_last() else { return };
    let Some(parent) = parents.iter().try_fold(root, |node, key| node.get_mut(*key)) else { return };
    let empty = parent.get(*last).and_then(Value::as_array).is_some_and(|list| list.is_empty());
    if empty {
        if let Some(object) = parent.as_object_mut() {
            object.remove(*last);
        }
    }
}

/// 删掉一个空对象键（路径不存在或不是空对象则不动）。
fn prune_empty_object(root: &mut Value, path: &[&str]) {
    let Some((last, parents)) = path.split_last() else { return };
    let Some(parent) = parents.iter().try_fold(root, |node, key| node.get_mut(*key)) else { return };
    let empty = parent.get(*last).and_then(Value::as_object).is_some_and(|object| object.is_empty());
    if empty {
        if let Some(object) = parent.as_object_mut() {
            object.remove(*last);
        }
    }
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
    const TOKEN: &str = "jolu-qwen.abcd1234";

    #[test]
    fn wire_appends_entries_and_keeps_the_users_own() {
        let original = r#"{"modelProviders":{"openai":[{"baseUrl":"https://mine","envKey":"MY_KEY","id":"my-model","name":"My Model"}]},"model":{"name":"my-model","baseUrl":"https://mine"},"env":{"MY_KEY":"sk-1"},"keep":1}"#;
        let wired = wire(original, BASE, TOKEN, Some("kimi/k2"), &catalog()).unwrap();
        let root: Value = serde_json::from_str(&wired).unwrap();

        let list = root["modelProviders"]["openai"].as_array().unwrap();
        assert_eq!(list.len(), 3, "用户 1 条 + 我们 2 条");
        assert_eq!(list[0]["id"], "my-model", "用户的条目保持在前、位置不动");
        assert_eq!(list[0]["envKey"], "MY_KEY", "用户的 envKey 不能被改");
        assert_eq!(list[1]["id"], "kimi/k2");
        assert_eq!(list[1]["envKey"], QWEN_ENV_KEY);
        assert_eq!(list[1]["baseUrl"], format!("{BASE}/v1"));
        assert_eq!(list[1]["generationConfig"]["contextWindowSize"], 256_000);
        assert!(list[2].get("generationConfig").is_none(), "不知道的窗口不编");
        assert_eq!(root["env"][QWEN_ENV_KEY], TOKEN, "密钥写在 env");
        assert_eq!(root["env"]["MY_KEY"], "sk-1", "用户自己的 env 键保留");
        assert_eq!(root["security"]["auth"]["selectedType"], "openai");
        assert_eq!(root["model"]["name"], "kimi/k2", "默认模型指向我们的");
        assert_eq!(root["model"]["baseUrl"], format!("{BASE}/v1"));
        assert_eq!(root["keep"], 1);
    }

    #[test]
    fn wire_and_restore_round_trip_is_clean() {
        let original = r#"{"modelProviders":{"openai":[{"baseUrl":"https://mine","envKey":"MY_KEY","id":"my-model","name":"My Model"}]},"model":{"name":"my-model","baseUrl":"https://mine"},"env":{"MY_KEY":"sk-1"},"keep":1}"#;
        let saved = snapshot(original);
        let wired = wire(original, BASE, TOKEN, Some("kimi/k2"), &catalog()).unwrap();
        assert!(current_base(&wired).is_some_and(|base| base.starts_with(BASE)));
        assert_eq!(current_token(&wired).as_deref(), Some(TOKEN));

        let restored: Value = serde_json::from_str(&restore(&wired, saved.as_ref()).unwrap()).unwrap();
        assert_eq!(restored, serde_json::from_str::<Value>(original).unwrap(), "还原后必须与原始配置一致");
    }

    #[test]
    fn restore_on_an_empty_file_leaves_no_shells() {
        let wired = wire("{}", BASE, TOKEN, None, &catalog()).unwrap();
        let restored: Value = serde_json::from_str(&restore(&wired, snapshot("{}").as_ref()).unwrap()).unwrap();
        assert_eq!(restored, json!({}), "空文件接完再断开必须回到空对象，不留 env / modelProviders 空壳");
    }

    #[test]
    fn restore_leaves_a_model_the_user_switched_to() {
        let wired = wire("{}", BASE, TOKEN, Some("kimi/k2"), &catalog()).unwrap();
        // 用户在 Qwen 里改选了别的模型
        let switched = wired.replace(r#""name": "kimi/k2""#, r#""name": "my-own""#);
        let restored: Value = serde_json::from_str(&restore(&switched, snapshot("{}").as_ref()).unwrap()).unwrap();
        assert_eq!(restored["model"]["name"], "my-own", "用户改选的模型不能被还原掉");
    }

    #[test]
    fn rewire_does_not_grow_our_entries() {
        let once = wire("{}", BASE, TOKEN, None, &catalog()).unwrap();
        let twice = wire(&once, BASE, TOKEN, None, &catalog()).unwrap();
        let list: Value = serde_json::from_str(&twice).unwrap();
        let ours: Vec<&Value> = list["modelProviders"]["openai"].as_array().unwrap().iter().filter(|entry| is_ours(entry)).collect();
        assert_eq!(ours.len(), 2, "重新接线不能重复追加我们的条目");
    }

    #[test]
    fn refuses_empty_catalog_and_bad_shapes() {
        assert!(wire("{}", BASE, TOKEN, None, &[]).is_err());
        assert!(wire("{oops", BASE, TOKEN, None, &catalog()).is_err());
        assert!(wire(r#"{"modelProviders":[]}"#, BASE, TOKEN, None, &catalog()).is_err());
        assert!(wire("\u{feff}{}", BASE, TOKEN, None, &catalog()).is_ok(), "带 BOM 的也能读");
    }

    #[test]
    fn snapshot_does_not_take_our_own_values_as_original() {
        let wired = wire("{}", BASE, TOKEN, None, &catalog()).unwrap();
        let saved = snapshot(&wired).unwrap();
        assert!(saved.get("env.JOLU_QWEN_API_KEY").is_none(), "我们的令牌不是用户原值");
        assert!(saved.get("security.auth.selectedType").is_none(), "我们写的 openai 不是用户原值");
        assert!(saved.get("model.name").is_none(), "我们写的模型名不是用户原值");
    }
}
