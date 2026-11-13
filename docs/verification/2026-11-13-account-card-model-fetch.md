# 账号卡里拉取上游模型清单（含导入侧修复）

日期：2026-11-13
范围：`backend/src/lib.rs`、`backend/src/connections.rs`、`backend/src/creds.rs`、`backend/src/gateway/import_sources.rs`、`frontend/src/components/providers/RoutingBlock.tsx`

## 1. 问题

用户从本机导入的第三方中转（华龙，`https://api.hualong-ai.com`）在账号管理里显示
「模型路由 · `claude-code-claude-code/` · 0 个模型」，开启路由也没有任何变化。

查库确认：`models_json = NULL`、`model = NULL`，`slug = claude-code-claude-code`。

## 2. 三个叠加原因

| 现象 | 根因 |
|---|---|
| 0 个模型 | `catalog::list` 对 `kind = api` 只读 `models_json` 缓存，缓存空时退到单填的 `model`；两者都空就是 0 条。而「从本机导入」只写 key / base_url，从不拉清单 |
| 点刷新也没用 | `gateway_refresh_models` 只处理 `zcode` / `grok` / `gemini`，其它平台走 `_ =>` 直接报「该平台没有可实时拉取的模型清单」 |
| slug 重复词 | 导入时显示名拼成「Claude Code · Claude Code 上游」，`slugify` 后得到 `claude-code-claude-code`；用户改名后 slug 按设计保持不变 |

开启路由（`setServe`）本身就只改 `serve_enabled` 标记，不负责拉清单，所以开关不会带来变化。

## 3. 改动

### 3.1 `gateway_refresh_models` 按连接类型分流（`lib.rs`）

原来一律要求网关在运行，且只支持三个订阅平台。现在：

- **API 连接**（`kind = api`）：从库里取 `base_url` / `secret` / `protocol`，走
  `connections::fetch_models`，与「获取模型」同一套逻辑，**不需要网关在跑**；
- **订阅连接**（`kind = auth`）：照旧走各平台适配器，这一类才要求网关运行、才检查
  `status` 是否可用。

凭证仍只从已存连接读，前端传不进来。

### 3.2 导入时顺带拉清单（`import_sources.rs`）

`apply()` 在 `connections::add` 之后，对刚建的连接调一次 `fetch_models`，成功就把
清单写进 `models_json`。失败**不回滚导入**——用户先拿到连接，可以在卡片上再点拉取。

### 3.3 导入的 slug 不再拼来源标签（`connections.rs` + `import_sources.rs`）

`NewConnection` 增加可选字段 `slug_base`（`#[serde(default)]`，既有 25 处构造点无需改动）。
`add()` 分配 slug 时优先用它，缺省才用 `name`。

导入侧传 `slug_base = draft.name`（「Claude Code 上游」），显示名仍保留来源标签。
**只影响新建**，已有连接的 slug 永不改写。

### 3.4 账号卡的模型数是拉取入口（`RoutingBlock.tsx`）

模型路由行那一行从只读文本改成按钮，三种状态：

| 状态 | 显示 |
|---|---|
| 没拉到 | 琥珀色「尚未拉取 · 点此拉取」+ 刷新图标 |
| 拉取中 | 「正在拉取…」+ 转圈，禁用 |
| 有清单 | 「17 个模型」+ 分组数，hover 才露出刷新图标 |

不往账号卡塞第 6 个按钮：卡片已有「详情 / 设为使用 / 刷新余额 / 编辑 / 删除」五个，
且「刷新余额」和「刷新模型」语义不同，放一起容易点错。

### 3.5 开启路由时自动拉一次

`useServeToggle` 在**打开**开关且当前 0 个模型时，自动拉一次清单。已经有清单的不重拉。
拉取失败不回滚开关，弹「模型清单还没拉到」并提示可以重试。

### 3.6 上游错误体带真实原因（`connections.rs`）

原来非 2xx 只报「HTTP 403，请检查 Key 与地址」。现在从响应体取短消息附上：

```
获取模型失败：HTTP 403 Forbidden（API Key 所属分组已停用），请检查 Key 与地址
```

`upstream_message()` 同时认 `{"message":…}`（第三方中转的形态）和
`{"error":{"message":…}}`（OpenAI 官方形态）；空串、无 message、非 JSON、超过 120 字
一律不取，宁可用通用文案也不把整页 HTML 塞进提示。不含任何凭证。

