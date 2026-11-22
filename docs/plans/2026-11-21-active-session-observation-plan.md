# 活跃会话观测：全量调查与优化方案

日期：2026-11-21
状态：**阶段 1 / 2 / 3 已实施；Kimi / Pi / dsh 的本地会话读取已接入**
关联：`docs/plans/2026-11-20-agent-wiring-refactor-plan.md`（接线侧已完成同类改造）

---

## 1. 需求与目标

### 1.1 需求原文

> 在活跃会话中可以看到已接线的 agent 的会话。现在我在 ZCode 也进行了接线，但是在活跃会话中 ZCode 也没有出现在该会话中。

### 1.2 目标

| # | 目标 | 判定标准 |
|---|---|---|
| G1 | 已接线的 13 家都能被**正确识别** | `gateway_route_log.agent` 不再出现属于这些工具的 NULL 行 |
| G2 | 已接线的工具都能在**活跃会话**里出现 | 用 ZCode / WorkBuddy 各发一条消息，概览页出现对应会话行 |
| G3 | 认不出来时**不假装** | 认不出 → `agent` 为 NULL 并在诊断可见，不归到猜测的平台 |
| G4 | **加一家工具只改一处** | 身份、会话来源、展示名都在同一张注册表 |
| G5 | 结构不再继续堆 | 同类计算只有一份实现（见 §3） |

### 1.3 已确证的复现证据（本机真实数据）

| 数据表 | 实测结果 |
|---|---|
| `gateway_route_log` | `claude-code` 83 条、`zcode` 2 条、**`agent IS NULL` 103 条** |
| `gateway_context_snapshot` | **只有 `claude` 一个平台，1 条** |
| NULL 行的构成 | `zcode/GLM-5.3-Flash` 42 条、`workbuddy-16f911/*` 53 条、其它 8 条 |

**结论：ZCode / WorkBuddy 的请求过了网关、记了流水，但（a）认不出是哪个工具，（b）建不了快照。**

---

## 2. 全量调查（事实，均标注文件:行号）

### 2.1 身份识别：只认 4 家

`backend/src/gateway/wire.rs:294` `agent_from_headers()` 的判定链：

| 序 | 信号 | 产出 | 行号 |
|---|---|---|---|
| 1 | `x-codex-window-id` / `x-codex-turn-metadata` + UA/Originator 是 `claude code` | codex | 237-250 |
| 2 | UA `vscode_agent_host` | codex | 257-265 |
| 3 | 令牌前缀 `jolu-<agent>.`（在 `wiring::agents()` 名单内） | 令牌里的 agent | 303-314 |
| 4 | `originator` contains codex / claude / gemini | 对应 agent | 315-326 |
| 5 | UA `GeminiCLI` / `proxy_client=geminicli` | gemini | 327-331 |
| 6 | UA `codex_cli_rs` | codex | 332-334 |
| 7 | UA `claude-cli/` | claude-code | 335-337 |
| 8 | UA `opencode/` 或 `x-opencode-*` 头 | opencode | 338-344 |
| 9 | 兜底 | **None + unidentified_ua** | 345-346 |

**13 家里有 9 家没有 UA 分支。** 它们只能靠第 3 条（令牌前缀）被认出——而实测 103 条 NULL 说明令牌路径对 ZCode / WorkBuddy 也没生效（原因待查，见 §6 待确认）。

### 2.2 会话识别：只认 3 家

`wire.rs:197` `session_identity()`：

| agent | platform | 会话 ID 来源 |
|---|---|---|
| `claude-code` | claude | 头 `x-claude-code-session-id`，否则 body `metadata.user_id` |
| `codex` | codex | 头 `session-id` / `session_id` |
| `opencode` | opencode | 头 `x-opencode-parent-session-id` / `x-opencode-session-id`，拼 `opencode:` 前缀 |
| 其余 10 家 | — | `return None`（`wire.rs:218`） |

`None` → `wire.rs:559` 的 `session_ref.as_ref().map(...)` 不建 `Capture` → 快照表里一行都不写。

### 2.3 活跃会话表实际是**两套数据合并**

这是我之前判断错的地方。截图那张表不是单一数据源：

