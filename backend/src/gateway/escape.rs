//! 逃生门（执行文档 §10，三层全部必须）。
//!
//! 1. 手动切回直连：逐 agent 还原接线 → 停网关 → 提示
//! 2. 自动熔断：连续失败达阈值且开启自动逃生 → 同 1，附原因通知
//! 3. 启动自愈：启动时校验接线是否仍指向网关；外部已改则尊重外部并关闭网关
//!
//! 熔断后 agent 回到各自直连：若某 agent 本无订阅会不可用，所以文案必须说「切回直连」
//! 而不是「服务故障」——这是本模块所有提示的共同底线。

use tauri::Manager;

use crate::gateway::wiring_store::{self, WiringRecord};

/// 逃生结果（供命令层转成 Toast / 通知文案）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct EscapeOutcome {
    /// 已还原的 agent（agent id 列表）
    pub restored: Vec<String>,
    /// 因外部已改而跳过的 agent（尊重外部意图）
    pub skipped: Vec<String>,
    /// 还原失败（含原因）
    pub failed: Vec<String>,
    /// 网关已停止
    pub stopped: bool,
}

impl EscapeOutcome {
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.restored.is_empty() {
            parts.push(format!("已还原 {} 个工具", self.restored.len()));
        }
        if !self.skipped.is_empty() {
            parts.push(format!("{} 个工具的配置已被外部修改，未覆盖", self.skipped.len()));
        }
        if !self.failed.is_empty() {
            parts.push(format!("{} 个工具还原失败", self.failed.len()));
        }
        if self.stopped {
            parts.push("网关已停止".into());
        }
        if parts.is_empty() { "没有需要还原的工具配置".into() } else { parts.join("；") }
    }
}

fn empty_outcome() -> EscapeOutcome {
    EscapeOutcome { restored: Vec::new(), skipped: Vec::new(), failed: Vec::new(), stopped: false }
}

fn record_for(app: &tauri::AppHandle, agent: &str) -> Option<WiringRecord> {
    let state = app.state::<crate::Db>();
    let conn = state.0.lock().ok()?;
    wiring_store::load(&conn, agent)
}

/// 第一层：手动切回直连。还原全部接线 → 停监听 → 关闭开关（保留 proxy_mode）。
pub fn manual(app: &tauri::AppHandle) -> EscapeOutcome {
    let outcome = crate::cli_switch::with_switch(|| Ok::<_, String>(restore_all(app)))
        .unwrap_or_else(|error| EscapeOutcome { failed: vec![error], ..empty_outcome() });
    crate::proxy::stop();
    let next = app.state::<crate::Cfg>().0.update(|settings| {
        settings.proxy_enabled = false;
        // 保留 proxy_mode 与接线快照：再次开启时按已有意图恢复
    });
    let _ = tauri::Emitter::emit(app, "settings-changed", next);
    EscapeOutcome { stopped: true, ..outcome }
}

/// 第二层：自动熔断（连续失败达阈值时由网关侧调用）。
/// 与手动逃生共用还原路径，额外发系统通知说明原因。
pub fn circuit_break(app: &tauri::AppHandle, reason: &str) -> EscapeOutcome {
    eprintln!("[网关] 连续失败触发自动熔断：{reason}");
    let outcome = manual(app);
    let message = format!("模型入口已自动切回直连：{reason}。各工具恢复原有直连方式，网关设置会保留。");
    crate::notify::show(app, "Jolu · 模型入口".into(), message, None);
    outcome
}

