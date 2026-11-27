//! DeepSeek Harness（dsh）的专用接线：按 profile 写 cordis.patch.yml（多文件）+ .env +
//! credentials refs（参照 magpie `internal/agent/dsh.go`，按本机实测结构裁剪的 v1）。
//!
//! 与 wiring_extra 四家（单 JSON 文件）的差异：patch 是用户手编的行级 YAML，全部用行级拼接
//! 保注释保缩进；接线要写每个 profile 的 patch + 密钥两处，所以走 plan_wire_files /
//! plan_unwire_files 的多文件接口（plan_wire / plan_unwire 的 dsh 特有分支调用）。
//!
//! v1 裁剪边界（差异都写在对应函数）：不做 home 层镜像、legacy config.yaml、WSL、
//! desktop 未开机制、web 搜索条目、llm-deepseek 旧接管行；密钥只写 apiKeyEnv 引用
//! （值进 .env 与 credentials refs），不在 patch 里放明文。
//!
//! 出处（magpie `internal/agent/dsh.go`）：dshAt:90-187（路径与 Fields）、dshProfiles:221、
//! dshSetFile:1118-1200（条目替换/还原与 stash 语义）、dshPutRoute:1202、dshEnv:1049、
//! dshCreds:1063（version 1 才写）、dshOurs:659（provider 判 ours）。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{dotenv_get, points_at_gateway, token_for, wire_dotenv, ZcodeModel};

/* ─────────────────────────── DeepSeek Harness（dsh，cordis.patch.yml） ───────────────────────────
 * 参照 magpie `internal/agent/dsh.go`（2349 行；本节是按本机实测结构裁剪的 v1）。
 *
 * dsh 0.1.5 起按 profile 存配置：`~/.dsh/profiles/<名>/cordis.patch.yml` 是「补丁层」——
 * 顶层 YAML 数组，每项 `- id: <目标>` 指向要改的插件条目。接线要动两个条目：
 *   - `llm-pi-ai`（自定义供应商所在的插件）：`config.providers.<路由>` 下放我们的路由
 *     （baseURL / apiKeyEnv / api / models 列表）；用户的其它供应商（如 hualong）原样保留；
 *   - `agent-default-model`：新会话的默认模型（provider = 我们的路由名 + 网关模型 id）。
 * 密钥不进 patch：路由里只写 `apiKeyEnv: JOLU_API_KEY` 引用，值写到 `~/.dsh/.env`
 * （产品 CLI 读）与 `~/.dsh/.credentials.yaml` 的 `refs`（桌面应用读，magpie #969）。
 *
 * 与 magpie 的差异（v1 裁剪，注释即边界）：
 *   - 不做 home 层 `cordis.patch.yml` 镜像（本机没有；magpie 在有该文件时才镜像）；
 *   - 不做 legacy `config.yaml`（0.1.5 之前）与 WSL、desktop 未开机制；
 *   - 不动 `llm-deepseek` 旧接管行与 web 搜索条目。
 * patch 是用户手编的 YAML：全部用**行级拼接**（按 `- id:` 切条目，条目内按缩进切子块），
 * 用户的条目、注释、缩进一行不动——不引入 YAML 库序列化重排。
 */

/// dsh 里我们路由的段名与 patch 条目 id（对齐 magpie dshRoute / dshPiRow）。
pub(crate) const DSH_PROVIDER: &str = "jolu";
/// 密钥引用名：patch 里 `apiKeyEnv: JOLU_API_KEY`，值在 `.env` 与 credentials `refs`。
pub(crate) const DSH_KEY_ENV: &str = "JOLU_API_KEY";

/// dsh 配置根目录：`DSH_HOME` 覆盖 → `~/.dsh`（与探测 dir_dsh 同源）。
fn dsh_dir(home: &Path) -> PathBuf {
    std::env::var_os("DSH_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".dsh"))
}

/// `dsh_dir` 的测试公开包装（`wiring_specs` 的回归测试要造临时 home）。
#[cfg(test)]
pub(crate) fn dsh_dir_public(home: &Path) -> PathBuf {
    dsh_dir(home)
}

/// 各 profile 的 patch 文件（web 优先，magpie dshProfiles 同款排序：files[0] 是读默认模型的地方）。
pub(crate) fn dsh_profiles(dir: &Path) -> Vec<PathBuf> {
    // profile 目录里有任何一个子目录带 cordis.patch.yml 才算有 profiles
    let mut files: Vec<PathBuf> = Vec::new();
    let entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(dir.join("profiles")) {
        Ok(entries) => entries.filter_map(Result::ok).collect(),
        Err(_) => return files,
    };
    let has_profiles = entries.iter().any(|e| e.file_type().is_ok_and(|k| k.is_dir()) && e.path().join("cordis.patch.yml").is_file());
    if has_profiles {
        files = entries
            .into_iter()
            .filter(|e| e.file_type().is_ok_and(|k| k.is_dir()) && e.path().join("cordis.patch.yml").is_file())
            .map(|e| e.path().join("cordis.patch.yml"))
            .collect();
    }
    files.sort_by_key(|path| {
        let name = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("").to_string();
        // web 是 dsh 的默认 profile，排最前（magpie 同款）；其余按名字排
        if name == "web" { String::new() } else { format!("1{name}") }
    });
    files
}

/// dsh 的主配置文件：第一个 profile 的 patch；没有 profile 时桌面应用的
/// `profiles/desktop`（由 dsh 自己生成）或 0.1.5 前的 `config.yaml`。
pub(crate) fn dsh_main_file(dir: &Path) -> PathBuf {
    if let Some(first) = dsh_profiles(dir).first() {
        return first.clone();
    }
    dir.join("profiles").join("desktop").join("cordis.patch.yml")
}

/** patch 条目：`- id: <目标>` 起头的一行块。id 从首行正则取，行块原样保留。 */
/// dsh 的条目就是「行级 YAML 条目」（实现见 `yaml_lines`：这套逻辑 2026-11-20 抽出，
/// 供后续 qwen / droid / workbuddy 这类「数组插项」接线复用）。
pub(crate) type DshItem = crate::gateway::yaml_lines::Item;

// 薄转发：本文件 30+ 处调用点沿用原名字，实现都在 `yaml_lines`（抽出来给后续工具复用）。
fn dsh_parse(text: &str) -> (Vec<String>, Vec<DshItem>) { crate::gateway::yaml_lines::parse(text) }
fn dsh_join(head: &[String], items: &[DshItem]) -> String { crate::gateway::yaml_lines::join(head, items) }
fn dsh_find(items: &[DshItem], id: &str) -> Option<usize> { crate::gateway::yaml_lines::find(items, id) }
fn dsh_block_range(lines: &[String], indent: usize, key: &str) -> Option<(usize, usize)> { crate::gateway::yaml_lines::block_range(lines, indent, key) }

/// 条目的 config 里是否指向我们的路由（provider == jolu）。条目被 dsh 重写去掉标记时，
/// 这是最可靠的「是我们写的」判定（magpie dshOurs 的 provider 分支）。
pub(crate) fn dsh_item_ours(item: &DshItem) -> bool {
    item.lines.iter().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("provider:") && trimmed[9..].trim() == DSH_PROVIDER
    })
}