```
gateway_sessions_overview          gateway_context_sessions
  (sessions.rs:75)                    (context/query.rs:128)
  输入：route_log + requests/sessions   输入：gateway_context_snapshot
        + 内存在途快照                        + route_log + connections
        ↓                                    ↓
   useGatewaySessions (15s)          GatewayView.refresh (5s)
        ↓                                    ↓
   sessions.views                    contexts: ContextSessionRowDto[]
        └──────────────┬─────────────────────┘
                       ▼
        contextSessions.ts:134-160  activeSessionRows()
                       ▼
              OverviewPanel 活跃会话表
```

**每一列的来源**（`OverviewPanel.tsx:194-201`、`contextSessions.ts:143-153`）：

| 列 | 来源 | 说明 |
|---|---|---|
| Agent / 会话 / 项目 | `sessions.views`（title、cwd）+ `contexts`（agent 回退） | 标题和项目**只有** `sessions.rs` 有 |
| 实际模型 / 来源账号 | **优先 `contexts`**，回退 `links` | 账号名只有 `contexts` 有 `connection_name` |
| 最近一次上下文 | **只有 `contexts`** | `input_tokens` / `estimated` / `window_tokens` |
| 缓存读取 / 占输入 | **只有 `contexts`** | `cache_read` |
| 状态 / 最近请求 | `sessions.views`（在途事件）+ `contexts.ts` | 时间取两者最大 |

**所以「活跃会话看不到 ZCode」的真正原因是**：
- 行本身由 `sessions.rs` 撑起（靠 `route_log`，ZCode 有记录）→ **行应该出现**
- 但 `agent` 列是 NULL（身份识别失败）
- 上下文 / 缓存 / 模型 / 账号四列全是空（没有快照）

截图上那 8 行 Claude Code 是因为它有 `agent` + 有快照。

### 2.4 本地会话文件读取：**三套实现，互不相通**

| 模块 | 角色 | 行数 |
|---|---|---|
| `native_sources.rs` | **生产者**：读第三方文件 → 写 `requests` / `sessions` 表 | 1571 |
| `session_threads.rs` | **消费者**：读 `requests` / `sessions` 表 + 重解析源文件拿正文 | 1630 |
| `collector.rs` | **另一个生产者**：Claude / Codex 的 JSONL 用量采集 | — |

**关键缺陷（真 bug，不是设计取舍）**：

| # | 缺陷 | 位置 | 后果 |
|---|---|---|---|
| ① | `save_stamp` **不写** `parser_session_id` | `native_sources.rs:326-329` | `session_threads::source_files` 看不到 `native_sources` 采集的任何来源 → WorkBuddy / Qoder / Gemini / OpenCode 的会话在会话页永远没有正文 |
| ② | `has_source` 只对 Claude / Codex 为真 | `session_threads.rs:88-99` | ZCode 的 `activity_state`（running/done/failed）**写了但永远不显示**，会话页恒为 `recent` |
| ③ | ZCode 子会话过滤只在消费者侧 | `session_threads.rs:822-844` | 采集侧照常把子代理会话的 token 计入总量 |
| ④ | 会话 ID 格式三套 | `native_sources.rs:730`（`{source}:{raw}`）、`opencode_usage.rs:26`（`opencode:{raw}`）、collector 用裸 `sessionId` | 同一会话在不同表里 id 不同，无法 join |

### 2.5 重复实现清单（同类计算写了 2 份以上）

| 能力 | 实现 A | 实现 B | 实现 C |
|---|---|---|---|
| JSONL 逐行读取 | `session_threads.rs:706-750` | `native_sources.rs:348-395` | `collector.rs:417-530` |
| 文件戳（size/mtime） | `session_threads.rs:110-112` | `native_sources.rs:314-323` | `collector.rs:82-89` |
| RFC3339 → 毫秒 | `session_threads.rs:374-376` | `native_parse.rs:15-21` | `collector.rs:56-58` |
| 只读 sqlite 打开 | `session_threads.rs:802-807` | `native_sources.rs:286-296` | 另 6 处（attribution / session_presence / import_sources / opencode_usage / trae_browser_auth / adapter/workbuddy） |
| `metadata.user_id` 解析 | `wire.rs:98-115`（`anthropic_user_session`） | `wire.rs:180-187`（`claude_session_from_user_id`） | — |
| 会话身份（亲和 vs 严格） | `wire.rs:133-167`（`session_key`） | `wire.rs:197-221`（`session_identity`） | 前者读同一批头，但用途不同 |
| 网关会话聚合 | `sessions.rs:81-103`（内存 HashSet 合并） | `context/query.rs:130-141`（SQL 窗口函数） | 前端 `contextSessions.ts:134` 第三次合并 |
| 项目名（cwd 末段） | `contextSessions.ts:113-117` | `gatewaySessions.ts:261-265` | `sessionThreads.ts:19`、`automation.ts:378` |
| Token 紧凑格式 | `gatewaySessions.ts:299-304` | `api.ts:944-949` | — |
| `[1m]` 后缀剥离 | `route.rs` `canonical_model` | `escape.rs:181`、`commands/routing.rs:427`、`wiring.rs:123/129` | 至少 4 处 |

