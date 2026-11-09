# ZCode 通道收尾与命名客户端 Key 修复（2026-11-09）

范围：`docs/plans/2026-11-09-magpie-parity-batch3.md` 批次 D 的 C1 / C5 / C2，签发表单接上 F1 的三项约束，修复命名 Key 的两处缺陷，统一 ZCode 版本号（C6 的常量部分）。参照 magpie 社区插件 `packages/zcode/index.mjs`（`startPlan`、`routes`、`edge`）。

## 改动

| 项 | 位置 | 行为 |
|---|---|---|
| C1 礼包按余额过滤 | `zcode_start.rs`：`gift_models_from_balance` / `gift_models`；`zcode.rs`：`models` | 生效套餐（`status=active`、已开始、未结束）下、未过期且未用完的桶，取 `capabilities` 里的 `model:xxx`，按 ZCode 注册表还原大小写。`None`（没有生效套餐）与 `Some(空)`（桶都用完）分开表达；按会话 JWT 哈希缓存 5 分钟，读失败不缓存。只在 Coding Plan / OAuth 两条链都列不出模型时用礼包补位，读不到礼包则如实报原错，不用静态清单冒充 |
| C5 边缘防火墙保持 | `zcode_start.rs`：`refusal_of` / `EdgeHold` / `settle` | 拒绝（405 或 3012 / unusual activity 文案）且回应是拦截页（`errors.aliyun.com` / HTML）→ 进程级保持 3 分钟，期间 Start Plan 请求直接回 503 并写明解除时间；3012 JSON 视为账号被风控，回可读的 403，不保持；成功请求清除保持 |
| C2 双通道按连接记忆 | `zcode.rs`：`channel_order` / `falls_back` / `forward_channels` | 成功的通道按连接 id 记 10 分钟，下次先走它；没有会话 JWT 时只走 Coding Plan；换通道条件沿用原口径（Coding Plan：402/429/502/504；Start Plan：非 2xx） |
| 版本号统一 | `zcode_quota.rs`：`ZCODE_APP_VERSION` / `balance_url` | 额度查询由 `ZCode/3.14.3` 改为与 Start Plan 一致的 `3.17.2`，余额请求带 `?app_version=`（余额端点按版本判定礼包能力） |
| 命名 Key：白名单 | `access.rs`：`check_client_key_limit_for` | **缺陷**：入口在读 body 前以 `model = None` 检查，设了白名单的 Key 所有请求都被拒成「不允许使用模型（未指定）」。现在入口跳过白名单，由 wire 层带模型名检查 |
| 命名 Key：状态码 | `access.rs` / `server.rs` / `wire.rs` | **缺陷**：过期、只读、白名单拒绝都回 429 + `Retry-After: 1`，客户端会当限流反复重试。现在过期 401、只读与白名单 403、当日到量 429，只有 429 带 Retry-After |
| 命名 Key：过期 Key 的读请求 | `server.rs`：`authorize_and_limit` | 原先只检查计费的 POST，过期 Key 仍能读模型清单；现在每个请求都查过期，读请求仍不受只读与当日请求数限制 |
| 签发表单 | `SharedAccessDialog.tsx`、`sharedAccess.ts` | 新增有效期（永不 / 1 / 7 / 30 / 90 天）、模型白名单（逗号或空白分隔）、只读；列表显示「N 天后过期 / 已过期 / 限 N 个模型 / 只读」。界面无画板，标「待确认」 |

## 验证

| 项 | 结果 |
|---|---|
| 新增单测（先确认失败再实现） | 后端 17 项：礼包解析 3、缓存 1、版本一致 1、拒绝判定 1、边缘保持 1、拒绝回应 1、通道顺序 / 换通道 / 记忆 3、通道转发 3、命名 Key 3；前端 4 项 |
| `cargo test --manifest-path backend/Cargo.toml` | 1135 通过，0 失败，43 忽略 |
| `pnpm --dir frontend typecheck` | 通过 |
| 前端逻辑测试（lib、hooks、components） | 433 通过 |
| 只读真机冒烟 `local_start_plan_gift_readonly_smoke` | 新版本号的余额请求正常返回；账号当前 `plans=0`，礼包结论为「没有生效套餐」 |

## 未验证

- 真实余额里 `capabilities` 的写法：当前账号没有套餐，没有样本；解析依据是 magpie 插件源码与其测试。
- C2 / C5 的端到端行为：要等 ZCode 额度恢复；C5 无法主动触发，只能靠日志观察。
- 签发表单的实机点击（签发带约束的 Key、白名单外模型返回 403、过期后 401）。
- 原有的 `local_zcode_balance_readonly_smoke` 断言本机登录是 zai 家族，本机现为 BigModel 家族，前置条件不满足，未运行到网络请求。