/// dsh 压缩预留的 headroom：maxTokens 占掉的部分之外还要留这么多空闲
/// （magpie dshHeadroom = compaction-basic 的默认 headroomTokens）。
const DSH_HEADROOM: i64 = 65_536;

/// maxTokens 的上限公式（magpie dshMaxTokens #2823/#3298）：dsh 把列出的 maxTokens 当成
/// 每次请求的输出并预留 65536 headroom 再决定何时压缩；厂商的输出上限贴近窗口时会把
/// 压缩空间挤没。所以压到「留一半窗口给压缩」（window/2 − 65536），不足 262144 的窗口
/// 没有可让的一半，退到窗口的 1/4。`output` 是目录里的输出上限。
fn dsh_max_tokens(context: i64, output: i64) -> i64 {
    let room = if context > 0 { (context / 4).max(context / 2 - DSH_HEADROOM) } else { i64::MAX };
    output.min(room.max(1))
}

/// OpenAI 兼容路由的 baseURL（magpie dshBaseURL）：pi-ai 把 base 原样交给 OpenAI SDK，
/// SDK 只补 /chat/completions 或 /responses，所以路径要带 /v1（Anthropic 的才在根，v1 不写该路由）。
fn dsh_base_url(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    format!("{trimmed}/v1")
}

/// 生成我们的路由子块（providers.jolu 下的一块，缩进 6）——字段对齐 magpie
/// `dshPiRoute` / `dshKindConfig`（dshKinds[0]，openai-completions 协议）：
///   - displayName / apiKeyEnv / api / baseURL 与 magpie 同形；
///   - models：id 用网关模型 id（发上游可路由）、contextWindow 照目录、maxTokens 走
///     压缩公式、input 收图才写且用 flow 数组（magpie `input,omitempty,flow`）；
///   - reasoningEfforts 不写：Jolu 目录暂无模型推理档位数据（magpie dshModelLevels 按
///     catalog.Efforts 生成），不写时 dsh 不提供 thinking 切换、按它自己的默认跑。
/// 与 magpie 的差异：magpie 按协议分三个路由（jolu / jolu-responses / jolu-anthropic，
/// 模型按其原生 API 分组）；Jolu 目录暂无 API 信息，v1 只写 openai-completions 一个路由，
/// 网关做协议翻译。
pub(crate) fn dsh_route_block(base: &str, models: &[ZcodeModel]) -> Vec<String> {
    let mut lines = vec![format!("      {DSH_PROVIDER}:")];
    lines.push("        displayName: Jolu".to_string());
    lines.push(format!("        apiKeyEnv: {DSH_KEY_ENV}"));
    lines.push("        api: openai-completions".to_string());
    lines.push(format!("        baseURL: {}", dsh_base_url(base)));
    lines.push("        models:".to_string());
    for entry in models {
        lines.push(format!("          - id: {}", entry.id));
        lines.push(format!("            name: {}", entry.name));
        if entry.context > 0 {
            lines.push(format!("            contextWindow: {}", entry.context));
        }
        if entry.output > 0 {
            lines.push(format!("            maxTokens: {}", dsh_max_tokens(entry.context, entry.output)));
        }
        if entry.image {
            lines.push("            input: [text, image]".to_string());
        }
    }
    lines
}