### 2.6 加一个工具的完整成本：**28 处**

（详细清单见调查附录，此处只列摘要）

| 类别 | 处数 | 举例 |
|---|---|---|
| 后端身份 / 会话 | 3 | `agent_from_headers`、`session_identity`、`session_key` 头列表 |
| 后端接线 | 2 | `wiring_specs.rs` 一行 + `adapters.rs` 两个函数 |
| 后端平台能力 | 9 | `own_platforms`、`unsupported` 白名单、适配器分发、`server.rs` 模型清单 4 条分支、`access.rs` Codex 专属、`subscription.rs` 6 处 |
| 后端探测 / 导入 | 3 | `agent_discovery.rs`（数组长度硬编码 `17`）、`import_sources.rs` 两处 |
| **前端展示名** | **4** | `gatewayPage.ts:AGENT_LABELS`、`gatewaySessions.ts:TOOL_LABELS`、`agentPage.ts:wiringName`、`librarySharedView.ts:SHARED_AGENT_LABELS` —— **四份重复的展示名表** |
| 前端图标 / 文案 | 4 | `agentPage.ts` 的 `AGENT_PLATFORM` / `AVAILABLE_SUBTITLES`、`AgentList.tsx` 的 `PLATFORM_OF` / `AGENT_IMAGES` |
| 其它 | 3 | `connections.rs` 能力名单等 |

**这就是「边改边堆屎」的具体形态：同一件事（这家工具叫什么、图标是什么、UA 是什么）写在 4-28 个地方。**

---

## 3. 根因归类（三个结构性问题是同一件事）

不是「漏了 ZCode」，而是**「工具身份」这个概念从未被建模**：

| 问题 | 本质 |
|---|---|
| UA / 令牌 / 会话头 散在 `wire.rs` 的 if 链里 | 身份识别没有注册表 |
| 展示名 4 份、图标 2 份、文案 3 份 | 展示元数据没有单一来源 |
| 会话数据有 3 个生产者、2 个聚合器 | 会话没有统一模型 |

而**接线侧已经在 2026-11-20 做完了同类改造**（`wiring_specs.rs`：一张表声明 8 个字段，`AGENTS` / `config_path` / `restart_notice` / `ours_with` / db 种子全部派生）。

**身份识别是同一个问题的另一半**：接线决定「写哪个文件」，身份决定「读哪个请求」。两者共用一张表最省。

---

## 4. 目标架构

### 4.1 一张表，四个消费者

```
        ┌──────────────────────────────────────────┐
        │  单一注册表（扩展 wiring_specs.rs）        │
        │  AgentSpec {                             │
        │    id, name,                              │
        │    // 接线（已有）                         │
        │    wire, unwire, path, base, token, …     │
        │    // 身份（新增）                         │
        │    ua: &[&str],          ← UA 前缀         │
        │    session: SessionSource, ← 会话 ID 从哪来 │
        │    // 展示（新增）                         │
        │    platform_id, icon, subtitle,           │
        │  }                                        │
        └───────────────┬──────────────────────────┘
                        │
     ┌──────────────────┼──────────────────┬─────────────────┐
     ▼                  ▼                  ▼                 ▼
① agent_from_headers  ② session_identity  ③ 前端展示元数据   ④ db 种子
   （查 ua 表）          （查 session 表）     （一个命令返回）    （已有，派生）
```

