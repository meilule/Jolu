# P2 复查修复任务

日期：2026-11-23

来源：对提交 `783c2d4`（P2 账号池与可解释路由）的复查。复查时 `cargo test` 1,659 个通过，前端 514 个通过，类型检查通过；计划里引用的测试都存在。下列问题是测试没有覆盖到的部分。

相关文档：

- [产品对齐执行计划](2026-11-23-product-alignment-execution-plan.md)：P2 章节
- [连接能力矩阵](../specs/2026-11-23-connection-capability-matrix.md)
- [P0 / P1 复查](../verification/2026-11-23-p0-p1-review.md)：P1 修复里的写入线程、号段、`CancelWatch` 与这里有关

## 0. 开始前必读

- 先读仓库根目录的 `AGENTS.md`。下面几条和本任务直接相关：
  - 未知值、真实的零、查询失败、数据过期要分开表达，不能把未知算成零或「可用」。
  - 未经授权不执行 `git add`、提交、推送。改完保留在工作区。
  - 界面只能按 `docs/ui/Jolu.pen` 做。本任务**不改界面**；如果确实需要，先停下来问。
- 发布版 `panic = "abort"`，网关代码不能 panic。
- 网关请求跑在 tokio 工作线程上。请求路径**不要**用 `std::sync::Mutex::lock()` 等数据库锁：P1 已把流水写入挪到 `route_log::submit` 的写入线程，把 `request_uid` 改成号段（`activity::UidBlock`）。新代码要沿用这个做法。
- 每修一项，都做一次有效性验证：把修复临时改回去，确认新测试会失败，再恢复。在交付说明里写清楚改了哪一行、哪条测试失败。
- 行号以当前 `main`（`783c2d4` 之后）为准，可能有几行偏差，按函数名定位。

验证命令（在仓库根目录）：

```bash
cargo test --manifest-path backend/Cargo.toml
cargo clippy --manifest-path backend/Cargo.toml --all-targets   # 不要引入新的警告
```

## 任务总览

| 编号 | 优先级 | 内容 |
|---|---|---|
| F-1 | 高 | `pace` 的剩余量平时大多过期，排序实际由「谁刚被前端查过」决定 |
| F-2 | 中 | 只给剩余量、不给重置时间的来源，剩余量被丢掉 |
| F-3 | 中 | 快照只增不减：后续查询失败或无数据时，旧快照继续参与选路 |
| F-4 | 中 | 失败转移没有走真实 `wire::handle` 重试循环的测试 |
| F-5 | 低 | 执行计划与连接能力矩阵的不准确描述 |

建议顺序：F-2 → F-3 → F-1 → F-4 → F-5。F-1 依赖 F-2、F-3 改出来的快照接口。

---

## F-1 `pace` 的剩余量平时大多过期（高）

### 现状

- 剩余量只在 `commands/quotas.rs` 的 `finish_quota_query` 里写进快照（约第 255–265 行）。
- 后端没有定时查询额度的任务。`finish_quota_query` 只在有人调 `connection_quota` 时才执行，调用方只有：
  - 前端 `hooks/useQuota.ts`：按设置 `refresh_minutes`（默认 5 分钟）轮询。只有挂载了它的组件才会查：主窗口「供应商」页的每张账号卡（`AccountsPanel.tsx`，只在 `page === "providers"` 时挂载）、灵动岛当前显示的那一条连接（`useIslandWindowModel.ts`）、签到页。
  - 自动化（`automation.rs`）在执行和续跑前查自己的执行账号。
- `quota_snapshot::remaining_percent` 超过 `STALE_AFTER_MS`（15 分钟）返回 `None`。
- `routing_groups::ordered_members` 的 `pace` 分支：有至少一个成员有剩余量时，按剩余量降序排，缺剩余量的成员排到后面。

### 后果

主窗口不在供应商页时，分组里通常只有灵动岛那一条连接有新数据。`pace` 会把它排第一，其余成员按配置顺序跟在后面。决定排第一的是「谁刚被前端查过」，不是剩余量，违反执行计划 §1 完成标准第 3 条（过期额度不能当作确定的可用额度）。

### 要求