/// llm-pi-ai 条目删掉我们的路由后的空清理（magpie dshPutRoute off 分支同款）：
/// providers 空了删键、config 空了删段、条目只剩 id 与 name 连条目一起删。
/// 返回 true = 条目已被清空，调用方要从列表里移除。
fn dsh_prune_pi(item: &mut DshItem) -> bool {
    let mut lines = item.lines.clone();
    if let Some((ps, pe)) = dsh_block_range(&lines, 4, "providers") {
        let has_child = lines[ps + 1..=pe].iter().any(|line| {
            let depth = line.len() - line.trim_start().len();
            !line.trim().is_empty() && depth > 4
        });
        if !has_child {
            let mut trimmed = lines[..ps].to_vec();
            trimmed.extend(lines[pe + 1..].to_vec());
            lines = trimmed;
        }
    }
    if let Some((cs, ce)) = dsh_block_range(&lines, 2, "config") {
        let has_child = lines[cs + 1..=ce].iter().any(|line| {
            let depth = line.len() - line.trim_start().len();
            !line.trim().is_empty() && depth > 2
        });
        if !has_child {
            let mut trimmed = lines[..cs].to_vec();
            trimmed.extend(lines[ce + 1..].to_vec());
            lines = trimmed;
        }
    }
    // config 删掉后条目只剩 id（与 name 这类元信息）时连条目一起删——
    // magpie dshPutRoute off 分支的 `len(n.Content) <= 4`（id 一对 / id+name 两对）
    let only_meta = lines.iter().skip(1).all(|line| {
        let trimmed = line.trim_start();
        line.trim().is_empty() || trimmed.starts_with('#') || trimmed.starts_with("name:")
    });
    if only_meta {
        return true;
    }
    item.lines = lines;
    false
}

/// 替换 / 插入 / 移除 llm-pi-ai 条目里的 providers.jolu 子块。
/// 条目或 providers 不存在时按需生成；`on=false` 只删除（用户的其它供应商不动）。
/// 返回 true = 条目已被清空，调用方要从列表里移除它。
pub(crate) fn dsh_put_route(item: &mut DshItem, on: bool, base: &str, models: &[ZcodeModel]) -> bool {
    let pi_id = "llm-pi-ai".to_string();
    // 条目不存在且要写：整条目生成（首行带 `# jolu` 标记，magpie dshMark 同款；
    // dsh 重写补丁后标记可能丢，ours 判定还有 provider 兜底）
    if dsh_find(std::slice::from_ref(item), &pi_id).is_none() {
        if !on {
            return false;
        }
        item.id = pi_id.clone();
        item.lines = vec![
            "- id: llm-pi-ai # jolu".to_string(),
            "  name: \"@deepseek-ai/dsh-llm-pi-ai\"".to_string(),
            "  config:".to_string(),
            "    providers:".to_string(),
        ];
        item.lines.extend(dsh_route_block(base, models));
        return false;
    }
    let (config_at, _) = match dsh_block_range(&item.lines, 2, "config") {
        Some(range) => range,
        None => {
            if !on {
                return false;
            }
            item.lines.push("  config:".to_string());
            item.lines.push("    providers:".to_string());
            item.lines.extend(dsh_route_block(base, models));
            return false;
        }
    };
    let (prov_at, prov_end) = match dsh_block_range(&item.lines, 4, "providers") {
        Some(range) => range,
        None => {
            if !on {
                return false;
            }
            let mut lines = item.lines[..=config_at].to_vec();
            lines.push("    providers:".to_string());
            lines.extend(dsh_route_block(base, models));
            lines.extend(item.lines[config_at + 1..].to_vec());
            item.lines = lines;
            return false;
        }
    };
    // 在 providers 子块里找我们的子块（缩进 6 的 `jolu:`）
    let ours = dsh_block_range(&item.lines[prov_at..=prov_end], 6, DSH_PROVIDER).map(|(s, e)| (prov_at + s, prov_at + e));
    match (ours, on) {
        (Some((s, e)), false) => {
            let mut lines = item.lines[..s].to_vec();
            lines.extend(item.lines[e + 1..].to_vec());
            item.lines = lines;
            dsh_prune_pi(item)
        }
        (Some((s, e)), true) => {
            // 替换我们的子块；用户在 dsh Models 页改过的线协议（三种网关都支持的）保留
            // （magpie dshAPI：改了一会就被接管的问题，#2504 同类）
            let current_api = item.lines[s..=e].iter().find_map(|line| {
                let trimmed = line.trim_start();
                trimmed.strip_prefix("api:").map(|value| value.trim().trim_matches(['\'', '"']).to_string())
            });
            let mut block = dsh_route_block(base, models);
            if let Some(api) = current_api.filter(|api| ["openai-completions", "openai-responses", "anthropic-messages"].contains(&api.as_str())) {
                if let Some(at) = block.iter().position(|line| line.trim_start().starts_with("api:")) {
                    block[at] = format!("        api: {api}");
                }
            }
            let mut lines = item.lines[..s].to_vec();
            lines.extend(block);
            lines.extend(item.lines[e + 1..].to_vec());
            item.lines = lines;
            false
        }
        (None, true) => {
            // 插到 providers 块末尾（保持用户子块在前）
            let insert_at = prov_end + 1;
            let mut lines = item.lines[..insert_at].to_vec();
            lines.extend(dsh_route_block(base, models));
            lines.extend(item.lines[insert_at..].to_vec());
            item.lines = lines;
            false
        }
        (None, false) => false,
    }
}

/// 生成 agent-default-model 条目行块（provider 指我们的路由，model 是网关模型 id）。
/// reasoningEffort 不写：Jolu 目录暂无模型推理档位数据（magpie dshDefaultLines 的 effort
/// 来自 dshLevels，空则不写），dsh 按它自己的默认（high）跑。
pub(crate) fn dsh_default_block(model: &str) -> Vec<String> {
    vec![
        "- id: agent-default-model # jolu".to_string(),
        "  name: \"@deepseek-ai/dsh-agent-default-model\"".to_string(),
        "  config:".to_string(),
        format!("    provider: {DSH_PROVIDER}"),
        format!("    model: {model}"),
    ]
}

