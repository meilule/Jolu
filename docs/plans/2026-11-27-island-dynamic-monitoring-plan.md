# 灵动岛动态监听 Agent 与账号使用实施计划（2026-11-27）

**定案组合**（鼠鼠拍板，对应 `docs/ui/jolu-new.pen` 画板 `R2-I-08` ～ `R2-I-11`）：
- **1A**：单岛「⚡ 智能跟随活跃 Agent / 账号」（`island_linkage.follow_active`）+ 多工具并发液体分离气泡轮换 + 展开态跨平台运行会话汇总。
- **2B**：直连多账号写入时刻按本机生效凭证指纹盖戳（`requests.attribution_source = 'active_config'`，`Confidence::ActiveConfig`，青色描边芯片诚实区分于网关确证 `exact`）。
- **3A（含陌生 Key 防呆）**：本机配置文件切换账号（CC Switch / CLI 重新登录）经 500ms 稳定去抖后，开启智能跟随的本体岛无感秒切 + 1.5s 轻提示并刷新新账号额度；遇到尚未录入 Jolu 的陌生凭证时置空额度环并展示脱敏掩码（`unregistered`），绝不张冠李戴显示旧账号额度。
- **4B（可选受控开关）**：提供可选开关「后台等待确认提醒（Claude Hooks / Codex Notify）」（`island_linkage.agent_hooks`，默认关），通过 `Jolu.exe --agent-hook <agent>` 零窗口无阻塞写回本机状态目录，关闭或卸载（`--restore-agents`）时按快照零残留还原。

## 分步任务

1. **设置与归因数据契约扩展**
   - `settings::types::IslandLinkage` 增加 `follow_active`（默认 `true`）与 `agent_hooks`（默认 `false`）。
   - `db::schema::run_column_migrations` 为 `requests` 增加可空列 `attribution_source TEXT`。
   - `usage::attribution` 新增 `Confidence::ActiveConfig`（序列化 `"active_config"`），保持 `Exact > ActiveConfig > Mapped > Platform` 四档语义清晰，严格模式下可降级。
2. **本机活跃凭证感知、2B 写入盖戳与 3A 陌生凭证防呆**
   - 新增 `auth::local_active`：维护各平台本机当前生效凭证的脱敏快照（`matched` / `unregistered` / `gateway_wired`），支持 OAuth 令牌刷新后的邮箱唯一回退匹配，明文绝不出后端。
   - `watcher::start_config_sync` 500ms 稳定去抖刷新快照并发出 `active-account-changed` 事件。
   - 采集器实时扫描（90s 内新记录且 `connection_id IS NULL`）按 `matched` 连接写入 `active_config` 归属。
3. **1A 全平台实时监听升级与观测代理在途事件**
   - `app::watcher` 从仅追踪 `settings.island_platform` 升级为追踪全平台 `active_sessions` 生命周期变化，补齐非主平台与分身岛的会话开始 / 结束实时推送。
   - 新增只读命令 `island_live_overview` 汇总跨平台活跃会话。
   - `proxy::observe` 在观测模式下同步发出 `RouteStart` / `RouteEnd` 实时在途事件。
4. **4B 可选轻量 Agent Hooks「⚑ 后台等待人工确认」**
   - 新增 `agents::hooks`：读写与快照还原 `~/.claude/settings.json`（`hooks` 的 `Notification` / `PermissionRequest` / `UserPromptSubmit` / `Stop`）与 `~/.codex/config.toml`（`notify`），接入 `main.rs` 的 `--agent-hook` 与 `--restore-agents`。
   - `app::watcher` 监听数据目录下的 `agent-hooks/`，实时发出 `agent-hook` 事件驱动灵动岛琥珀色高优提示。
5. **前端灵动岛智能跟随状态机、并发气泡、陌生凭证条与设置项**
   - 新增纯逻辑状态机 `lib/islandDynamicFollow.ts`（附 `.test.mjs` 单测）：处理空闲自动接管、多工具并发锁焦与气泡点击轮换、2.4s 完结交接、3A 切号 1.5s 提示、陌生 Key 防呆与 4B 等确认高优插队。
   - 接入 `useIslandWindowModel`、`CapsuleIsland`、`IslandConnectionSwitcher`、`IslandView` 与 `snapshotCommands`。
6. **全套验证**
   - `node scripts/dev/check-backend-architecture.mjs`
   - `node scripts/dev/check-frontend-architecture.mjs`
   - `cargo test --manifest-path backend/Cargo.toml`
   - `pnpm --dir frontend typecheck`
   - `node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs`