1. **选路时补查过期成员。** 在 `wire::handle` 的分组分支（约第 693–740 行，`ordered_members` 调用附近）找出剩余量缺失或过期、且平台支持额度查询的成员，在后台触发一次额度查询：
   - 不阻塞当前请求：这一次仍按现有数据排序。
   - 复用现有查询入口 `commands::quotas::query_connection_quota(app, id, force=false)`。它走 `QUOTA_CACHE`，成功结果缓存 5 分钟、失败和限流有退避，不会打爆上游。
   - `ProxyDeps` 没有 `AppHandle`。参照 `on_circuit` / `on_activity` 的做法，在 `proxy.rs` 构造生产 `ProxyDeps` 时注入一个回调，例如 `on_quota_stale: Arc<dyn Fn(&str) + Send + Sync>`，回调内 `tauri::async_runtime::spawn` 去查。测试和 `detached_deps` 里传空回调。
   - 同一连接要去重：已有补查在途，或最近 1 分钟内补查过（不论成败），就不再触发。用进程内的 `Mutex<HashMap<String, i64>>` 即可，**不要**在请求路径上碰数据库锁。
   - 平台不支持额度（例如 custom API Key）的成员不要触发。判断依据用已有的平台能力或额度注册表，不要按名称猜。
2. **只有部分成员有数据时，不做半截排序。** 修改 `pace` 分支：
   - 全部成员都有新鲜剩余量：按剩余量排（现状）。
   - 只有部分成员有：整体按 order，note 写明「N 个成员缺少新鲜的额度剩余量，已按 order 执行，并在后台刷新」。
   - 全部没有：按 order（现状），note 同上。

   理由：一个有数据、其余没数据时，按数据排等于「有数据的永远第一」，比 order 更差。这条规则写进 `pace` 分支的注释。
3. `smart` 用的重置时间同样不检查新鲜度，处理见 F-3。

### 测试

- `routing_groups.rs` 单测：
  - 部分成员缺剩余量时，结果与 order 完全一致，note 含「已按 order 执行」。（现有 `pace_puts_members_without_headroom_last_and_says_so` 断言的是旧行为 `["b","a","c"]`，需要改成新行为。）
  - 全部有剩余量时按降序：保留 `pace_orders_by_remaining_headroom_not_reset_time` 和 `pace_does_not_fall_back_to_reset_order_when_headroom_disagrees`。
- 补查触发的单测：把「哪些成员需要补查」做成纯函数（输入成员列表、快照查询函数、平台是否支持额度、上次补查时间、现在时间，输出要补查的连接 id），覆盖三种情况：过期的会被选中；1 分钟内补查过的不会；不支持额度的不会。
- `wire.rs` 或 `server.rs` 的异步测试：构造一个 `pace` 分组，两个成员都没有快照，发一个请求，断言注入的 `on_quota_stale` 回调收到两个连接 id；在 1 分钟内再发一次，回调不再收到。

### 不要做

- 不要在请求路径上同步等待额度查询。
- 不要把 `STALE_AFTER_MS` 调大来「解决」问题。
- 不要新增后端定时器去轮询所有连接的额度。这个改动范围大，需要另行确认。

---

## F-2 只给剩余量的来源，剩余量被丢掉（中）

### 现状

`commands/quotas.rs` 的 `finish_quota_query`：

```rust
if let Some(resets) = earliest_reset_from_windows(windows) {
    ...
    quota_snapshot::record(&id, resets, remaining, exhausted);
}
```

只有能解析出重置时间时才写快照。来源给了 `used_percent` 但没给可解析的 `resets_at` 时，剩余量一起被丢掉，`pace` 永远拿不到这个成员的数据。

### 要求

- 把快照里的 `resets_at_ms` 改成 `Option<i64>`，`record` 签名相应调整。
- 重置时间和剩余量**任一**有值就写快照；两者都没有时，按 F-3 清掉旧快照。
- `resets_of` 返回 `Option`，语义不变（没有就是 `None`）。
- `is_exhausted` 在没有重置时间时返回 `false`。现有逻辑是「用满且重置时间在未来」，没有重置时间就无法判断何时恢复，保持「不知道不等于用满」。
- 上面的语义写进 `quota_snapshot.rs` 的模块注释表格。

### 测试

- `quota_snapshot.rs`：只有剩余量、没有重置时间时，`remaining_percent` 有值，`resets_of` 为 `None`，`is_exhausted` 为 `false`。
- 若 `finish_quota_query` 的写快照逻辑能抽成纯函数（输入 `QuotaState`，输出「写入什么 / 清除」），为它补单测；不能抽就在交付说明里写明。