/// 生成 / 替换 / 移除 agent-default-model 条目。`model` 为空 = 移除（还原快照原块由调用方做）。
pub(crate) fn dsh_put_default(items: &mut Vec<DshItem>, model: &str) {
    let id = "agent-default-model".to_string();
    let at = dsh_find(items, &id);
    if model.is_empty() {
        if let Some(at) = at {
            if dsh_item_ours(&items[at]) {
                items.remove(at);
            }
        }
        return;
    }
    let block = DshItem { id: id.clone(), lines: dsh_default_block(model) };
    match at {
        Some(at) => items[at] = block,
        None => items.push(block),
    }
}

/// 读 agent-default-model 条目里的 model 值（只在我们路由上时返回 Some）。
pub(crate) fn dsh_default_model(items: &[DshItem]) -> Option<String> {
    let at = dsh_find(items, "agent-default-model")?;
    let item = &items[at];
    dsh_item_ours(item).then(|| {
        item.lines.iter().find_map(|line| {
            let trimmed = line.trim_start();
            trimmed.strip_prefix("model:").map(|value| value.trim().trim_matches(['\'', '"']).to_string()).filter(|value| !value.is_empty())
        })
    }).flatten()
}

/// patch 里我们的路由子块（providers.jolu）的 baseURL。
pub(crate) fn dsh_route_base(items: &[DshItem]) -> Option<String> {
    let at = dsh_find(items, "llm-pi-ai")?;
    let lines = &items[at].lines;
    let (prov_at, prov_end) = dsh_block_range(lines, 4, "providers")?;
    let (start, end) = dsh_block_range(&lines[prov_at..=prov_end], 6, DSH_PROVIDER)?;
    let slice = &lines[prov_at + start..=prov_at + end];
    slice.iter().find_map(|line| {
        let trimmed = line.trim_start();
        trimmed.strip_prefix("baseURL:").map(|value| value.trim().trim_matches(['\'', '"']).to_string()).filter(|value| !value.is_empty())
    })
}

/// 接线后的 patch：写路由 + 默认模型（`model` 空 = 断开）。返回新文本。
pub(crate) fn dsh_patch_wire(text: &str, base: &str, model: &str, models: &[ZcodeModel]) -> Result<String, String> {
    let (head, mut items) = dsh_parse(text);
    // 不是条目列表的文件不动（magpie dshRead 报错即跳过同款）
    if items.is_empty() && !text.trim().is_empty() {
        return Ok(text.to_string());
    }
    let emptied = match dsh_find(&items, "llm-pi-ai") {
        Some(at) => dsh_put_route(&mut items[at], !model.is_empty(), base, models),
        None => {
            // 条目不存在：预填 id / name 两行再交给 dsh_put_route 补 config（含 `# jolu` 标记）
            let mut item = DshItem {
                id: "llm-pi-ai".to_string(),
                lines: vec!["- id: llm-pi-ai # jolu".to_string(), "  name: \"@deepseek-ai/dsh-llm-pi-ai\"".to_string()],
            };
            let emptied = dsh_put_route(&mut item, !model.is_empty(), base, models);
            if !item.lines.is_empty() {
                items.push(item);
            }
            emptied
        }
    };
    if emptied {
        if let Some(at) = dsh_find(&items, "llm-pi-ai") {
            items.remove(at);
        }
    }
    dsh_put_default(&mut items, model);
    Ok(dsh_join(&head, &items))
}

/// dsh 的接线快照：agent-default-model 条目原行块 + providers.jolu 原行块。
/// 路由块只在**不是**指向本机网关时才存（指向网关 = 我们或别人替我们写的，断开时直接删，
/// 不能当「用户原块」放回去——本机接手前已有手工写的 jolu 块，正是这个场景）。
pub(crate) fn dsh_snapshot(text: &str, port: u16) -> Option<serde_json::Value> {
    let (_, items) = dsh_parse(text);
    let default = dsh_find(&items, "agent-default-model")
        .filter(|&at| !dsh_item_ours(&items[at]))
        .map(|at| serde_json::Value::Array(items[at].lines.iter().map(|l| serde_json::Value::String(l.clone())).collect()));
    let route = dsh_find(&items, "llm-pi-ai").and_then(|at| {
        let lines = &items[at].lines;
        dsh_block_range(lines, 4, "providers").and_then(|(prov_at, prov_end)| {
            dsh_block_range(&lines[prov_at..=prov_end], 6, DSH_PROVIDER).map(|(s, e)| (prov_at + s, prov_at + e))
        }).filter(|(s, _)| {
            let base = lines[*s..].iter().find_map(|line| {
                let trimmed = line.trim_start();
                trimmed.strip_prefix("baseURL:").map(|value| value.trim().trim_matches(['\'', '"']).to_string())
            });
            !base.as_deref().is_some_and(|base| points_at_gateway(base, port))
        }).map(|(s, e)| {
            serde_json::Value::Array(lines[s..=e].iter().map(|l| serde_json::Value::String(l.clone())).collect())
        })
    });
    (default.is_some() || route.is_some()).then(|| serde_json::json!({ "default": default, "route": route }))
}

