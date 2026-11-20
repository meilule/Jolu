# Agent 接线剩余 9 家调研报告（面向用户的风险评估）

日期：2026-11-19
参考：magpie `e9dd8e8`（`internal/agent/`）
状态：**调研完成，未实施**。本文回答两件事：magpie 有没有对应实现、Jolu 能不能安全地接。

---

## 0. 先说结论：风险在哪

**Jolu 已接 11 家，探测 20 家，还剩 9 家没接。** 你担心的「开发这边有、用户那边没有就容易出事」，在这 9 家里有 4 家是**明确的高风险**——不是技术难，而是**接了也没法验证**：

| 风险等级 | 家 | 为什么 |
|---|---|---|
| 🔴 **不建议先接** | cline、vscode、zed | 写系统凭据库 / 多文件 + VSCode 扩展状态 / 依赖 VS Code 内部设置结构，magpie 自己都说「扩展只在启动时读一次文件」，写错用户会莫名掉登录 |
| 🟡 需真机样本 | qwen、droid、workbuddy | 结构清晰但本机没装，无法验证真实文件形状；workbuddy 还有「文件监视只在启动时生效」的坑 |
| 🟢 **低风险，建议优先** | **goose** | **独立 JSON 文件、单文件、结构固定**，magpie 有完整实现，最适合先做 |
| ⚪ 无参考 | continue | magpie 没有实现，只能自己逆向 Continue 的 config.yaml |
| 🟡 视需求 | copilot | magpie 只有 `copilotjetbrains.go`（JetBrains 插件），和 Jolu 探测的 Copilot CLI 不是同一个东西 |

**本机实际情况**：只装了 Continue（`~/.continue`）、dsh、VS Code。其余 7 家**无法真机验证**——这是最需要你决策的点。

---

## 1. Goose（🟢 最优先，推荐先做）

**magpie 实现**：`internal/agent/goose.go`（314 行），核心 `gooseProviderJSON` @119

### 配置文件
```
%APPDATA%\Block\goose\config\custom_providers\jolu.json     （Windows）
```
**独立 JSON 文件**，magpie 自己创建，不碰 `config.yaml`。

### 写入形状（`gooseProviderJSON` @119-146）
```json
{
  "name": "jolu",
  "engine": "openai",
  "display_name": "Jolu",
  "description": "Every model Jolu has, through its local gateway",
  "api_key_env": "",
  "base_url": "http://127.0.0.1:12731/v1",
  "headers": { "Authorization": "Bearer jolu-goose.<实例号>" },
  "requires_auth": false,
  "skip_canonical_filtering": true,
  "dynamic_models": false,
  "models": [
    { "name": "<网关模型 id>", "reasoning": false, "context_limit": 200000 }
  ]
}
```

关键细节：
- `requires_auth: false` + `headers.Authorization` 带令牌（**不用环境变量**）
- `dynamic_models: false`：不让 goose 去调 `/v1/models`（否则拿到全量模型但没窗口信息）
- `skip_canonical_filtering: true`：保留 magpie 的顺序、允许 goose 注册表不认识的模型
- `reasoning`：magpie 用正则判模型名（`o\d+` / `gpt-5|6`）决定是否显示 thinking 档；**Jolu 可以直接写 `false`**（目录无档位数据）
- `context_limit` 有值才写
- **models 为空时不写 `dynamic_models`**（goose 拒绝静态列表为空的 provider）

### 用户手改保留（`gooseProviderKept` @155-180）
goose 自己的编辑器会把不显示的键带进文件，用户可能设过 `timeout_seconds` / `toolshim` / `supports_streaming` / `preserves_thinking` / 自定义 header。magpie 的逻辑：
- `display_name` 用户改过就用用户的
- `headers` 合并（用户的自定义头 + magpie 的 Authorization）
- 其余用户键原样保留，只有 magpie 管的字段（`name`/`engine`/`api_key_env`/`base_url`/`requires_auth`/`dynamic_models`/`models`/`headers`）覆盖

### 还原
删掉整个 `custom_providers/jolu.json` 文件（因为是我们创建的独立文件，不涉及用户数据）。

### 状态判定
读该文件：`base_url` 指向本机网关 + `headers.Authorization` 是我们的令牌。

### Jolu 实现难度：**低**
- 单文件、结构固定、无数组插项（models 数组是我们整段写的）
- 现有 `custom_agents.rs` 的 JSON 往返能力即可覆盖
- **但要注意**：需要「保留用户手改字段」的合并逻辑（magpie `mergeKept`），这部分要比 qwen 简单（因为文件是独立命名的，用户一般不会手改）