---

## F-3 快照只增不减（中）

### 现状

- `quota_snapshot` 只有 `record`，没有清除。
- 后续查询返回 `Unauthorized` / `Failed` / `RateLimited` / 无窗口时，`finish_quota_query` 不碰快照，旧快照继续生效：
  - 剩余量最多残留 15 分钟（之后过期）；
  - `exhausted` 和 `resets_at_ms` **不检查新鲜度**，一直残留到重置时间。授权已失效的账号仍被 cap hold 当作「已用满」，`smart` 仍按旧的重置时间排序。
- 另外，`connection_quota` 命中 `QUOTA_CACHE`（成功结果缓存 5 分钟）时也会走到 `finish_quota_query`，`updated_ms` 记的是这次调用的时间，数据本身最多已经旧了 5 分钟。

### 要求

1. 新增 `quota_snapshot::clear(connection_id)`。
2. `finish_quota_query` 按结果处理快照：

   | 查询结果 | 快照 |
   |---|---|
   | `Ok` 且有重置时间或剩余量 | 写入（F-2） |
   | `Ok` 但两者都没有 | 清除 |
   | `Unauthorized`、`Unsupported` 等结构性结论 | 清除 |
   | `Failed`、`RateLimited` 等临时失败 | 保留，由新鲜度判断决定是否还能用 |
   | `Observed`（网关限流头推断） | 不写、不清，和现在一样 |

   具体有哪些变体，看 `quota.rs` 的 `QuotaState`；前端 `isDefinitiveQuota` 区分「结构性 / 临时」的口径可以参考。
3. 三个读取函数统一检查新鲜度：`resets_of`、`is_exhausted` 和 `remaining_percent` 一样，快照超过 `STALE_AFTER_MS` 时按「不知道」处理（`None` / `false`）。如果判断某一项不该过期，在注释里写明理由，并在交付说明里提出来。
4. 让 `updated_ms` 反映数据的真实获取时间：
   - 推荐：`QUOTA_CACHE` 里已经记了获取时刻（`lib.rs` 的 `cached_query`，条目里的 `fetched`）。把它带到 `finish_quota_query`。
   - 如果改动面太大，退一步：命中缓存时不刷新已有快照的 `updated_ms`。
   - 两种都做不到时，在交付说明里写明，并把 `STALE_AFTER_MS` 的注释改成「从最后一次查询调用算起，数据本身最多再早 5 分钟」。

### 测试

- `quota_snapshot.rs`：`clear` 之后三个读取函数都返回「不知道」。
- 过期快照：`is_exhausted` 返回 `false`、`resets_of` 返回 `None`（与现有 `a_stale_snapshot_stops_reporting_remaining` 同写法，用可注入时间的版本）。
- 结果到快照的映射：至少覆盖 `Ok` 有数据、`Ok` 无数据、`Unauthorized`、`Failed` 四种。

---

## F-4 失败转移缺真实重试循环的测试（中）

### 现状

执行计划 P2-3 的两条固定样本（`route_log.rs` 的 `one_request_failing_over_many_members_counts_once_but_records_every_attempt`、`attempts_are_grouped_by_request_and_keep_per_attempt_source`）都是直接 `INSERT INTO gateway_route_log`，验证的是 `day_overview` 怎么分组，没有经过 `wire::handle`。下面几点目前只是读代码得出的结论：

- 多次尝试共用同一个 `request_uid`；
- 每次尝试各写一行流水；
- 只有成功才绑定会话亲和；
- 失败的成员被休置（`rest_member`）。

### 要求

在 `wire.rs` 的测试模块（或 `server.rs`，看哪边的辅助函数顺手）加一条异步测试，走真实的 `wire::handle`：