/// 断开：删我们的路由子块与 agent-default-model（是我们的才动），快照里的用户原块放回。
pub(crate) fn dsh_patch_restore(text: &str, saved: Option<&serde_json::Value>) -> Result<String, String> {
    let (head, mut items) = dsh_parse(text);
    if items.is_empty() {
        return Ok(text.to_string());
    }
    // 还原 agent-default-model：是我们的条目 → 快照有原块就放回，没有就删
    if let Some(at) = dsh_find(&items, "agent-default-model") {
        if dsh_item_ours(&items[at]) {
            let original = saved.and_then(|saved| saved.get("default")).and_then(|value| value.as_array()).map(|lines| {
                DshItem {
                    id: "agent-default-model".to_string(),
                    lines: lines.iter().filter_map(|value| value.as_str().map(str::to_string)).collect(),
                }
            });
            match original {
                Some(block) if !block.lines.is_empty() => items[at] = block,
                _ => {
                    items.remove(at);
                }
            }
        }
    }
    // 换我们的路由子块：快照里存了用户同名块（非网关地址）就放回，否则直接删除
    if let Some(at) = dsh_find(&items, "llm-pi-ai") {
        let lines = &items[at].lines;
        let ours = dsh_block_range(lines, 4, "providers").and_then(|(prov_at, prov_end)| {
            dsh_block_range(&lines[prov_at..=prov_end], 6, DSH_PROVIDER).map(|(s, e)| (prov_at + s, prov_at + e))
        });
        if let Some((s, e)) = ours {
            let original = saved.and_then(|saved| saved.get("route")).and_then(|value| value.as_array()).map(|lines| {
                lines.iter().filter_map(|value| value.as_str().map(str::to_string)).collect::<Vec<_>>()
            });
            let mut new_lines = items[at].lines[..s].to_vec();
            match original {
                Some(block) if !block.is_empty() => new_lines.extend(block),
                _ => {}
            }
            new_lines.extend(items[at].lines[e + 1..].to_vec());
            items[at].lines = new_lines;
            if dsh_prune_pi(&mut items[at]) {
                items.remove(at);
            }
        }
    }
    Ok(dsh_join(&head, &items))
}

/// dsh 的令牌读取：`.env` 里的 `JOLU_API_KEY`（wire 时写入）。
pub(crate) fn dsh_current_token(env_text: &str) -> Option<String> {
    dotenv_get(env_text, DSH_KEY_ENV)
}

/// dsh 状态判定：默认模型指向我们的路由 + 路由 baseURL 指向本机网关 + 令牌是我们的。
pub(crate) fn dsh_wired(patch_text: &str, env_text: &str, port: u16, instance_short: &str, kept: Option<&str>) -> bool {
    let (_, items) = dsh_parse(patch_text);
    let model = dsh_default_model(&items);
    let base = dsh_route_base(&items);
    let token = dsh_current_token(env_text).unwrap_or_default();
    model.is_some()
        && points_at_gateway(base.as_deref().unwrap_or(""), port)
        && (token == token_for("dsh", instance_short)
            || token.starts_with("jolu-")
            || kept.is_some_and(|kept| !kept.is_empty() && token == kept))
}

/// credentials.yaml 的 refs 块写 / 替换我们的键（行级；`records` 等其它块一行不动）。
/// 只认 version: 1 布局（magpie dshCredsOurs：别的布局是 dsh 自己的，不碰）。
pub(crate) fn dsh_creds_wire(text: &str, token: &str) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(format!("version: 1\nrefs:\n  {DSH_KEY_ENV}: {token}\n"));
    }
    if !text.lines().any(|line| line.trim() == "version: 1") {
        return Ok(text.to_string());
    }
    let line = format!("  {DSH_KEY_ENV}: {token}");
    let mut out: Vec<String> = Vec::new();
    let mut in_refs = false;
    let mut wrote = false;
    for line_existing in text.lines() {
        let indent = line_existing.len() - line_existing.trim_start().len();
        if line_existing.trim_start().starts_with("refs:") && indent == 0 {
            in_refs = true;
            out.push(line_existing.to_string());
            continue;
        }
        if in_refs && indent == 0 && !line_existing.trim().is_empty() {
            in_refs = false;
        }
        if in_refs && line_existing.trim_start().starts_with(&format!("{DSH_KEY_ENV}:")) {
            out.push(line.clone());
            wrote = true;
            continue;
        }
        out.push(line_existing.to_string());
    }
    if !wrote {
        // refs 块末尾插入：找到 refs: 之后第一个缩进 0 的行，插在它之前
        let mut result: Vec<String> = Vec::new();
        let mut inserted = false;
        let mut seen_refs = false;
        for line_existing in out {
            let indent = line_existing.len() - line_existing.trim_start().len();
            let top = !line_existing.trim().is_empty() && indent == 0;
            if seen_refs && !inserted && top {
                result.push(line.clone());
                inserted = true;
            }
            if line_existing.trim_start().starts_with("refs:") && indent == 0 {
                seen_refs = true;
            }
            result.push(line_existing);
        }
        if !inserted {
            result.push(line);
        }
        out = result;
    }
    let mut joined = out.join("\n");
    if !joined.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

/// credentials.yaml 的 refs 块删我们的键：值是我们的（jolu- 前缀）才删，用户的键不动。
pub(crate) fn dsh_creds_restore(text: &str) -> String {
    let prefix = format!("  {DSH_KEY_ENV}: ");
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix(&prefix) {
            if value.trim().trim_matches(['\'', '"']).starts_with("jolu-") {
                continue;
            }
        }
        out.push(line.to_string());
    }
    let mut joined = out.join("\n");
    if !joined.ends_with('\n') && !joined.is_empty() {
        joined.push('\n');
    }
    joined
}


/* ─────────────────────────── 对外接口（wiring.rs 的分发点调用） ─────────────────────────── */

/// 主配置文件 = 第一个 profile 的 patch（探测与接线同源，files[0] 必须与它一致）。
pub(crate) fn config_path(home: &Path) -> PathBuf {
    dsh_main_file(&dsh_dir(home))
}

