# Qoder 模型清单：拉取入口、离网关拉取与官方显示名

日期：2026-11-14
范围：`backend/src/lib.rs`、`backend/src/proxy.rs`、`backend/src/gateway/catalog.rs`、`backend/src/gateway/adapter/qoder.rs`、`frontend/src/components/providers/RoutingBlock.tsx`

## 1. 问题链

1. 账号卡点「拉取模型清单」报「该平台没有可实时拉取的模型清单（qoder）」——刷新命令只接了
   zcode / grok / gemini；
2. 接上后报「网关未在运行」——订阅适配器的签名要 `ProxyDeps`；
3. 拉到的模型名是 `gmodel / kmodel_latest` 这类内部 id，看不出是什么模型。

## 2. magpie / 官方对照

- magpie 已把 Qoder 内置订阅退役为插件（`@magpie-community/opencode-qoder-auth`），
  凭证形态移植自 CLIProxyAPI（`ufec/CLIProxyAPI`，MIT）；凭证 blob 里直接存
  `Models json.RawMessage`——模型清单随凭证保存；
- 官方端点 `GET /algo/api/v2/model/list?Encode=1` + cosy 签名头，**Jolu 适配器同源实现，
  端点与签名本身没有错**；
- 官方每条模型都带 `display_name`（Fast / GLM-5.3 / Kimi-K3.1-Pro / DeepSeek-V4-Pro…）、
  `key`（chat 接口认的内部 id）、`context_config`、`thinking_config`、`price_factor` 等。

## 3. 改动

### 3.1 刷新命令接入 qoder / trae / workbuddy

`gateway_refresh_models` 的订阅分支按 `load_pool` 同款拼 `PoolEntry`，分发给各平台适配器。

### 3.2 `proxy::detached_deps`：离网关拉清单

调查确认六个订阅适配器的模型路径只碰 `db` 与 HTTP 客户端（`source_is_current` 快照比对
直接打真实连接，不查池索引），网关没在监听时用惰性 deps 兜住，账号卡随时能拉。
转发路径不受影响——那要真池索引与会话亲和表，仍只能用 `running_deps`（其文档警告针对转发）。

### 3.3 缓存形状冲突（重要）

适配器自管富缓存（`credential_hash` 守卫 + 带 display_name 的条目），而刷新命令原来会
再用**纯 key 数组**覆盖 `models_json`——qoder / trae / workbuddy 的富缓存被砸掉，
账号卡退回内置清单、display_name 丢失。现在这三个平台**跳过命令的落库**（适配器在
fetch 时已自写），只有 zcode / grok / gemini 与 API 连接保留命令落库。

### 3.4 官方 display_name 上屏

- `qoder::display_names(conn, id)`：缓存条目按 key 配对 `display_name`；
- `catalog::list` 对 qoder 填 `CatalogModel.label`（字段两端早已预留，此前从未填过）；
- 前端 `RoutingIndex.labelsBySlug`，编辑弹窗的模型清单主位显示官方名、右侧小字保留 key，
  悬停显示调用 id `前缀/key`；排序也按官方名。

## 4. 真机数据（Qoder CN，12 条）

key → display_name：fast→Fast、qmodel_latest→Qwen4-Max-Preview、qmodel_38max→Qwen3.8-Max、
qfmodel→Qwen3.8-Flash、dmodel→DeepSeek-V4-Pro、dfmodel→DeepSeek-Flash、gmodel→GLM-5.3、
gfmodel→GLM-5.3-Flash、kmodel_latest→Kimi-K3.1-Pro、kmodel→Kimi-K2.8-Preview、
jmodel→Step-5-Preview、mmodel→MiniMax-M3.1-Flash。

## 5. 测试与验证

- 全量 `cargo test --lib` **1384 passed / 0 failed**；`pnpm --dir frontend typecheck` 通过；
- 只读 smoke（`local_models_without_gateway_readonly_smoke`，`#[ignore]`）：网关未运行时
  经 `detached_deps` 实时拉取成功（12 条），两次执行结果一致；
- 真机 UI：编辑弹窗清单主位显示官方名（Step-5-Preview / Qwen4-Max-Preview / …），右侧
  保留调用 key，账号卡显示「12 个模型」；富缓存对象（12 entries + hash）在刷新后保持完好。

## 6. 备注

- 上游的场景列表会变（曾返回过 4 条带「Auto Efficient」的 app 场景），我们如实存
  当时拿到的那份；下次拉取自动更新。
- `context_config` / `thinking_config` / `price_factor` 已在缓存条目里，档位与计费展示
  要用时不必再改协议层。