**结论：建议第一个做，风险最低。**

---

## 2. Qwen Code（🟡 结构清晰但无本机样本）

**magpie 实现**：`internal/agent/qwen.go`（440 行），`qwenIn` @155

### 配置文件
```
~/.qwen/settings.json        （$QWEN_HOME 覆盖）
```

### 写入形状
追加到 `modelProviders.openai` 数组（`qwenEntriesAt` @62）：
```json
{
  "baseUrl": "http://127.0.0.1:12731/v1",
  "envKey": "JOLU_QWEN_API_KEY",
  "generationConfig": { "contextWindowSize": 200000 },
  "id": "<网关模型 id>",
  "name": "<显示名>"
}
```
另写三处：
- `env.JOLU_QWEN_API_KEY = <令牌>`（密钥在这里，不写进数组项）
- `model.name = <选中的模型 id>`
- `model.baseUrl = <网关地址>`（同名 id 可能有多个，靠 baseUrl 区分）
- `security.auth.selectedType = "openai"`（跳过启动时的 /auth 提示）

**用户的条目保持在前**（magpie 追加到末尾，用户自己的 id 不会错位）。

### 快照与还原（`setEnvKey` / `setAuth` @200-260）
- `model.name` / `model.baseUrl` / `security.auth.selectedType` / `env.JOLU_QWEN_API_KEY` 四项各自快照
- `security.auth.selectedType` 有 `keyAuthSeen` 标记：防止第二次接线把 magpie 自己写的 `"openai"` 当成用户原值
- 空父键清理（`qwenPrune` @127）：`env: {}` / `modelProviders: {}` 是 magpie 清空后留下的，删掉

### Jolu 实现难度：**中**
- **需要数组追加**（`modelProviders.openai` 是数组，Jolu 现有引擎只支持点路径标量键）
- 需要「按 envKey 识别自己的条目」的判定逻辑
- 四个键的快照 + 空父键清理
- **无本机样本**，无法验证 Qwen 真实写出的文件形状

---

## 3. Cline（🔴 高风险，暂不建议）

**magpie 实现**：`internal/agent/cline.go`（456 行），`clineIn` @61

### 为什么高风险：三套状态并存

| 载体 | 文件 | 说明 |
|---|---|---|
| CLI (3.x) | `$CLINE_DIR/data/settings/providers.json` + `models.json` | 两个文件，provider 在 `providers["openai-compatible"].settings`，模型列表在 `models.json` |
| VS Code 扩展 (4.x) | 同上 **+** `globalState.json` + `secrets.json` | 扩展**优先读自己的 state**（`actModeApiProvider` / `actModeOpenAiModelId` / `openAiBaseUrl`），只有没有时才读 providers.json；密钥在 `secrets.json` |
| 桌面版 (0.0.x) | 上述文件**都不读** | provider / model / effort 存在 web view 的 localStorage，magpie 只能把模型放进它的 picker，「运行哪个」由用户在 UI 里选 |

magpie 的做法：接管 Cline **内置**的 `openai-compatible` provider（用户自建的 provider 在会话启动时会被拒绝，cline#14180），再把模型写进 `models.json` 替换内置列表。

### 风险
- 要写 4 个文件（providers.json / models.json / globalState.json / secrets.json）
- `globalState.json` 是 VS Code 扩展的内部状态，**写错可能导致用户 Cline 配置错乱**
- 桌面版**无论如何都接不了**（模型选择在 localStorage）
- 扩展「只在启动时读一次文件」，写完后用户必须重启
- **本机没装**，无任何验证手段

**结论：magpie 有实现，但风险与收益不匹配。建议放到最后，或永久标记「暂不支持」。**

---

## 4. Droid（🟡 结构清晰但无样本）

**magpie 实现**：`internal/agent/droid.go`（297 行），`droidIn` @134

### 配置文件
```
~/.factory/settings.json      （$FACTORY_HOME_OVERRIDE/.factory 覆盖）
```
（旧的 `~/.factory/config.json` 的 `custom_models` 仍被读取，magpie 不去动它）

### 写入形状
追加到 `customModels` 数组：
```json
{
  "model": "<网关模型 id>",
  "id": "custom:jolu/<网关模型 id>",
  "displayName": "<显示名>",
  "baseUrl": "http://127.0.0.1:12731/v1",
  "apiKey": "<令牌>",
  "provider": "generic-chat-completion-api",
  "maxContextLimit": 200000,
  "maxOutputTokens": 16384,
  "noImageSupport": false
}
```
另写 `sessionDefaultSettings.model = "custom:jolu/<id>"`。