/// patch 里我们路由的 baseURL（状态判定与「外部已改就不覆盖」共用）。
pub(crate) fn current_base(text: &str) -> Option<String> {
    dsh_route_base(&dsh_parse(text).1)
}

/// 令牌：`.env` 里的 JOLU_API_KEY（接线时写入；credentials refs 由 dsh 自己优先读，
/// 但那里只该有用户自己的键——我们的键同时写在两处，读 .env 即可判定归属）。
pub(crate) fn current_token(home: &Path) -> Option<String> {
    let text = crate::steady::read_optional(&dsh_dir(home).join(".env")).ok().flatten().unwrap_or_default();
    dsh_current_token(&text)
}


/// 状态判定：patch 两项 + .env 令牌归属。kept 是旧接线记录里保留的令牌。
pub(crate) fn wired(home: &Path, text: &str, port: u16, instance_short: &str, kept: Option<&str>) -> bool {
    let env_text = crate::steady::read_optional(&dsh_dir(home).join(".env")).ok().flatten().unwrap_or_default();
    dsh_wired(text, &env_text, port, instance_short, kept)
}


/// 接线计划（多文件）：主 patch + 其余 profile patch + .env + credentials。
pub(crate) struct DshWirePlan {
    /// 主 patch（第一个 profile）的新内容；路径由 `config_path` 给出（同一函数，无需重复存）
    pub main_text: String,
    pub extra: Vec<(PathBuf, String)>,
    pub snapshot: Option<Value>,
}

pub(crate) fn plan_wire_files(
    home: &Path,
    base: &str,
    token: &str,
    model: Option<&str>,
    catalog: &[ZcodeModel],
) -> Result<DshWirePlan, String> {
    let dir = dsh_dir(home);
    let profiles = dsh_profiles(&dir);
    let main_path = config_path(home);
    let selected = model.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string).or_else(|| catalog.first().map(|first| first.id.clone())).ok_or("网关模型清单为空；先在「模型池」里启用要提供给 DeepSeek Harness 的模型")?;
    let port = base.rsplit_once(':').and_then(|(_, port)| port.parse::<u16>().ok()).unwrap_or(0);
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut snapshots: Vec<Value> = Vec::new();
    if profiles.is_empty() {
        files.push((main_path.clone(), crate::steady::read_optional(&main_path).ok().flatten().unwrap_or_default()));
    } else {
        for path in &profiles {
            let text = crate::steady::read_optional(path).ok().flatten().unwrap_or_default();
            files.push((path.clone(), text));
        }
    }
    for (path, text) in &files {
        if let Some(snap) = dsh_snapshot(text, port) {
            snapshots.push(json!({ "path": path.to_string_lossy(), "default": snap["default"], "route": snap["route"] }));
        }
    }
    // 用户原来在 .env 里放过的 JOLU_API_KEY（非我们的令牌）存进快照，断开时放回
    let env_path = dir.join(".env");
    let env_text = crate::steady::read_optional(&env_path).ok().flatten().unwrap_or_default();
    let env_saved = dotenv_get(&env_text, DSH_KEY_ENV).filter(|value| !value.starts_with("jolu-"));
    // credentials 只认 version: 1 布局；用户原有的 refs.JOLU_API_KEY（非我们的）也存
    let creds_path = dir.join(".credentials.yaml");
    let creds_text = crate::steady::read_optional(&creds_path).ok().flatten().unwrap_or_default();
    let creds_saved = creds_refs_value(&creds_text, DSH_KEY_ENV)
        .filter(|value| !value.trim_matches(['\'', '"']).starts_with("jolu-"));
    let snapshot = (!snapshots.is_empty() || env_saved.is_some() || creds_saved.is_some()).then(|| {
        json!({ "files": snapshots, "env": env_saved, "creds": creds_saved })
    });
    // 逐个写 patch
    let mut written: Vec<(PathBuf, String)> = Vec::new();
    for (path, text) in &files {
        written.push((path.clone(), dsh_patch_wire(text, base, &selected, catalog)?));
    }
    let main_text = written.remove(0).1;
    // .env：写我们的令牌
    let mut extra = written;
    extra.push((env_path, wire_dotenv(&env_text, DSH_KEY_ENV, Some(token))));
    // credentials：refs 下写我们的键（version 1 才写，别的布局是 dsh 的不动）
    let creds_next = dsh_creds_wire(&creds_text, token)?;
    if creds_next != creds_text {
        extra.push((creds_path, creds_next));
    }
    Ok(DshWirePlan { main_text, extra, snapshot })
}

