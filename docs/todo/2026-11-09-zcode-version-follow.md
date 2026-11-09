# ZCode 升级后的跟进检查（C6）

Jolu 的 Start Plan 通道与 ZCode 账号额度查询都冒充 ZCode 客户端发请求。ZCode 升级后，供应商可能按新版本的请求样式做风控（405 / 3012），余额端点也会按 `app_version` 判定礼包能力，所以每次 ZCode 发新版都要核对一遍。

## 每次 ZCode 升级后

- [ ] 更新 `backend/src/zcode_quota.rs` 的 `ZCODE_APP_VERSION`（额度查询与 Start Plan 共用这一处）
- [ ] 同步 `backend/src/gateway/adapter/zcode_start.rs` 的 `UA` 字面量（`balance_url_carries_app_version_and_ua_matches_it` 测试会在两者不一致时失败）
- [ ] 按 `backend/src/gateway/zcode_start_prompt.rs` 模块头注释，核对系统提示三块与环境段是否逐字节一致
- [ ] 对照 magpie 社区插件 `packages/zcode`（`index.mjs` 的 `APP_VERSION`、`prompt.mjs`）是否有同步改动
- [ ] 跑只读冒烟：`cargo test --manifest-path backend/Cargo.toml --lib local_start_plan_gift_readonly_smoke -- --ignored --nocapture`

## 当前状态（2026-11-09）

- 版本统一为 `3.17.2`（此前额度查询用 `3.14.3`、Start Plan 用 `3.17.2`）；余额请求带 `?app_version=`。
- 只读冒烟：新版本号的余额请求正常返回，账号当前没有套餐（`plans=0`）。