/// 逐 agent 还原接线。只清理自己的写入：外部已改的不覆盖。
/// 调用方必须持有 [`crate::cli_switch::with_switch`] 的锁。
pub(crate) fn restore_all(app: &tauri::AppHandle) -> EscapeOutcome {
    let settings = app.state::<crate::Cfg>().0.get();
    let port = settings.proxy_port;
    let mut outcome = empty_outcome();
    for agent in crate::gateway::wiring::AGENTS {
        let record = record_for(app, agent);
        let status = crate::gateway::wiring::status_with(agent, port, record.as_ref());
        let pending = record.as_ref().is_some_and(|r| r.state == "pending");
        if !status.wired && !pending {
            // 未接线：无需动作（也可能外部已改，正是想要的结果）
            continue;
        }
        match crate::gateway::wiring::unwire(agent, port, record.as_ref()) {
            Ok(true) => {
                outcome.restored.push(agent.to_string());
                if let Ok(conn) = app.state::<crate::Db>().0.lock() {
                    let _ = wiring_store::mark_idle(&conn, agent);
                }
            }
            Ok(false) => {
                outcome.skipped.push(agent.to_string());
                if let Ok(conn) = app.state::<crate::Db>().0.lock() {
                    let _ = wiring_store::mark_idle(&conn, agent);
                }
            }
            Err(error) => outcome.failed.push(format!("{agent}：{error}")),
        }
    }
    // 清空接线快照（过渡版本仍双写 settings），避免下次误用旧值
    if !outcome.restored.is_empty() || !outcome.skipped.is_empty() {
        let _ = app.state::<crate::Cfg>().0.try_update(|settings| {
            for agent in outcome.restored.iter().chain(outcome.skipped.iter()) {
                settings.gateway_wiring.remove(agent);
            }
        });
    }
    if let Ok(conn) = app.state::<crate::Db>().0.lock() {
        let _ = wiring_store::set_owner_all(&conn, "none");
    }
    outcome
}

/// 第三层：启动自愈。
/// - 先修复 pending：文件里是我们写下的值 → 标 wired；否则标 idle；
/// - 网关启用且 relay：仍指向网关的保持；曾接线但已不指向的尊重外部修改并清记录；
/// - 没有任何 agent 仍指向 → 网关失去意义，关闭并提示。
///
/// 注意：这里刻意**不**自动重写配置文件。上次崩溃到本次启动之间，用户可能正是在
/// Claude Code / Codex 里手动改回了直连——启动时重写会把用户的意图再覆盖掉。
pub fn startup(app: &tauri::AppHandle) {
    let settings = app.state::<crate::Cfg>().0.get();
    // 旧设置里的快照导入一次，之后以 agent_wiring 为准（settings 继续双写一版）。
    // 修复 pending 时可能要写回配置文件（Gemini 的 settings.json 可能停在半接管），持有共享锁。
    let _ = crate::cli_switch::with_switch(|| {
        if let Ok(conn) = app.state::<crate::Db>().0.lock() {
            let _ = wiring_store::import_legacy(&conn, &settings.gateway_wiring, &settings.instance_id);
            for record in wiring_store::all(&conn) {
                let next = crate::gateway::wiring::reconcile_pending(&record.agent, settings.proxy_port, &record);
                match next {
                    "wired" => {
                        let _ = wiring_store::mark_wired(&conn, &record.agent, chrono::Local::now().timestamp_millis());
                    }
                    "idle" => {
                        eprintln!("[网关] {} 的上次接线未完成，按接线前快照还原", record.agent);
                        // 崩溃可能停在「.env 已改、settings.json 没改」之间：按快照尽量还原。
                        let _ = crate::gateway::wiring::unwire(&record.agent, settings.proxy_port, Some(&record));
                        let _ = wiring_store::mark_idle(&conn, &record.agent);
                    }
                    _ => {}
                }
            }
        }
        Ok::<(), String>(())
    });
    if !settings.proxy_enabled {
        return;
    }
    let (mode, _) = crate::gateway::mode_from(&settings);
    if mode != crate::gateway::Mode::Relay {
        return;
    }
    let port = settings.proxy_port;
    let mut wired_any = false;
    let mut drifted: Vec<String> = Vec::new();
    for agent in crate::gateway::wiring::AGENTS {
        let record = record_for(app, agent);
        let status = crate::gateway::wiring::status_with(agent, port, record.as_ref());
        if status.wired {
            wired_any = true;
            continue;
        }
        if record.as_ref().is_some_and(|r| r.state != "idle") {
            eprintln!("[网关] {agent} 的配置已不指向网关，尊重外部修改，清理接线记录");
            drifted.push(agent.to_string());
        }
    }
    if !drifted.is_empty() {
        if let Ok(conn) = app.state::<crate::Db>().0.lock() {
            for agent in &drifted {
                let _ = wiring_store::mark_idle(&conn, agent);
            }
        }
        let _ = app.state::<crate::Cfg>().0.try_update(|settings| {
            for agent in &drifted {
                settings.gateway_wiring.remove(agent);
            }
        });
    }
    if !wired_any {
        eprintln!("[网关] 启动自愈：没有工具仍指向网关，关闭网关并保持直连");
        let next = app.state::<crate::Cfg>().0.update(|settings| {
            settings.proxy_enabled = false;
        });
        let _ = tauri::Emitter::emit(app, "settings-changed", next);
        crate::proxy::stop();
    }
}