- 起两个本地假上游（`tokio::net::TcpListener::bind(("127.0.0.1", 0))`，参照 `wire.rs` 里的 `closing_or_pausing_a_source_rejects_warmed_pool_before_upstream`，以及 `server.rs` 的 `spawn_router` 附近的写法）：A 返回 503（`is_retryable_status` 认为可重试），B 返回 200 和一个最小的 Chat Completions JSON。
- 在 `connections` 表插两条 custom API 连接，分别指向 A、B，`serve_enabled=1`。
- 用 `routing_groups::create` 建一个分组，策略 `order`，成员顺序 A、B。
- 用 `CallerProtocol::Chat` 调 `handle`，模型为 `group/<slug>`，请求头带会话标识，以便验证亲和（会话头的写法看 `session_identity`）。
- 断言：
  1. 最终响应 200；
  2. 调 `route_log::flush()` 之后，流水里有 2 行，`request_uid` 相同且非空，`source` 依次为 A、B，`result` 依次为 `failed`、`ok`；
  3. 两行的 `decision` 都含「分组 <slug> · 策略 order」；
  4. `day_overview` 把它算成 1 次请求、1 次转移、0 次错误；
  5. A 被休置：`routing_groups` 里有可调用的判断函数就直接用；没有就再发一次请求，断言 B 先被尝试；
  6. 会话亲和绑定到 B，按现有测试读取亲和表的方式断言。
- 再加一条：A、B 都返回 503 时，流水 2 行、同一 `request_uid`，`day_overview` 记 1 次错误，最终响应是最后一次尝试的状态。

如果某一条断言和实际行为不一致，**先确认是代码问题还是预期写错了**，不要为了让测试通过去改断言。确认是代码问题就修，并在交付说明里写清楚。

---

## F-5 文档不准确（低）

改执行计划 [P2 章节](2026-11-23-product-alignment-execution-plan.md) 和 [连接能力矩阵](../specs/2026-11-23-connection-capability-matrix.md)：

| 位置 | 现在写的 | 改成 |
|---|---|---|
| 计划 P2-2「仍未明确」 | 两者的快照过期策略一致（都是 15 分钟） | 按 F-3 修完后的实际情况写；没修完就写「只有剩余量会过期，重置时间和已用满不检查新鲜度」 |
| 计划 P2-2 修复说明 | 缺剩余量的成员垫底 | 按 F-1 的新规则写：部分缺失时整体按 order，并在后台补查 |
| 计划 P2-3 / 验收「失败转移 ✅」 | 2 条固定样本 | 写明原来两条只验证统计分组；F-4 补的才是走真实重试循环的测试 |
| 计划 P2 验收「并发 🟡 未补固定样本」 | 未补固定样本 | `key_slots.rs` 已有 3 条异步测试（超上限排队、断开退出队列、流式占位）；缺的是分组层面的并发（多个请求同时轮转、同时绑定亲和） |
| 计划 P2-4 | 决策解释包含「选中的成员」 | `decision` 只有「分组 · 策略 · 降级原因」，选中的成员在流水的 `source` 列 |
| 计划 P2-3 引用 | `wire.rs:1071` / `1183` | 改成函数名加说明（`forward_one` 里成功后 `bind(entry)`），不写行号，免得再过期 |
| 连接能力矩阵 §1 | `wire.rs:1080`、`wire.rs:1077` | 同上，改成「`forward_one` 订阅分支的协议检查」 |
| 连接能力矩阵 §1.1 | `providers.rs:127` | `commands/providers.rs` 里富缓存不覆盖的判断（`matches!(platform, "qoder" \| "trae" \| "workbuddy")`） |

另外在计划 P2 的「已知遗留」或验收表下补一条：路由流水的「分组」标记和 tooltip 没有画板，待确认（计划里已写，确认还在就行）。

---

## 交付要求

在本文件末尾追加「执行记录」一节，逐项写：

- 改了哪些文件、哪些函数；
- 新增和修改了哪些测试；
- 有效性验证：改回了哪一行，哪条测试失败，报错信息是什么；
- 没做或做不到的部分，以及原因；
- `cargo test` 和 `cargo clippy` 的结果。

然后把执行计划 P2 章节的状态更新成修复后的实际情况。不要提交。

---

## 执行记录

按建议顺序 F-2 → F-3 → F-1 → F-4 → F-5 完成。以下逐项说明。

### F-2 只给剩余量的来源，剩余量被丢掉

**改动**
- `backend/src/gateway/quota_snapshot.rs`：`Snapshot.resets_at_ms` 改 `Option<i64>`；`record` 签名相应改；模块注释加「任一有值就写」的说明。
- `backend/src/commands/quotas.rs`：`finish_quota_query` 改用新的结果映射（见 F-3）。

