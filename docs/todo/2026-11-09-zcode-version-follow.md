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

## 2026-11-16 核对 3.18.2：暂不升级

本机 ZCode 3.18.2（卸载注册表 `DisplayVersion`，安装在 `D:\Zcode`）。Agent 引擎是 `resources/glm/zcode.cjs`，桌面主程序 `resources/app.asar` 里只有管理接口。只读比对的结果：

- `ai-sdk/anthropic` 仍是 `3.0.81`（变量 `$Qs="3.0.81"`），UA 只需把前缀换成 `ZCode/3.18.2`。
- `zcode_start_prompt.rs` 的各段与 3.18.2 原文逐行比对：前缀、日期行、Harness、动态行为、Skill 指引、上下文管理、模型行、环境段都一致。Harness 里「The system may send updates…」那一行在 3.18.2 中由三元表达式拼出，条件为真时文字相同。
- **结构性变化**：
  - 桌面段（`buildDesktopContextSection` / `createDesktopSection`）在 3.18.2 里是 `injectionTarget:"meta_user"`、`cacheHint:"dynamic"`，也就是从系统提示的 stable 块挪进了用户侧的动态元消息。3.17.2（插件 `prompt.mjs` 的 `stableSections[1]`、Jolu 的 `STABLE_SECTIONS`）是放在系统提示里的。
  - 没有可视化目录时，3.18.2 会**整段省略** `### Interactive views`；3.17.2 输出的是「No visualization output directory…」和「Widget modelContent…」两行。
  - 3.18.2 的段落清单还有 `kWo` / `wWo`（指引）/ 记忆段 / 环境段 / `xWo` / `TWo` / `p8e` 等，每段按自己的 `injectionTarget` 分别落到系统块或元消息。要还原完整的请求结构，得把这套分发逻辑读通。
- magpie 社区插件（`magpie-community/plugins` `fe19854`）已经处理了 3.18 的 `enc:v2` 登录（zcode 0.1.67），但 `APP_VERSION` 仍是 `3.17.2`，`prompt.mjs` 最后一次改动是 0.1.47（对齐 3.17.2）。
- 只读冒烟 `local_start_plan_gift_readonly_smoke`：3.17.2 的余额请求正常，`plans=0`。
- **决定：暂不升级。** 理由：
  1. 3.18.2 改的是请求结构，不是文案，还原容易出错。插件注释记录过，着装与版本号不一致会被风控拒绝（405/3012）。
  2. 唯一的验证方式是用真实账号发 Start Plan 模型请求，有风控风险；而且本账号目前没有套餐，测不了。
  3. 上游插件也停在 3.17.2，3.17.2 目前可用。`zcode_version.rs` 会照常提示「本机版本比适配版本新」。
- **以下情况再做**：插件的 `APP_VERSION` / `prompt.mjs` 跟进到 3.18；或者 3.17.2 的请求开始被拒（405/3012、余额端点按版本拒绝）；或者鼠鼠的账号有了 Start Plan 套餐、可以做真实请求验证。做的时候按上面的结构变化，把 `dress` 的系统块与元消息一起改。