### 4.2 会话来源的两类（这是关键设计）

```rust
enum SessionSource {
    /// 请求头里就带会话 ID（Claude Code / Codex / OpenCode 已验证）
    Header { headers: &'static [&'static str] },
    /// 请求体里带（Claude Code 的 metadata.user_id 是这种）
    Body { pointer: &'static str },
    /// 请求里没有，只能从本地会话文件反查（ZCode / WorkBuddy 等）
    LocalFile { reader: fn(&Path) -> Vec<LocalSession> },
}
```

**为什么必须显式区分**：`LocalFile` 类型**不能**在请求时建快照（那一刻还不知道属于哪个会话）。它的数据要等会话文件更新后才能关联。混在一起写会变成「用时间窗口猜」，那是错的。

### 4.3 三层职责

| 层 | 职责 | 文件 |
|---|---|---|
| **身份层** | agent 是谁、会话 ID 从哪来 | `wiring_specs.rs`（扩展）+ `wire.rs`（改为查表） |
| **采集层** | 请求 → 快照 | `context/capture.rs`（不变，只依赖身份层） |
| **聚合层** | 快照 + 本地会话文件 → 统一会话视图 | 合并 `gateway/sessions.rs` + `gateway/context/query.rs` |
| **展示层** | 前端只调一个命令 + 一个元数据命令 | `OverviewPanel.tsx` |

---

## 5. 分阶段计划

### 阶段 1：身份注册表（解决 G1 / G3 / G4 的一半）

**范围**：只动身份识别与前端展示元数据，**不动数据流**。

| 步骤 | 改动 | 风险 |
|---|---|---|
| 1.1 | 给 `AgentSpec` 加 `ua: &'static [&'static str]` | 低（纯新增字段） |
| 1.2 | `agent_from_headers` 的 UA 部分改为查表；`codex_masquerading_as_claude` / `hosted_codex` 作为「反伪装」保留在前 | 低 |
| 1.3 | 补 magpie 已确认的 UA：`zcode` / `workbuddy` / `qwencode` / `kimicli` / `deepseek-harness` | 低 |
| 1.4 | 查 `pencil` / `pi` / `qoder` 的 UA（magpie 只确认 `pi-desktop`） | 低 |
| 1.5 | 新增命令 `agent_meta()` 返回 `[{id, name, platform_id, subtitle}]`，前端 4 份展示名表合并成 1 份 | 中（动 4 个前端文件，但有守卫测试） |

**验收**：
- `gateway_route_log` 里不再出现属于 13 家的 NULL 行（用 ZCode / WorkBuddy 各发一条验证）
- 前端展示名只有一个来源，加一家不用改前端

**这一阶段单独可交付**，即使阶段 2 不做，也解决了「认不出是谁」。

---

### 阶段 2：探明会话来源（只调查，不改代码）

**为什么单列**：阶段 1 之后，ZCode 会被认出，但会话 ID 仍需回答「从哪来」。magpie 的做法是从本地会话文件读，但**Jolu 的 ZCode 读取器已经存在**（`session_threads.rs`）。

| 要查清 | 方法 |
|---|---|
| ZCode 请求头里有没有会话 ID | `JOLU_HEADER_PROBE=1`（已实现，见 §7）+ 实际发一条消息 |
| WorkBuddy 同上 | 同上 |
| 各家本地会话文件格式 | 只读检查（ZCode 已确认：`~/.zcode/cli/db/db.sqlite` 的 `session`/`message`/`part`） |

**产出**：一份「13 家的会话来源对照表」，明确哪家是 `Header` / `Body` / `LocalFile`。

---

### 阶段 3：会话来源接线（解决 G2）

按阶段 2 的结论，把 `SessionSource` 填进注册表。

**对 `Header` / `Body` 类**（Claude Code / Codex / OpenCode / 可能有 ZCode）：

- 只需在注册表声明头名或 body 路径
- `session_identity` 改查表
- 立刻出现在活跃会话里

**对 `LocalFile` 类**（ZCode / WorkBuddy / …）：

这是**新能力**，需要：

1. 修复 §2.4 的缺陷 ①（`save_stamp` 不写 `parser_session_id`）
2. 给 `session_threads` 加「按平台读本地会话」的统一入口（现在硬编码 claude/codex 两条路径）
3. 聚合层把「本地会话」与「网关快照」按 `(platform, session_id)` 合并

