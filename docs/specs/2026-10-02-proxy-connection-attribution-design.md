# 代理流量按连接归账 · 设计说明（2026-10-02）

鼠鼠批准的方案；实现随 `todo/2026-10-01-platform-completion.md` 第 5 组落地。

> 2026-11-11 核对（对照 `backend/src/lib.rs` 的 `generate_handler!` 与 `backend/src/db.rs` / `usage_stats.rs` / `attribution.rs`）：识别、落库（`db::apply_proxy_timing`）与备份 `attributions` 段与下文一致；「查询」一节的命令名已过期：
> - 不存在 `connection_attributed_count` 命令；归账记录数的实际实现是 `db::attributed_request_count`（仅后端内部与测试使用，未注册为 Tauri 命令），以及 `stats_overview` 返回的 `Overview.attributed` 字段（`usage_stats::count_attributed`）。
> - 六个旧统计命令已不再注册；`db.rs` 里的 `*_scoped` 函数仍在，但当前只有 `cost_estimate` 命令带可选参数 `connection`。用量统计页改用 `stats_overview` / `stats_calls` / `stats_activity`，按 `usage_stats::StatsFilter.connections`（多选，可含 `__unattributed__`）筛选。
> - 下文「本机统计范围：全部本机记录 / 仅此连接」分段控件在当前前端中搜不到，已由用量统计页的账号筛选取代。
> - `backend/src/attribution.rs` 是后来新增的实时归因（exact / mapped / platform 三档，供灵动岛显示正在跑的连接，经 `live_usage` 返回的 `attribution` 字段），不提供独立 Tauri 命令。

## 问题

`requests` 表只按平台存储，本机统计无法区分同平台下的多个账号或 API Key。本地代理只知道平台与上游地址，不知道是哪条连接；「启用连接」写进 CLI 的配置也不带连接 id。

## 方案

- **识别**：代理转发时取请求头里的凭证（Claude 优先 `x-api-key`，其次 `Authorization: Bearer`；Codex 用 Bearer），只在内存里算 SHA-256，与该平台已保存的 API Key 连接指纹比对，命中即得到连接 id。指纹索引 30 秒重建一次，拿不到数据库锁时沿用旧索引。
- **落库**：`requests` 新增可空列 `connection_id`，随计时一起由 `apply_proxy_timing` 回填（`COALESCE` 保留已有值）；记录尚未入库时随待关联队列补写。未经代理的记录保持 NULL。
- **查询**：统一过滤器 `Filter::new` 增加可选连接条件，六个统计查询与请求日志的 `*_scoped` 版本接受 `connection`；Tauri 命令同名增加可选参数 `connection`，旧调用方不受影响。新增 `connection_attributed_count` 返回归账记录数。
- **界面**：主面板选中连接且该连接有归账记录时，显示「本机统计范围：全部本机记录 / 仅此连接 · N 条」分段控件；切换后统计卡、趋势、日志、费用估算与模型下拉都按连接筛选，提示文案同步改写。换平台后回到全部记录。
- **备份**：导出文件新增 `attributions` 段（来源、去重键、连接 id），导入时回填；旧备份没有该段，保持无归属。

## 边界

- 只覆盖开启代理之后、走网关的 API Key 连接；OAuth 令牌会轮换，不参与匹配；官方直连本就不经代理。
- 不把密钥明文或哈希写入磁盘；索引只在进程内存。
- 灵动岛与托盘的实时数仍按平台合并，不随该范围切换。