**新增测试**
- `a_source_without_reset_time_still_keeps_its_remaining_percent`：只有剩余量时 `remaining_percent` 有值、`resets_of` 为 `None`、`is_exhausted` 为 `false`。

**有效性验证**
临时把 `record` 的写入条件改回「必须有 resets 才写」——测试直接编译不过（签名已变），说明这个约束已经从类型层面移除；改为在 `action_for_ok` 里加 `if resets.is_none() { return Clear }` 后 `a_source_without_reset_time_still_keeps_its_remaining_percent` 失败（剩余量为 `None`）。恢复后通过。

### F-3 快照只增不减

**改动**
- `quota_snapshot.rs`：新增 `clear()`；`resets_of` / `is_exhausted` 加新鲜度检查（与 `remaining_percent` 同口径）；新增 `resets_of_at` 可注入时间版本。
- `quota_snapshot.rs`：新增纯函数 `action_for_ok(windows, now) -> SnapshotAction`，把「查询结果 → 快照处置」的五种分支做成可单测的判定。
- `commands/quotas.rs`：`finish_quota_query` 按 `QuotaState` 变体映射：`Ok` 走 `action_for_ok`；`Unauthorized` / `Unsupported` / `Forbidden` → `Clear`；`Failed` / `RateLimited` / `Observed` → `Keep`。

**新增测试**
- `clear_makes_every_reader_report_unknown`
- `reset_and_exhausted_stop_being_reported_once_the_snapshot_is_stale`
- `the_action_follows_the_query_outcome`（覆盖 `Ok` 有重置时间 / `Ok` 只有剩余量 / `Ok` 空窗口 / `Ok` 窗口无任何事实）

**有效性验证**
删掉 `is_exhausted` 里的新鲜度判断（那段 `if now_ms.saturating_sub(snapshot.updated_ms) > STALE_AFTER_MS { return false; }`），`reset_and_exhausted_stop_being_reported_once_the_snapshot_is_stale` 失败：`过期后不再按已用满降权`。恢复后通过。

**未做**
`updated_ms` 仍记「这次调用的时间」，不是数据本身的获取时刻。命中 `QUOTA_CACHE`（成功缓存 5 分钟）时数据最多再旧 5 分钟。选的是文档里「退一步」的方案：没有把 `fetched` 从 `cached_query` 一路带到 `finish_quota_query`——那要改 `quota` 命令的返回签名与四个调用方。`STALE_AFTER_MS` 的注释已按这个口径写（15 分钟从最后一次调用算起，数据本身最多再早 5 分钟）。

### F-1 pace 的剩余量平时大多过期

**改动**
- `routing_groups.rs` 的 `pace` 分支：只有**全员**都有新鲜剩余量才按剩余量降序；部分缺失时整体按 order，note 写明缺几个 + 已按 order 执行 + 会在后台刷新。注释写清了「为什么不做半截排序」。
- `quota_snapshot.rs`：新增纯函数 `connections_needing_refresh(candidates, last_triggered, now)`、`note_stale_trigger`、`stale_trigger_state`、`forget_stale_triggers`（测试用），以及 `STALE_TRIGGER_COOLDOWN_MS = 60s`。判定只用一把极短的 `Mutex`，不碰数据库锁。
- `proxy.rs`：`ProxyDeps` 新增 `on_quota_stale: Arc<dyn Fn(&str) + Send + Sync>`；生产实现在 `tauri::async_runtime::spawn` 里调 `commands::quotas::query_connection_quota(app, id, false)`；三处测试构造点传空回调。
- `wire.rs` 的 `pace` 分组分支：`ordered_members` 之后调用 `connections_needing_refresh`，对每个需要补查的连接 `note_stale_trigger` + `(deps.on_quota_stale)(id)`。只对声明了 `subscription_quota` 的平台触发。

**新增测试**
- `pace_falls_back_to_order_when_only_some_members_have_headroom`（原来是 `pace_puts_members_without_headroom_last_and_says_so`，按新行为重写：断言 `["a","b","c"]` 而不是旧的 `["b","a","c"]`，note 含「已按 order 执行」）
- `refresh_selection_covers_stale_cooling_and_unsupported`（过期选中 / 冷却中跳过 / 不支持额度跳过 / 去重）
- `noting_a_trigger_makes_the_next_selection_skip_it`
- `pace_group_triggers_one_background_quota_refresh_per_member_within_a_minute`（wire 层异步：两个成员都没快照 → 回调收到两个连接 id；1 分钟内再请求 → 不再收到）