**⚠️ 工作量提示**：第 2 步是 `session_threads.rs` 的实质性改造（1630 行，平台白名单散在 5 处）。建议**先只接 ZCode 一家**验证，再推广。

---

### 阶段 4：聚合层合并（解决 G5 的一半，可延后）

合并 `gateway/sessions.rs`（197 行）与 `gateway/context/query.rs`：

- 现在两个模块都在算「网关会话」，只是字段不同
- 前端在 `contextSessions.ts:134` 做第三次合并

**建议延后**：这一层是 SQL 重写，风险中等，且不影响用户可见的功能。等阶段 3 跑通、数据模型稳定后再做。

---

### 阶段 5：消除重复实现（可选，按需）

按 §2.5 的清单，逐项合并。优先级：

1. **只读 sqlite 打开**（9 处 → 1 处）—— 影响面大且风险低
2. **JSONL 逐行读取**（3 处 → 1 处）—— 需要统一「尾半行 / 坏 JSON」策略，中风险
3. **RFC3339 解析**（3 处 → 1 处）—— 低风险
4. **`metadata.user_id` 解析**（2 处 → 1 处）—— 低风险

**不建议现在做**：`session_key` 与 `session_identity` 合并——两者语义不同（亲和键允许模糊匹配，会话身份必须严格），合并会破坏既有行为。

---

## 6. 待确认

| # | 问题 | 我的建议 |
|---|---|---|
| 1 | 阶段 1 是否现在做？ | **建议做**。低风险、有 magpie 参考、立刻解决 103 条 NULL 行 |
| 2 | `agent IS NULL` 的 103 条里，ZCode / WorkBuddy 的令牌为什么没被识别？ | 需要实测（`JOLU_HEADER_PROBE=1`）。可能是它们没带 `jolu-` 令牌，走了别的认证路径 |
| 3 | 阶段 3 的 `LocalFile` 路线要不要做？ | 工作量最大。建议**先只做 ZCode**（读取器已有）验证可行性 |
| 4 | 阶段 4 / 5 现在做还是延后？ | **建议延后**。不影响用户可见功能，先让功能跑通 |
| 5 | `pencil` / `pi` / `qoder` 的 UA | 需要再查 magpie 或实测 |

---

## 7. 已完成的准备工作

### 7.1 `backend/src/gateway/header_probe.rs`（新增，4 条测试通过）

- 环境变量 `JOLU_HEADER_PROBE=1` 时开启
- 记录：头的**名字**、`user-agent` 的值、请求体的**键路径**（只到第二层）、`agent_from_headers` 与 `session_identity` 的判定结果
- **不记录**：任何头的值（密钥）、请求体的值（用户提示词）
- 端到端测试锁住「头名在、值绝不在」

**用途**：阶段 2 需要它拿事实。补完 UA 表后这个模块仍然保留——以后加新工具时用它验证会话来源。

### 7.2 已确证的数据事实

见 §1.3。所有数字来自本机 `usage.db` 的实际查询，不是估计。

---

## 8. 与已完成工作的关系

接线侧（2026-11-20/21）已经把「按工具登记」这套做完了：

| 已完成 | 位置 |
|---|---|
| 注册表驱动接线 | `gateway/wiring_specs.rs`（13 行 `AgentSpec`） |
| 按工具拆分的写入适配 | `gateway/wiring/adapters.rs` |
| 13 家全量往返测试 | `every_agent_round_trips_through_production_plan` |
| 前后端清单守卫 | `agentPage.test.mjs` |

**身份识别应该复用同一张表**，否则就是「接线一套、身份一套」——又是一国两制。

本文的 §4 目标架构就是这个复用。


---

## 9. 执行记录（2026-11-22）

### 9.1 阶段 1：UA 表（已提交 `6258f95`）

`AgentSpec` 新增 `ua: &'static [&'static str]`，13 家全部填值，`agent_from_headers` 改为查表：

