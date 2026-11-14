# ZCode 会话在灵动岛的运行状态（`session.time_updated` 信号）

日期：2026-11-14
范围：`backend/src/native_sources.rs`、`frontend/src/hooks/useLiveUsage.ts`、`frontend/src/hooks/useIslandWindowModel.ts`

## 1. 问题

灵动岛会话行里，正在执行的 ZCode 会话显示「已完成」徽标，不会在开跑后变回运行态。
（截图：`学习抖音视频的前端设计 · ZCode · 已完成 · 耗时未知 · +59.3K Token`，而该会话当时正在流式执行。）

## 2. 根因

ZCode 的 `model_usage` **只在每条请求结束时落行**。一轮对话由多条请求组成，正在跑下一条
请求时，该会话最后一行还是上一条请求的 `completed` → `zcode_session_state` 映射成 done →
「已完成」。行数据里此刻没有任何「正在跑」的痕迹。

## 3. 信号与实测

`session` 表的 `time_updated` 随会话活动实时前进（新消息、流式 part 落库都会更新）。
实机测量（2026-11-14，四条会话的 `time_updated − 最新 usage 行 ts`）：

| 会话 | 差值 |
|---|---|
| 正在流式执行（学习抖音…） | **3649ms，且随时间持续增长** |
| 刚完成的（Fork…、子代理、你好） | 13–33ms |

一轮结束后 usage 行总是最后落库，差值只有几十毫秒；正在执行的会话差值持续增长。
**2 秒阈值**（`ZCODE_TURN_GAP_MS`）能干净分开这两种形态。

## 4. 实现

- `native_sources.rs`：探测 `session.time_updated` 列（旧版缺列时 NULL 占位，退回只看行状态）；
  会话汇总后叠加 `zcode_boost_running`——最新行状态不是 running、但 `time_updated` 比它
  新出 2 秒且仍在 [`ZCODE_RUNNING_GRACE_MS`]（10 分钟）宽限期内 → running。
  CLI 异常退出时 `time_updated` 一并停走，同样被宽限期拦住，不会长期假 running。
- `useLiveUsage.ts`：`activityKind` 对 zcode 动态化——有 running 会话时与 claude / codex
  同为 running 档（区块标题、`islandWriting` 信号 1 生效），否则仍是「最近活跃」。
- 前端其它逻辑不变：running 行显示绿点（与 claude / codex 一致），本轮 Token 锁存、
  「正在写」眼色等三信号机制照旧工作。

## 5. 测试

- `zcode_session_time_updated_boosts_recently_active_sessions_to_running`（纯函数）：
  33ms 差不抬、3.6s 差抬、failed 行同样抬、已是 running 不动、缺列不抬、超宽限期不抬、未来时间戳不抬。
- `zcode_time_updated_marks_active_turns_running`（合成库端到端）：带 `time_updated` 列的
  schema 下，completed 行 + 新活动 → running；收尾 33ms 差 → done；超宽限期 → done。
- 全量：`cargo test --lib` **1384 passed / 0 failed**；`pnpm --dir frontend typecheck` 通过。

## 6. 真机验证

重启 debug 构建后（用户会话正在执行）：

- 胶囊 aria 状态「Zcode（Zcode），工作中」；
- 额度环实时显示本轮 running tokens（7.1k → 11k 连续上涨）；
- 会话跑完后落库 `activity_state = done`、`activity_updated_ms` 为收尾时刻——收尾差几十毫秒，
  判 done 正确；下一轮开跑时 `time_updated` 立刻领先 usage 行 2 秒以上，行状态翻回 running。