/// 断开（多文件）：每个 profile 还原快照原块，.env / credentials 还原用户的键或删我们的。
pub(crate) fn plan_unwire_files(home: &Path, saved: Option<&Value>) -> Result<Vec<(PathBuf, String)>, String> {
    let dir = dsh_dir(home);
    let files = dsh_profiles(&dir);
    let per_file = |path: &Path| {
        saved.and_then(|saved| saved.get("files")).and_then(Value::as_array).and_then(|list| {
            list.iter().find(|entry| entry.get("path").and_then(Value::as_str).is_some_and(|p| p == path.to_string_lossy()))
        })
    };
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    for path in &files {
        let text = crate::steady::read_optional(path).ok().flatten().unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let file_saved = per_file(path);
        let next = dsh_patch_restore(&text, file_saved)?;
        if next != text {
            out.push((path.clone(), next));
        }
    }
    // .env：快照里存过用户的值就放回；否则仍是我们的令牌才删
    let env_path = dir.join(".env");
    if let Ok(Some(env_text)) = crate::steady::read_optional(&env_path) {
        let saved_env = saved.and_then(|saved| saved.get("env")).and_then(Value::as_str).map(str::to_string);
        let ours = dsh_current_token(&env_text).is_some_and(|value| value.starts_with("jolu-"));
        let next = if ours {
            wire_dotenv(&env_text, DSH_KEY_ENV, saved_env.as_deref())
        } else {
            env_text.clone()
        };
        if next != env_text {
            out.push((env_path, next));
        }
    }
    // credentials：只在当前仍是我们的键时恢复快照，保留接线后用户自行修改的值。
    let creds_path = dir.join(".credentials.yaml");
    if let Ok(Some(creds_text)) = crate::steady::read_optional(&creds_path) {
        let ours = creds_refs_value(&creds_text, DSH_KEY_ENV)
            .is_some_and(|value| value.trim_matches(['\'', '"']).starts_with("jolu-"));
        let saved_creds = saved.and_then(|saved| saved.get("creds")).and_then(Value::as_str);
        let next = if ours {
            match saved_creds {
                Some(value) => dsh_creds_wire(&creds_text, value)?,
                None => dsh_creds_restore(&creds_text),
            }
        } else {
            creds_text.clone()
        };
        if next != creds_text {
            out.push((creds_path, next));
        }
    }
    Ok(out)
}