## 4. 测试

| 测试 | 覆盖 |
|---|---|
| `slug_base_keeps_source_label_out_of_the_prefix` | 显示名带来源标签、slug 基名单独给时，前缀是 `claude-code` 而不是 `claude-code-claude-code` |
| `upstream_message_reads_short_reasons_only` | 两种错误体形态都能取到；空串 / 无 message / 非 JSON / 超长一律不取 |

全量：`cargo test --lib` **1382 passed / 0 failed**；前端 `node --test` **435 passed**；
`pnpm --dir frontend typecheck` 通过。

## 5. 真机验证（本机华龙中转）

| 步骤 | 结果 |
|---|---|
| 卡片初始状态 | 琥珀色「尚未拉取 · 点此拉取」可点击 |
| 点击拉取 | 弹出「拉取失败 · 获取模型失败：HTTP 403 Forbidden（API Key 所属分组已停用）」 |
| 关掉再打开开关 | 自动拉取触发，弹出「模型清单还没拉到」+ 原因 + 重试提示 |
| WorkBuddy 卡片（已有 17 个模型） | 显示「17 个模型」，hover 出刷新图标，不自动重拉 |

## 6. 为什么这次仍然拉不到模型

华龙这把 Key 在**上游被停用**了，不是 Jolu 的问题：

```
$ curl -H "Authorization: Bearer sk-***" https://api.hualong-ai.com/v1/models
HTTP 403
{"code":"GROUP_DISABLED","message":"API Key 所属分组已停用"}
```

同一个 Key 的余额查询也报「查询权限不足：该凭证没有查询此数据的权限」，是同一个根因。
现在链路已经通了、错误也能说清，等换一把可用的 Key 就能直接拉到清单。

鼠鼠确认：这个凭证上游已经没了，本就不可能返回模型。这条连接在 Jolu 里就是永久 0 个模型。

## 7. 编辑连接弹窗的布局返工

拉清单接通后暴露出第二个问题：24 个模型铺开会把弹窗撑到接近三屏，下面的「用在哪里」
「默认来源」被挤到看不见，用户会以为弹窗卡住了。四处不合理与对应改法：

| 问题 | 改法 |
|---|---|
| 模型列表把所有模型全铺开，撑高弹窗 | 模型路由改成**可折叠分区**：折叠时一行摘要（`前缀 · N 个模型` + 开关），展开才出细节。开关本身不折叠，它是最常点的东西 |
| 上半截是「label 在上」的纵向表单，模型路由突然变成「label 在左」的横向两列 | `Field` 统一成纵向（label 在上），与「连接名称」「API Key」同一套骨架 |
| 菜单前缀输入框 `w-40` 被 `claude-code-claude-code` 撑满还显示不全 | 输入框 `flex-1` 撑满，保存按钮改 `JoluButton` 放右侧 |
| 24 个模型铺 12 行；`flex-wrap` 让 `-thinking` 变体被甩到另一行 | 按名字排序（同模型变体相邻）+ 只显示 4 个 + 刷新入口合成一个「按账号权限自动获取 · 还有 N 个」 |

试过但没采用：三列固定网格。`claude-haiku-4-5-20251001` 这类长名字被截成
`claude-haiku-4-5-…`，反而认不出来 —— 模型名必须完整显示，列数不能固定。

效果：折叠态弹窗高度约 400px（原 ~700px），展开态约 730px（原超两屏）。

## 8. 存量数据
已经导入且 slug 是 `claude-code-claude-code` 的这条连接**不会被自动改名**（slug 一旦
分配就不随改名变化，改了会破坏已接线的工具）。要换前缀有两条路：

- 卡片「编辑连接」里的「菜单前缀」手动改；
- 删掉重新导入（新导入就会用草稿名字分配前缀）。

## 9. 待确认

- 「从本机导入」的弹窗目前不显示「导入后是否拉到模型」。导入失败时清单为空是静默的，
  用户要点进账号管理才看得到。是否需要导入结果里带上每个条目的模型数，待鼠鼠定。
- `slug_base` 目前只有导入在用。`creds.rs` 的本机发现（Claude / Codex）直接传 `None`，
  因为它们的显示名本来就是干净的名字。