`provider` 字段决定协议：`anthropic`（Messages）/ `openai`（Responses）/ `generic-chat-completion-api`（Chat）。Jolu 目录无 API 信息，统一写 `generic-chat-completion-api`。

### 风险
- 数组追加（同 qwen）
- 用户条目保持在前，默认模型快照
- **本机没装**，无样本

---

## 5. Zed（🔴 高风险：系统凭据库）

**magpie 实现**：`internal/agent/zed.go`（247 行）+ `zed_credential_windows.go`（47 行）

### 配置文件
```
%APPDATA%\Zed\settings.json     （Windows）
```

### 为什么高风险：密钥在 Windows 凭据管理器

Zed **不读配置文件里的 api_key**，只从系统凭据库取。magpie 用 Win32 `CredWriteW` 写入：
```go
target := "zed:url=" + url
type   := CRED_TYPE_GENERIC (1)
user   := "Bearer"
blob   := []byte(apiKey)          // UTF-8，不是 UTF-16
persist:= CRED_PERSIST_LOCAL_MACHINE (2)
```

### 写入形状（settings.json）
```json
{
  "language_models": {
    "openai_compatible": {
      "jolu": {
        "api_url": "http://127.0.0.1:12731/v1",
        "available_models": [ { "name": "<id>", "max_tokens": ..., "max_output_tokens": ... } ]
      }
    }
  },
  "agent": { "default_model": { "provider": "jolu", "model": "<id>" } }
}
```

### 风险
- **要调 Win32 API 写凭据库**（Rust 需要 `windows` crate + `CredWriteW`）
- 凭据写入失败用户会看到「鉴权失败」但不知道为什么
- **删除时也要清凭据**，否则残留
- **本机没装**，无法验证凭据目标名格式是否正确
- 写错可能污染 Windows 凭据管理器（虽然 target 名带 `zed:url=` 前缀，相对安全）

**结论：magpie 有实现，但需要引入 unsafe Win32 调用。技术上可做（dsh 也没多难），但验证成本最高，建议放在最后。**

---

## 6. VS Code Chat / Copilot（🔴 高风险）

**magpie 实现**：`internal/agent/vscode.go`（608 行，最大）

### 配置文件
```
%APPDATA%\Code\User\chatLanguageModels.json      （+ 每个 profile 的 User 目录，settings.json 旁）
%APPDATA%\Code\User\settings.json                （chat.defaultModel）
```

### 写入形状
`chatLanguageModels.json` 是 **JSONC 数组**，magpie 加一个 group：
```json
{
  "vendor": "customendpoint",
  "name": "jolu",
  "models": [
    { "id": "<id>", "name": "<显示名>", "url": ".../v1/chat/completions",
      "requestHeaders": { "Authorization": "Bearer <令牌>" },
      "toolCalling": true, "maxInputTokens": ..., "maxOutputTokens": ... }
  ]
}
```
另写 `settings.json` 的 `chat.defaultModel`。

### 为什么高风险
- **apiKey 不能写在文件里**：VS Code 只从 secret storage 读，所以 magpie 用 `requestHeaders.Authorization` 绕过
- **VS Code 与 VSCodium 的扩展不同**：VSCodium 装的是 Marketplace 版 Copilot Chat（0.48.1），用的是 `customoai` provider 且**会丢弃 Authorization 头**，需要改用 `x-api-key`
- 需要判断「装的哪个扩展」来决定写哪种 group（`vscodeChatOf`）
- **JSONC 格式**（带注释的 JSON），需要 JSONC 解析
- **多 profile**：每个 VS Code profile 都有独立的 User 目录
- **本机有 VS Code 但没装 Copilot Chat**，无法验证

**结论：magpie 做了 608 行，是这 9 家里最复杂的。风险高、收益低（VS Code 用户大多用 Copilot 官方模型）。**

---

## 7. WorkBuddy（🟡 需样本 + 有坑）

**magpie 实现**：`internal/agent/workbuddy.go`（405 行）

### 配置文件
```
~/.workbuddy/models.json              （国内版，$WORKBUDDY_CONFIG_DIR 覆盖）
~/.workbuddy-ai/models.json           （国际版 WorkBuddy AI）
```