/// credentials refs 块里某键的值（行级；version 1 之外的布局返回 None 不碰）。
fn creds_refs_value(text: &str, key: &str) -> Option<String> {
    if !text.lines().any(|line| line.trim() == "version: 1") {
        return None;
    }
    let prefix = format!("  {key}: ");
    let mut in_refs = false;
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        if line.trim_start().starts_with("refs:") && indent == 0 {
            in_refs = true;
            continue;
        }
        if in_refs && indent == 0 && !line.trim().is_empty() {
            in_refs = false;
        }
        if in_refs {
            if let Some(value) = line.strip_prefix(&prefix) {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机 desktop profile 的形状缩影（掩掉密钥值，其余照抄）：用户的 hualong 块、
    /// ui 条目、以及接手前已手工写好的 jolu 块与 agent-default-model。
    fn local_desktop() -> String {
        [
            "# Your patch layer for this dsh profile, applied after every bundle layer:",
            "- id: ui-theme",
            "  name: \"@deepseek-ai/dsh-client-ui-theme\"",
            "  config:",
            "    preference: system",
            "- id: agent-default-model",
            "  name: \"@deepseek-ai/dsh-agent-default-model\"",
            "  config:",
            "    provider: jolu",
            "    model: zcode/GLM-5.3-Flash",
            "    reasoningEffort: max",
            "- id: llm-pi-ai",
            "  name: \"@deepseek-ai/dsh-llm-pi-ai\"",
            "  config:",
            "    providers:",
            "      hualong:",
            "        displayName: 华龙",
            "        apiKeyEnv: HUALONG_API_KEY",
            "        api: openai-responses",
            "        baseURL: https://api-fast.hualong-ai.com/",
            "        models:",
            "          - id: deepseek-v4.1-flash-1017",
            "            name: deepseek-v4.1-flash-1017",
            "      jolu:",
            "        apiKeyEnv: JOLU_API_KEY",
            "        api: openai-completions",
            "        baseURL: http://127.0.0.1:12731",
            "        models:",
            "          - id: zcode/GLM-5.3-Flash",
            "            name: Zcode · GLM-5.3-Flash",
        ].join("\n")
    }

    fn catalog() -> Vec<ZcodeModel> {
        vec![ZcodeModel { id: "zcode/GLM-5.3-Flash".into(), name: "GLM-5.3-Flash".into(), context: 200_000, output: 32_768, image: true }]
    }

    #[test]
    fn patch_parse_keeps_user_entries_intact() {
        let text = local_desktop();
        let (head, items) = dsh_parse(&text);
        assert!(head[0].starts_with("# Your patch layer"), "文件头注释保留：{:?}", head);
        assert_eq!(items.len(), 3, "三个条目：ui-theme / agent-default-model / llm-pi-ai");
        assert_eq!(dsh_find(&items, "llm-pi-ai"), Some(2));
        // 拼回去要逐字还原（行级拼接的底线；dsh_join 保证文件以换行结尾）
        assert_eq!(dsh_join(&head, &items), text + "
");
    }

    #[test]
    fn wire_rewrites_route_but_keeps_hualong_and_ui() {
        let original = local_desktop();
        let wired = dsh_patch_wire(&original, "http://127.0.0.1:12731", "zcode/GLM-5.3-Flash", &tests::catalog()).unwrap();
        // 用户的块一行不动
        assert!(wired.contains("hualong:"), "用户的 hualong 供应商要保留：{wired}");
        assert!(wired.contains("baseURL: https://api-fast.hualong-ai.com/"), "hualong 的地址不动");
        assert!(wired.contains("preference: system"), "ui 条目不动");
        // 我们的块按 magpie 形状重写：baseURL 带 /v1、maxTokens 按压缩公式
        assert!(wired.contains("baseURL: http://127.0.0.1:12731/v1"), "OpenAI 路由要带 /v1（magpie dshBaseURL）：{wired}");
        assert!(wired.contains("maxTokens:"), "目录给了输出上限要写 maxTokens");
        assert!(wired.contains("input: [text, image]"), "收图模型用 flow 数组");
        assert!(!wired.contains("reasoningEfforts"), "目录无档位数据时不写推理档位");
        assert!(wired.contains("provider: jolu") && wired.contains("model: zcode/GLM-5.3-Flash"), "默认模型指向网关");
        // 状态判定
        let env = format!("{}=jolu-dsh.abcd1234", DSH_KEY_ENV);
        assert!(dsh_wired(&wired, &env, 12731, "abcd1234", None));
    }

    #[test]
    fn unwire_removes_jolu_and_keeps_everything_else() {
        let original = local_desktop();
        let wired = dsh_patch_wire(&original, "http://127.0.0.1:12731", "zcode/GLM-5.3-Flash", &tests::catalog()).unwrap();
        // 本机场景：接手前已有人手工写了 jolu 块（指向网关）——快照不把它当用户原块
        let snap = dsh_snapshot(&original, 12731);
        let restored = dsh_patch_restore(&wired, snap.as_ref()).unwrap();
        assert!(!restored.contains("127.0.0.1:12731"), "我们的路由要删干净：{restored}");
        assert!(restored.contains("hualong:"), "用户的 hualong 保留：{restored}");
        assert!(restored.contains("preference: system"), "ui 条目保留：{restored}");
        // 接手前的手工 agent-default-model（provider=jolu=ours）没有用户原值 → 删除
        assert!(!restored.contains("agent-default-model"), "手工写的默认模型条目没有用户原块，删除：{restored}");
        // 接过线再断开（快照为空）同样干净
        let wired2 = dsh_patch_wire(&original, "http://127.0.0.1:12731", "zcode/GLM-5.3-Flash", &catalog()).unwrap();
        let restored2 = dsh_patch_restore(&wired2, None).unwrap();
        assert!(!restored2.contains("127.0.0.1:12731/v1"), "{restored2}");
    }

    #[test]
    fn unwire_restores_a_user_default_model_and_foreign_route() {
        // 用户自己在用别的 provider + 自己命名过一个 jolu 路由（不在网关上）→ 快照要存
        let original = [
            "- id: agent-default-model",
            "  name: \"@deepseek-ai/dsh-agent-default-model\"",
            "  config:",
            "    provider: deepseek-official",
            "    model: deepseek-chat",
            "- id: llm-pi-ai",
            "  name: \"@deepseek-ai/dsh-llm-pi-ai\"",
            "  config:",
            "    providers:",
            "      jolu:",
            "        displayName: 我的自建",
            "        baseURL: https://my-own.example.com",
        ].join("\n");
        let snap = dsh_snapshot(&original, 12731).expect("用户的默认模型与自建路由都要进快照");
        let wired = dsh_patch_wire(&original, "http://127.0.0.1:12731", "zcode/GLM-5.3-Flash", &tests::catalog()).unwrap();
        assert!(wired.contains("provider: jolu") && wired.contains("model: zcode/GLM-5.3-Flash"));
        let restored = dsh_patch_restore(&wired, Some(&snap)).unwrap();
        assert!(restored.contains("provider: deepseek-official") && restored.contains("model: deepseek-chat"), "用户的默认模型放回：{restored}");
        assert!(restored.contains("https://my-own.example.com"), "用户自建的同名路由放回：{restored}");
        assert!(!restored.contains("127.0.0.1:12731"), "我们的路由删除：{restored}");
    }

    #[test]
    fn web_patch_with_unrelated_entries_is_untouched_by_unwire() {
        // web profile 只有用户自己的 MCP 条目（无 llm-pi-ai / agent-default-model）：
        // 接线插入我们的条目，断开删除后逐字还原
        let original = [
            "# 用户自己的 MCP 配置",
            "- insert:",
            "    - id: mcp-anysearch",
            "      name: '@deepseek-ai/dsh-mcp-client'",
            "      config:",
            "        serverName: anysearch",
        ].join("\n");
        let wired = dsh_patch_wire(&original, "http://127.0.0.1:12731", "zcode/GLM-5.3-Flash", &tests::catalog()).unwrap();
        assert!(wired.contains("mcp-anysearch"), "用户条目保留：{wired}");
        assert!(wired.contains("- id: llm-pi-ai # jolu"), "我们的条目追加在后");
        let restored = dsh_patch_restore(&wired, None).unwrap();
        assert_eq!(restored, original + "
", "断开后逐字还原（写盘文件以换行结尾）");
    }

    #[test]
    fn creds_and_env_round_trip() {
        let creds = "version: 1\nrefs:\n  OTHER: user-key\nrecords:\n  - id: 1\n";
        let wired = dsh_creds_wire(creds, "jolu-dsh.abcd1234").unwrap();
        assert!(wired.contains("  JOLU_API_KEY: jolu-dsh.abcd1234"), "{wired}");
        assert!(wired.contains("  OTHER: user-key"), "用户键保留");
        assert!(wired.contains("records:"), "records 块不动");
        let restored = dsh_creds_restore(&wired);
        assert!(!restored.contains("JOLU_API_KEY"), "我们的键删除：{restored}");
        assert!(restored.contains("  OTHER: user-key"), "用户键还在");
        // 别的布局不碰（magpie dshCredsOurs）
        let foreign_layout = "kind: v2\nrefs: []\n";
        assert_eq!(dsh_creds_wire(foreign_layout, "jolu-dsh.x").unwrap(), foreign_layout);
    }

    #[test]
    fn max_tokens_leaves_half_the_window_to_compact() {
        // magpie #2823/#3298：输出上限贴近窗口时压到 window/2 − 65536
        assert_eq!(dsh_max_tokens(1_000_000, 500_000), 434_464);
        // 窗口不足 262144 没有可让的一半 → 退到 1/4
        assert_eq!(dsh_max_tokens(131_072, 131_072), 32_768);
        // 输出远小于窗口 → 原值
        assert_eq!(dsh_max_tokens(1_000_000, 32_768), 32_768);
    }
}
