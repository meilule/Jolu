# 阶段 1：交付了但用不上 / 数据风险（2026-11-09）

范围：`docs/plans/2026-11-09-complete-remaining-work.md` 阶段 1 的 1.1–1.8。1.9、1.10 未做。

## 改动

| 项 | 位置 | 行为 |
|---|---|---|
| 1.1 中间件配置入口 | `service/gateway/middleware.rs`、`lib.rs`（`gateway_middleware_list / save / delete`）、`MiddlewareDialog.tsx`、`middlewareForm.ts` | 保存前在样例请求上试跑，执行链会跳过的配置直接拒存；弹窗可增删改、启停、调顺序。think-tags 的 `wrap` 原先没有任何效果，现在把 OpenAI `reasoning_content` 与 Anthropic thinking 块包进 `<think>` 放到正文前（仅非流式） |
| 1.2 成员暂停 | `routingGroups.ts`、`RoutingGroupsSection.tsx`、`gatewayApi.ts` | 已勾选成员可暂停 30 分钟 / 1 / 4 / 24 小时或立即恢复，随「保存」写入；成员类型带 `paused_until_ms`，保存分组不再丢暂停字段；smart / pace 文案改为实际排序依据 |
| 1.3 ZCode 通道开关 | `ZcodeChannelOption.tsx`、`settingsApi.ts` | 「管理来源」弹窗底部，池里有 ZCode 来源时显示 |
| 1.4 计价时间 | `usage_stats.rs`、`db.rs`（汇总、日志、趋势）、`session_threads.rs` | 全部按请求自身时间计价；汇总与趋势在原分组上再按 `(ts / 3600000) % 168` 分桶（峰谷价只取决于 UTC 小时与星期） |
| 1.5 lastgood | `settings.rs` | 保存成功写 `settings.json.lastgood`；主文件存在但解析失败时另存 `.corrupt` 并回读上一版，两者都没有才用默认值 |
| 1.6 steady | `steady.rs`；`gateway/wiring.rs`、`proxy_config.rs`、`creds.rs`、`cli_apply.rs`、`settings.rs` | 遇到不存在 / 被占用（含 ERROR_SHARING_VIOLATION）时 10 × 3ms 重试。修掉的风险：接线写入前 `path.exists()` 落在 rename 窗口会得到空配置，随后按空配置写回、覆盖用户原有配置 |
| 1.7 Token 预算 | `gateway/budget.rs`、`wire.rs`、`access.rs` | 命名 Key 设了 `tokens_per_day` 时按「字节 ÷ 4 + max_tokens（缺省 4096）」预扣，在途预扣计入上限；响应流透传时读 usage，结束或客户端断开时回冲；读不到用量按预扣额记，上游非 2xx 且无用量按 0 记；超量 429 并带到次日零点的 Retry-After |
| 1.8 ZCode 版本 | `zcode_version.rs`、`Cargo.toml`（`windows-sys` 加 `Win32_System_Registry`）、`lib.rs`（`zcode_version_check`、启动日志）、`ZcodeChannelOption.tsx` | 读卸载注册表里显示名以「ZCode」开头的 `DisplayVersion`，按数字比较；比适配版本新时提示 |

## 验证

| 项 | 结果 |
|---|---|
| 新增后端测试 | 中间件服务 3、think-tags wrap 1、计价时间 2、steady 2、lastgood 2、预算 8、ZCode 版本 2（另有 1 项 ignored 的注册表冒烟） |
| 新增前端测试 | 中间件表单 8、成员暂停 3 |
| `cargo test --manifest-path backend/Cargo.toml` | 1154 通过、1 失败、44 忽略。失败项 `handoff_facts::tests::redact_is_linear_on_pathological_input` 为原有耗时测试（1 秒上限），本轮未改该文件；单独连跑 3 次均通过（约 1.2 秒） |
| `pnpm --dir frontend typecheck` | 通过 |
| 前端逻辑测试（lib、hooks、components） | 444 通过 |
| 注册表只读冒烟 | 本机读到 ZCode `3.17.2`，与适配版本一致 |

## 未验证

- 中间件弹窗、成员暂停、ZCode 通道开关的桌面端实机点击；`Jolu.pen` 无对应画板，未做截图对照。
- Token 预算未用真实客户端走过；估算是粗略上界，临近上限时会比实际更早拒绝（界面已写明）。