### 写入形状
数组，每项：
```json
{
  "id": "<网关模型 id>",
  "name": "<显示名>",
  "vendor": "jolu",
  "apiKey": "<令牌>",
  "url": "http://127.0.0.1:12731/v1/chat/completions",
  "maxInputTokens": 200000,
  "maxOutputTokens": 16384,
  "contextWindow": { "supportedLengths": [200000], "defaultLength": 200000 },
  "supportsToolCall": true,
  "supportsImages": false,
  "supportsReasoning": false
}
```

### 坑（magpie 注释明写）
> **5.6 的 app.asar 只在启动时监视文件**，且监视的是文件本身，所以**重写可能不被察觉，直到重启**。

所以接完必须提示用户重启 WorkBuddy。

另：WorkBuddy 的模型选择在 UI 里，不在文件中——magpie 只负责「把模型放进 picker」。

**本机没装。**

---

## 8. Continue（⚪ magpie 无实现）

magpie **没有** Continue 的实现（`continue.go` 不存在）。

Continue 的配置是 `~/.continue/config.yaml`（YAML）+ `config.json`（旧版）。要接需要自己逆向：
- `models[]` 数组，每项 `{provider, model, apiBase, apiKey, title}`
- 需要 YAML 写入（Jolu 现有引擎是点路径字符串键，能写但 YAML 格式需自建行级拼接，如 dsh）

**结论：无参考，需要自研；本机有 Continue 可验证，但 magpie 经验无法借用。**

---

## 9. Copilot（🟡 目标不一致）

magpie 只有 `copilotjetbrains.go`（252 行）——写的是 **JetBrains 插件**的配置。

Jolu 探测的 `copilot` 是 **Copilot CLI**（`copilot` 命令，`%LOCALAPPDATA%\Microsoft\VS Code\bin\copilot-shim\copilot.exe`）——**两个不同的东西**。

**结论：magpie 的 copilot 实现不能直接参考。Copilot CLI 要自己逆向（本机有，可验证）。**

---

## 10. 实施建议（按风险排序）

### 第一批（建议做）
1. **goose** — 单文件、结构固定、magpie 参考完整。**但本机没装**，需要你确认是否接受「无真机验证」上线，或等装了再验。

### 第二批（需要样本）
2. **qwen** — 结构清晰，需要数组追加能力
3. **droid** — 同上
4. **workbuddy** — 同上 + 重启提示

### 第三批（建议暂缓）
5. **continue** — 无 magpie 参考，需自研 YAML
6. **copilot** — 目标不一致，需自研

### 不建议做
7. **cline** — 4 文件 + VS Code 扩展内部状态 + 桌面版根本接不了
8. **vscode** — 608 行、JSONC、多 profile、两套扩展差异
9. **zed** — 系统凭据库 + Win32 unsafe

---

## 11. 关于「开发有、用户没有」的风险控制

你的担心是对的，具体建议：

1. **先补「无样本」的探测验证**：本机只装了 3/20 家，其余 17 家的探测逻辑本身就没在真机上验证过。建议先做一轮「让用户提供配置文件样本」的机制，或在你同事/其他机器上跑一遍探测。
2. **接线必须有 preview**：Jolu 已有 `gateway_wiring_preview`（写入预览），9 家里每一家接之前都要能在 UI 上看到「将要写什么」，这样即使结构猜错，用户也能在点确认前发现。
3. **失败要能一键还原**：magpie 的 stash 机制（记原值）必须在每一家都实现，尤其是**多文件**的（cline / vscode / qwen 的 models.json）。
4. **标注验证状态**：接线的 UI 上如实标「未在真机验证」（Jolu 已有 `verified` 机制，如 `CUSTOM_AGENT_PRESETS` 里的字段），不要冒充已验证。
5. **goose 先做但先不发布**：代码可以写，但标「待真机验证」，等你有了 goose 样本再放开。

---

## 12. 一句话回答你的问题

**magpie 有 7 家的实现**（goose / qwen / cline / droid / zed / vscode / workbuddy），其中：
- **只有 goose 适合现在做**（单文件、结构固定、风险低）
- **qwen / droid / workbuddy 可以做但需要数组能力 + 真实样本**
- **cline / vscode / zed 建议不做**（高风险、低收益、无法验证）
- **continue / copilot 没有参考**，要自研

**最大的风险不是技术，是「没装就接」**：9 家里 7 家本机没有，接了也没法验证。建议先解决「样本获取」问题，再谈接线。