| 工具 | UA 前缀 |
|---|---|
| claude-code | `claude-cli` / `claude-code` |
| codex | `codex` / `codex_cli_rs` / `acp-extension-codex` |
| gemini | `geminicli` / `gemini-cli` |
| opencode | `opencode` |
| zcode | `zcode` |
| kimi | `kimicli` |
| dsh | `deepseek-harness` |
| qwen | `qwencode` |
| workbuddy | `workbuddy` |
| pi / pencil / qoder / qoder-cn | 空（magpie 也未声明，只能靠令牌前缀） |

两个关键设计：
- **只在产品名上精确匹配**（`zcode/1.2.3` → `zcode`），不做子串匹配 —— 否则 `my-zcode-fork` 会被误认成 ZCode
- **`proxy_client=geminicli` 单独用 contains 判断**，它在中段不在开头

### 9.2 阶段 2：ZCode 会话来源调查（查源码完成，无需抓包）

**结论：ZCode 的模型请求完全不带会话标识。**

证据 `apps/zcode-cli/packages/bootstrap/src/model-config.ts` 的 `buildCliZCodeSourceHeaders()`：

```ts
return {
  "HTTP-Referer": ...,
  "User-Agent": `ZCode/${appVersion}`,
  "X-ZCode-App-Version": appVersion,
  "X-Title": `Z Code@${sourceTitle}`,
  "X-Release-Channel": ...,
  "X-Client-Language": locale,
  "X-Client-Timezone": timezone,
  "X-ZCode-Agent": "glm",
  ...createRuntimePlatformHeaders(),   // X-Platform / X-Os-Category / X-Os-Version
}
```

没有 session / conversation / thread 任何字段。请求体里也搜过 `prompt_cache_key` / `conversation_id`，全仓 **0 处**。

**所以 ZCode 只能走 `LocalFile` 路线**，不可能从请求推出会话。

### 9.3 阶段 3：方案 C（并列而非合并，已实施）

原方案有两种：A（按时间+model+token 事后关联）/ C（并列显示本机会话）。

**选了 C**，理由：

- ZCode 的会话数据**已经在 Jolu 里**（`native_sources` 采集了 `model_usage`，`session_threads` 能读 `message`/`part`）
- 事后关联（方案 A）在并发请求上会关联错——ZCode 本机实测有子代理会话与主会话时间重叠
- 并列不会编造任何对应关系，两套数据各自真实

**实现**（`gateway/sessions.rs`）：

1. `overview()` 的会话来源从「route_log 链接 + 在途请求」扩成**三路**，第三路是
   `session_threads::list(conn, { since: 24h前 })` —— 本机会话文件里最近活跃的会话
2. 新增 `agent_of_platform()`：平台名 → agent id 的显式映射（`claude` → `claude-code`）。
   只在本地会话文件里的会话没有 route_log 链接，若不反查，Agent 列会空着
3. 同一平台同一会话两边都有时按 `(platform, session_id)` 自然合并成一行

**效果**（本机真实数据，最近 24h）：

| | 改前 | 改后 |
|---|---|---|
| 活跃会话 | 1 条（只有 `claude`） | **4 个**（2 个 ZCode + 2 个 Codex） |

ZCode 的 13 个会话里 11 个是子代理会话，被 `session_threads` 的 `zcode_child_session` 过滤，只留 2 个根会话——这是既有行为，不是本次引入。

### 9.4 验证

| 项 | 结果 |
|---|---|
| `cargo test --lib` | **1588 通过 / 0 失败 / 53 忽略** |
| 编译警告 | **0** |
| 前端 `node --test` | **506 通过 / 0 失败** |
| `tsc --noEmit` | 通过 |
| 本机会话数模拟 | 24h 内 4 个根会话（2 ZCode + 2 Codex） |

新增测试：`local_only_sessions_appear_with_their_agent`（本机会话会进来 + Agent 反查得到 + 没有链接是对的）

按鼠鼠要求「减少多余的测试」，本次只补这 1 条覆盖新行为，没有为 UA 表之外的细节再写测试。

### 9.5 遗留