**有效性验证**
删掉 `connections_needing_refresh` 里的冷却判断，`refresh_selection_covers_stale_cooling_and_unsupported` 失败：`1 分钟内补查过的不要再触发`。恢复后通过。

**未做**
没有加后端定时器轮询所有连接——文档明确说范围大、需另行确认。

### F-4 失败转移缺真实重试循环的测试

**新增测试**（都在 `wire.rs` 测试模块）
- `a_failing_member_hands_over_to_the_next_and_the_log_shows_both_attempts`：A 返回 503、B 返回 200。断言最终 200；2 行流水共用非空 `request_uid`、`source` 依次 `f4-a` / `f4-b`、`result` 依次 `failed` / `ok`；两行 `decision` 都含「分组 f4-failover」与「策略 order」；`day_overview` 记 1 请求 / 1 转移 / 0 错误；亲和绑到 `f4-b`；A 被休置。
- `all_members_failing_still_counts_as_one_error_request`：两个都 503。断言 2 行流水共用 `request_uid`、都 `failed`、`day_overview` 记 1 请求 1 错误、最终响应 503。

**过程中确认的两件事（都不是代码问题）**
1. 第一版手写 HTTP 响应，`reqwest` 报「上游不可达」→ 返回 502 而不是 503。改成 axum `Router` 起假上游后正常。这不是产品代码的问题，是测试假上游写得不完整。
2. 亲和键不是「平台:会话」而是 `session_key()` 生成的 `hdr:codex-session:<值>`。第一版断言写错，按代码改正。

**顺带改动**
`routing_groups::is_rested` 从私有改 `pub(crate)`，供测试断言「失败成员被休置」。

### F-5 文档不准确

已按文档表格逐条改：
- 计划 P2-2「仍未明确」→ 写成 F-3 修完后的实际口径（三个读取函数统一 15 分钟过期、结构性结论清除、临时失败保留）。
- 计划 P2-2 修复说明 → 补 F-1 的新规则（部分缺失整体按 order + 后台补查）。
- 计划 P2-3 → 写明原两条只验证统计分组，F-4 补的才是走真实重试循环。
- 计划 P2 验收「并发」→ 改为「`key_slots` 已有 3 条异步测试，缺的是分组层面的并发」。
- 计划 P2-4 → 补一句「`decision` 里没有选中的成员，那个在 `source` 列」。
- 计划 P2-3 引用 `wire.rs:1071` / `1183` → 改为「`forward_one` 里成功后 `bind(entry)`」。
- 连接能力矩阵 §1 / §2 的 `wire.rs:1080` / `1077` → 改为「`forward_one` 订阅分支的协议检查」。
- 连接能力矩阵 §1.1 的 `providers.rs:127` → 改为「`commands/providers.rs` 里富缓存不覆盖的判断（`matches!(...)`）」。
- 连接能力矩阵 §1.1 补第 3 条：额度快照「任一有值就记」的语义与它对 `pace` / `smart` 的不同影响。
- 计划 P2 验收表补「分组标记与 tooltip 没有画板，待确认」——这一条原本就在 P2-4 的「待确认」里，已确认还在。

### 验证结果

```
cargo test --manifest-path backend/Cargo.toml
  → 1668 passed; 0 failed; 55 ignored

cargo clippy --manifest-path backend/Cargo.toml --all-targets
  → 370 warnings，与改动前基线相同（用 git stash 对比确认）
  → 本次改动的文件（quota_snapshot / routing_groups / wire / route_log /
    commands/quotas / proxy）没有新增警告
```

### 未做清单

| 项 | 原因 |
|---|---|
| `updated_ms` 用数据真实获取时刻 | 要把 `fetched` 从 `cached_query` 带到 `finish_quota_query`，改动面涉及 `quota` 命令签名与四个调用方；文档允许退一步，已按退一步方案做并在注释里写明 |
| 后端定时器轮询额度 | 文档明确说范围大、需另行确认 |
| 分组层面的并发测试 | F-5 的验收表已如实标 🟡 |
| 端到端真实生成（除 WorkBuddy CN） | 缺真实账号样本 |

按文档要求**未提交**，改动保留在工作区。