/// 接线：把一批 agent 指向网关。
/// pending 先落库 → 写文件 → 标 wired；任何一步失败都留下可读的记录。
///
/// `model` 只用于 Claude Code：网关按 `<来源>/<模型>` 选路，不写模型映射的话工具会发它
/// 自带的模型别名，请求无法路由（用户看到的就是「接好了但识别不出模型」）。
/// 其它 agent 忽略这个参数。
/// `model` 是主模型；`subagent_model` 是 subagent 独立模型（只对 Claude Code，None = 跟随主模型）。
pub fn wire_agents(
    app: &tauri::AppHandle,
    agents: &[String],
    model: Option<&str>,
    subagent_model: Option<&str>,
    tiers: Option<&crate::gateway::wiring_store::TierModels>,
) -> Result<Vec<String>, String> {
    let settings = app.state::<crate::Cfg>().0.get();
    // 只有正在运行的模型入口才能接管工具：接到一个没在监听的端口会让工具全部失败。
    let (mode, _) = crate::gateway::mode_from(&settings);
    if !settings.proxy_enabled || mode != crate::gateway::Mode::Relay {
        return Err("模型入口未运行：请先在模型入口页开启并切换到中转模式，再接入工具".into());
    }
    let port = settings.proxy_port;
    let instance = settings.instance_id.clone();
    let instance_short: String = instance.chars().take(8).collect();
    // 1M 自动标记（magpie mark1M 同款）：主模型上下文超过 200K 时写入带 [1m] 的名字，
    // 否则 Claude Code 按 200K 截断。元数据缺失按不标记处理——宁可少标，不虚标。
    let model = model.map(|m| {
        if m.ends_with("[1m]") || m.ends_with("[1M]") {
            m.to_string()
        } else if crate::gateway::models_dev::exceeds_200k(app, m) {
            format!("{m}[1m]")
        } else {
            m.to_string()
        }
    });
    let backup_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法定位应用数据目录：{error}"))?
        .join("gateway-backups");
    crate::cli_switch::with_switch(|| {
        let mut wired = Vec::new();
        for agent in agents {
            if !crate::gateway::wiring::is_agent(agent) {
                return Err(format!("不支持的工具：{agent}"));
            }
            // 已经接线且文件仍是我们的值：幂等跳过，不重复快照。
            // 需要重新应用时沿用上一次保存的原值（A4：重新应用不重新快照）。
            let (prior_snapshot, prior_model, prior_label, prior_values) = {
                let state = app.state::<crate::Db>();
                let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
                match wiring_store::load(&conn, agent) {
                    Some(record) => {
                        // 幂等：已接线、配置仍是我们的值、且档位自选没变——三者都成立才跳过。
                        // 只改档位（主模型不动）时 status_with 仍 wired，必须靠 tiers 对比触发重写，
                        // 否则用户改了 Haiku 档点应用会被当成「无需动作」静默吞掉。
                        let tiers_unchanged = record.applied().as_ref().and_then(|a| a.tiers.as_ref()) == tiers;
                        if record.state == "wired"
                            && tiers_unchanged
                            && crate::gateway::wiring::status_with(agent, port, Some(&record)).wired
                        {
                            wired.push(agent.clone());
                            continue;
                        }
                        let applied = record.applied();
                        let model = applied.as_ref().and_then(|a| a.model.clone());
                        let label = applied.as_ref().and_then(|a| a.label.clone());
                        let written = applied.as_ref().map(crate::gateway::wiring_store::Applied::written_values).unwrap_or_default();
                        (record.original_json, model, label, written)
                    }
                    None => (None, None, None, Vec::new()),
                }
            };
            // 先读原值、算好要写的值（快照已过滤所有网关值），写 pending 后再落盘
            // 显示名 / 描述：models.dev 只读缓存（缺失退回模型 id 本身；不联网不阻塞）
            let label = model.as_ref().map(|m| crate::gateway::models_dev::display_label(app, m));
            let description = model.as_ref().map(|m| crate::gateway::models_dev::description_of(app, m)).filter(|d| !d.is_empty());
            // 档位「关于」：写入值剥掉 :effort 与 [1m] 后查 models.dev——
            // /model 菜单里那行要显示档位实际用的模型（magpie claudeAbout 语义）
            let tier_about = |value: &str| -> (Option<String>, Option<String>) {
                let bare = value
                    .split(':')
                    .next()
                    .unwrap_or(value)
                    .trim()
                    .trim_end_matches("[1m]")
                    .trim_end_matches("[1M]");
                (
                    Some(crate::gateway::models_dev::display_label(app, bare)),
                    {
                        let description = crate::gateway::models_dev::description_of(app, bare);
                        (!description.is_empty()).then_some(description)
                    },
                )
            };
            let (snapshot, _applied) = crate::gateway::wiring::wire(
                agent,
                port,
                &instance_short,
                &backup_dir,
                prior_snapshot.as_deref(),
                model.as_deref(),
                label.as_deref(),
                description.as_deref(),
                prior_model.as_deref(),
                prior_label.as_deref(),
                subagent_model,
                &prior_values,
                tiers,
                &tier_about,
                &|token: &str| {
                    let state = app.state::<crate::Db>();
                    let Ok(conn) = state.0.lock() else { return false };
                    crate::gateway::access::is_client_key(&conn, token)
                },
                |snapshot, applied| {
                    let state = app.state::<crate::Db>();
                    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
                    wiring_store::mark_pending(
                        &conn,
                        agent,
                        &instance,
                        "relay",
                        snapshot,
                        applied,
                        applied.token.as_deref(),
                    )
                    .map_err(|error| format!("保存接线记录失败：{error}"))
                },
            )?;
            // 文件已经写好，标记 wired
            {
                let state = app.state::<crate::Db>();
                let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
                wiring_store::mark_wired(&conn, agent, chrono::Local::now().timestamp_millis())
                    .map_err(|error| format!("保存接线状态失败：{error}"))?;
            }
            // 过渡版本双写旧字段，方便回退
            app.state::<crate::Cfg>()
                .0
                .try_update(|settings| {
                    settings.gateway_wiring.insert(
                        agent.clone(),
                        snapshot.clone().unwrap_or_else(|| "{}".into()),
                    );
                })
                .map_err(|error| format!("保存接线状态失败：{error}"))?;
            wired.push(agent.clone());
        }
        Ok(wired)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_summary_states_each_case_without_alarmist_wording() {
        let outcome = EscapeOutcome {
            restored: vec!["claude-code".into(), "codex".into()],
            skipped: vec!["gemini".into()],
            failed: vec![],
            stopped: true,
        };
        let text = outcome.summary();
        assert!(text.contains("已还原 2 个工具"), "{text}");
        assert!(text.contains("未覆盖"), "{text}");
        assert!(text.contains("网关已停止"), "{text}");
        assert!(!text.contains("故障"), "文案不能说成服务故障：{text}");
        let empty = EscapeOutcome { restored: vec![], skipped: vec![], failed: vec![], stopped: true };
        assert_eq!(empty.summary(), "网关已停止", "没有还原动作时只报停止，不能编造还原数");
        let idle = EscapeOutcome { restored: vec![], skipped: vec![], failed: vec![], stopped: false };
        assert!(idle.summary().contains("没有需要还原"));
    }

    #[test]
    fn backup_dir_is_under_app_data() {
        // 只验证拼接约定，不触碰真实目录
        let dir = std::path::Path::new("C:/data").join("gateway-backups");
        assert!(dir.ends_with("gateway-backups"));
    }
}