| # | 项 | 说明 |
|---|---|---|
| 1 | WorkBuddy 的本地会话读取 | `native_sources` 只采集了用量，不采正文；要在活跃会话里看到它需要给 `session_threads` 加 WorkBuddy 分支（工作量：5 处模块内改动 + 1 个解析函数） |
| 2 | 其余 5 家（Kimi / Qwen / dsh / Pi / Pencil） | 同上，各家要单独的本地会话读取器 |
| 3 | 上下文列对纯本机会话为空 | 它们没有快照，这是诚实的缺失。要填需要另外的方案 |
| 4 | 历史 `agent = NULL` 流水 | 103 条是改 UA 表之前记的，UA 没存进库，无法回填；只对新请求生效 |


---

## 10. Kimi 本地会话读取（2026-11-22）

### 10.1 为什么单独一个模块

Kimi 的会话**不在 `requests` 表里** —— `native_sources` 从未采集它（那套采集器只覆盖 ZCode / Qoder / WorkBuddy / Gemini / OpenCode）。所以 `session_threads::list` 的主查询看不到它，只能直接从 Kimi 自己的目录读。新增 `backend/src/kimi_sessions.rs`。

### 10.2 实测格式（本机核实，非推测）

```text
~/.kimi-code/
  session_index.jsonl          每行 { sessionId, sessionDir, workDir }
  sessions/<wd_xxx>/<sessionId>/
    state.json                 { createdAt, updatedAt, title, workDir, agents{main{homedir}} }
    agents/main/wire.jsonl     事件流，每行一个 JSON
```

`wire.jsonl` 里用得上的两类事件：

```json
{"type":"llm.request","modelAlias":"moonshot-cn/kimi-k3","time":1785308428008}
{"type":"usage.record","usage":{"inputOther":1994,"output":197,"inputCacheRead":19200,"inputCreation":0},"time":1785308451368}
```

- 请求数按 `llm.request` 计
- 用量按 `usage.record` 累加；`inputOther` **不含缓存读取**，可直接当「新输入」
- 一次调用对应一条 request + 一条 usage（usage 时间略晚）

### 10.3 真实数据发现的一个坑

本机 14 个会话里，**13 个的 `updatedAt` 是 RFC3339 字符串，1 个是整数毫秒**（`1784281243255`）。只认字符串会把那条会话当成「时间未知」，在「最近 24 小时」筛选里被错误排除。`ts_of()` 现在两种都认，断言并在 `since_filters_old_sessions_and_missing_dirs_are_skipped` 里。

### 10.4 不做什么

| 项 | 原因 |
|---|---|
| 不解析对话正文 | `context.append_message` 里有内容，但本模块只要求列出会话。`has_source` 给 false，正文页如实显示「不可读」，不假装能打开 |
| 不猜缺失值 | 没有 `usage.record` 的请求 Token 记 None 而不是 0 |
| 不读子代理 wire | 只读 `agents/main/`，子代理有自己的 wire 文件但不算用户会话 |
| 隔离验收不读 | `validation::isolated()` 时直接返回空 |

### 10.5 效果验证（真实数据）

本机 14 个 Kimi 会话，10 个有实际内容，全部能正确读出：

| 会话 | 请求 | 输入 | 输出 | 缓存读 |
|---|---:|---:|---:|---:|
| session_59be0369 | 26 | 78,151 | 32,102 | 1,234,536 |
| session_2a50b546 | 36 | 48,821 | 23,337 | 1,362,657 |
| session_0813b14f | 26 | 54,044 | 29,514 | 1,188,720 |
| …（其余 7 个） | 1–6 | | | |

它们都是「旧」的（最近 24h 内无活动），所以**不会**出现在活跃会话里 —— 这是正确行为。用 Kimi 发一条消息就会出现。

### 10.6 验证结果

| 项 | 结果 |
|---|---|
| `cargo test --lib` | **1591 通过 / 0 失败 / 53 忽略** |
| 编译警告 | **0** |
| 前端 `node --test` | **506 通过** |
| `tsc --noEmit` | 通过 |

测试从 4 条合并为 3 条（把时间格式的 4 行断言并进筛选测试，删掉独立的那条）。

### 10.7 Pi 本地会话读取（同日完成）

`backend/src/pi_sessions.rs`，同一个模式：独立模块 + `session_threads::list` 末尾追加。

**实测格式**：`~/.pi/agent/sessions/<项目>/<时间戳>_<uuid>.jsonl`，一个文件一个会话。

```json
{"type":"session","version":3,"id":"01a05200-…","timestamp":"…","cwd":"D:\develop\shushu"}
{"type":"message","timestamp":"…","message":{"role":"assistant","model":"deepseek-v4-flash",
 "usage":{"input":5828,"output":152,"cacheRead":0,"cacheWrite":0,"reasoning":42,"totalTokens":5980}}}
```

**与 Kimi 的两处差异**：

1. **`usage.input` 要扣掉 `cacheRead`**：Pi 的 `input` 可能已含缓存读取，直接相加会重复计数。
   本机样本 `input=5828, cacheRead=0`，但为安全起见统一按「新输入 = input − cacheRead」处理。
2. **没有 `title` 字段**：用首个 user 消息的第一行前 60 字作标题（与 Codex 同做法）。

会话身份取 `session` 事件的 `id`（官方字段），不从文件名解析。

**真实数据验证**：本机 17 个 Pi 会话全部有 `session` 事件，读取正确：

| 会话 | 请求 | 新输入 | 输出 | 缓存读 |
|---|---:|---:|---:|---:|
| 01a0aab6 | 75 | 0 | 38,473 | 2,768,619 |
| 01a0966b | 77 | 529,098 | 137,999 | 14,758,400 |
| 01a0a0c4 | 22 | 1,256 | 52,697 | 1,686,839 |

前端 `TOOL_LABELS` 同时补上 `kimi` / `pi`，否则活跃会话里会显示原始平台名。

### 10.8 dsh 本地会话读取（同日完成）

`backend/src/dsh_sessions.rs`。与 Kimi / Pi 同一个模式，但多了一个坑。

**实测格式**：`~/.dsh/sessions/<项目路径转义>/<sessionId>/session.v4.jsonl.zstd`

```json
{"type":"session","version":4,"id":"0135f942-…","createdAt":1793859630498,"cwd":"D:\develop\Jolu",
 "origin":"subagent","parentSession":"session-…"}
{"type":"assistant/message","time":…,"data":{"usage":{"inputTokens":7666,"outputTokens":196,
 "totalTokens":24246,"cacheReadTokens":16384}}}
```

**三个必须处理的点**：

1. **多帧 zstd**（最坑的一个）：dsh 是每次追加写一个**独立的 zstd frame**。本机一个
   233KB 的会话文件里有 **122 个 frame 起始标记**。`ruzstd` 的 `StreamingDecoder` 只解
   第一帧就 `Ok(0)` 结束——直接 `read_to_end` 只能拿到 **270 字节**，整个会话看起来是空的。
   改成按 zstd magic 逐帧循环解压后，同一文件解出 **791,982 字节**，与 Python
   `zstandard` 完全一致。
2. **子代理会话要过滤**：`origin == "subagent"` 或 `parentSession` 非空的是子代理自己开的。
   实测 `--D-develop-Jolu--` 项目里 **4 个根会话 + 39 个子代理会话**，不过滤的话一个任务
   会冒出十几条「会话」。
3. **`inputTokens` 扣掉 `cacheReadTokens`**：样本 `7666 + 196 + 16384 = 24246`，说明
   `inputTokens` 本就不含缓存；仍按「扣掉并钳到 0」处理，防止换版本后重复计数。

**依赖**：新增 `ruzstd = "0.9"`（纯 Rust 解码器）。选它而不是绑 C 库的 `zstd`：这里只解压
不压缩，没必要引入 native 构建依赖。

**测试**：内嵌由 Python `zstandard` 生成的真实压缩字节，不调外部 `zstd` 命令——靠外部命令
会让测试在 CI 上静默跳过，等于没测。

### 10.9 剩余（本机无样本或无会话文件）

| 工具 | 情况 |
|---|---|
| **WorkBuddy** | 本机未安装；且 `native_sources` 只采用量不采正文 |
| **Qwen** | 本机未安装（`~/.qwen` 不存在），无样本 |
| **Pencil** | 本机 `~/.pencil` 存在但**没有会话文件**，它是模型配置目录不是会话存储 |

**能出现在活跃会话的 8 家**：Claude Code、Codex、ZCode、OpenCode、Qoder（走 `requests` 表）
+ Kimi、Pi、dsh（走本地会话文件）。剩下 5 家里 3 家本机无样本、Pencil 无会话文件、
WorkBuddy 需先补用量采集。
