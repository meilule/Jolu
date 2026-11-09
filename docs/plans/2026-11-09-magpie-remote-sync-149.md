# magpie 远端同步 · 149 个新提交对照（2026-11-09 晚）

**参照物：** [yetone/magpie](https://github.com/yetone/magpie)，本地克隆从 `627fd1a`（11-08）更新到 `623e797`
**时间跨度：** 11-08 当天 27 个 + 11-09 当天 **122 个** —— magpie 的迭代速度是**一天 100+ 提交**
**本报告回答：** 这 149 个提交做了什么，哪些与 Jolu 重叠、哪些可借鉴、哪些修正了之前的审计结论。

---

## 0. 先修正一处审计错误

**C4「每账号并发队列」没有消失，是换了名字。**

之前的审计按 `DEFAULT_CONCURRENCY` 符号名搜索，搜不到就判「作废」。实际：

- `e215d33`（11-08，**就在克隆当天**）：网关 Key 新增 `MaxConcurrency`（1–1000），`key_slots.go` 按槽位排队，超出的请求等位
- `0024660`：Key 的槽位在账号轮到「近 cap」之前先占——防的是「账号快满了还把新请求排给它」

**教训**：按符号名搜会漏——功能改个名就「不存在」了。以后判断「magpie 没有某功能」必须按**行为**搜（比如搜 commit message 的关键词、搜测试文件名），不能只搜一个符号。

---

## 1. 149 个提交的分类

| 领域 | 数量 | 对 Jolu 的意义 |
|---|---|---|
| gateway | 17 | **高**：多条直接重叠 Jolu 刚做的功能 |
| gui | 16 | 低（形态不同） |
| site | 15 | 无（文档站，含 llms.txt 给 AI 搜索用） |
| routing | 11 | **高**：Smart 语义修正、In order 回切 |
| provider | 10 | 中：Kimi 钱包、WorkBuddy 上限修正、Kiro |
| sessions | 9 | 中：ZCode 会话去重（Jolu 也在解析 zcode） |
| agent | 6 | 中：新 agent（Claude Science、Antigravity effort） |
| library | 5 | 中：skill router、GitHub 跟随 |
| usage / update / settings / claude | 各 4 | update 里有可直接抄的下载重试 |
| plugin / browser / edit / catalog 等 | 其余 | 低-中 |

---

## 2. 与 Jolu 直接重叠的（magpie 又往前走了）

### 2.1 裸模型名的路由优先级（`608c154`，11-09）

**magpie 新规则**：裸模型名如果**询问 agent 自己的订阅**能服务（如 Codex 的 ChatGPT 账号、Claude Code 的 Claude 账号），**优先走它**——即使有按名字匹配到的分组也不走分组。

**Jolu 现状**：裸名只落默认来源（`gateway_default_connection`）。

**差异**：Jolu 没有这个「自己的订阅优先」概念。场景：用户在 Codex 里接了 Jolu，同时 Codex 自己还有 ChatGPT 登录——裸名 `gpt-6` 应该先走 ChatGPT 本尊，而不是 Jolu 的池子。magpie 发现不这么做会把本该自己服务的请求白白送给分组里的第三方（freefcw 的案例：gpt-6-astra 被送去了 Devin）。

**建议**：Jolu 记录「这个 agent 当前自己登录的账号」（`current_connection` 已有雏形），裸名路由时插到默认来源之前。

### 2.2 Key 并发上限（`e215d33`）

见 §0。Jolu 的 Key 只有每日请求数上限，**没有瞬时并发上限**。局域网共享多设备接入时，一把 Key 可以同时打满所有上游账号。

### 2.3 Smart 策略语义修正（`4436b7d`）

**magpie 澄清了 Smart 的语义**：「额度最早重新的先用；还没开始的窗口排最后」——所以**新加的账号在 Smart 下会一直等到别的账号用到 90% 才轮到**。GUI 文案之前写反了（说「首选的发请求」）。

**Jolu 检查点**：Jolu 的 `smart`（重置最近先用）实现语义一致，但 **UI 文案是否也给了用户同样的误解**需要核对——用户在分组里把某账号设为「首选」，Smart 模式下它可能一直不被用，这和用户预期相反。

### 2.4 In order 路由的会话回切（`82ca554`）

**magpie 修复**：provider 自己的账号列表设为 In order 时，会话因第一个账号休息而落到第二个，第一个恢复后**要切回去**。之前只有分组成员会回切，provider 级不会。

**Jolu 现状**：分组成员有会话亲和（绑定成员），但绑定是「粘住不回切」。magpie 的语义是「按顺序恢复」。两种语义各有道理，但**Jolu 没有提供「恢复后回切」的选项**。

---

## 3. Jolu 会踩的同一个坑（可直接借鉴的修复）

按价值排序：

| # | 提交 | 问题 | Jolu 是否会踩 | 修复量 |
|---|---|---|---|---|
| 1 | **`a6a9971`** | **无签名的 thinking 块发给 Claude 会 400**：Grok 产的 thinking 块没签名，分组切回 Claude 后整个请求连续 13 次 invalid_request | **会**。`translate.rs:120-162` 在多协议转换时搬运 thinking 块；路由分组跨模型切换是 Jolu 的核心场景 | 半天 |
| 2 | **`0f04dca`** | WorkBuddy 的 `deepseek-v4.1-flash` 回复上限：models.dev 说 384000，实际 **128000**。WorkBuddy 插件 0.2.26 自己修正 | **会**。Jolu 的 `models_dev.rs` 就是拉 models.dev，同样拿到错的上限；`exceeds_200k`（200k 判定）会被它误导 | 1 小时 |
| 3 | **`a46190e`** | 更新下载对 502/503/504/500/429 重试 | 会。Jolu 的 `updater.rs` 无重试 | 2 小时 |
| 4 | **`b657a21`** | URL-safe / 无填充 base64 要转标准 base64 再发上游 | 可能。取决于上游账号的 token 格式 | 1 小时 |
| 5 | **`fc8e98b`** | Codex 的 config.toml 有**几千个 `[projects]` 表**，不该全量解析 | 会。Jolu 的 codex 接线读写 config.toml | 半天 |
| 6 | **`a39b63e`/`7cabf0d`** | ZCode 会话的草稿文件夹要归成一组、会话只列一次 | **会**。Jolu 的 `session_threads.rs` 也在解析 zcode 的 sqlite | 待核对 |
| 7 | **`92e8edd`** | Windows「已安装应用」列出 magpie，卸载时**把每个 agent 的配置还原** | 部分。Jolu 的 NSIS 卸载钩子只删自启项，**不还原 agent 接线**——卸载 Jolu 后用户的 Claude Code 还指着已失效的网关端口 | 半天 |
| 8 | **`3480f82`** | library 的 instructions 丢失时，apply 只动 agent 部分，不碰其他 | 思路可借鉴到 Jolu 的接线恢复 | 思路 |

第 1、2 条建议**立即修**：一个是用户可见的连续失败，一个是错误的上下文标注。

---

## 4. 新能力（Jolu 没有的）

| 提交 | 能力 | 备注 |
|---|---|---|
| `39569d5` | **Claude Magpie**：在你自己的 Claude Desktop 旁边挂一个可选的 Claude app（#5671） | agent 阵容继续扩张 |
| `c056b7c` | Responses provider 的多轮可走 **WebSocket**（#5781） | Codex 链路的新通道 |
| `08a42c4` | **Remote magpie 链式传递**：一台远程 magpie 的卡片可以再往下游传 | 远程网关的级联 |
| `2a64b4b` | **skill router**：一个 `magpie-skills` skill 动态列出当前 agent 的 skills | 解决「skills 多了 prompt 膨胀」 |
| `dc098c1` | 从 GitHub 装的 skill **跟随更新** | Jolu 的 skills 有 Git 导入但没有跟随 |
| `81798f7` | 浏览器扩展**录制 MP4**（magpie 内编码） | browserext 在快速增强 |
| `7cce4b4` | 插件宿主超 **512MiB** 自动换新进程 | 插件内存治理 |
| `4314f4b` | OTLP 导出带 `magpie.gateway_key.name` 属性 | **Jolu 的 OTLP 也没有带 Key 名**——排查多 Key 问题时需要 |
| `22076f2` | `/llms-full.txt`：全站文档给 AI 搜索索引 | 15 个 site 提交都是这个方向——magpie 在认真做 AI 可发现性 |

---

## 5. 之前判「作废」的 C5（405 防火墙保持）复核

换关键词（405 / WAF / edge / firewall / StatusMethodNotAllowed）重搜 149 个提交与现有代码，仍无命中。**C5「不存在」的结论维持**。但鉴于 C4 的教训，此结论的可信度标注为「按当前代码与提交记录，未发现」。

---

## 6. 给 Jolu 的行动清单（合并后的优先级）

| 优先 | 项 | 来源 | 量 |
|---|---|---|---|
| **本周** | thinking 无签名块过滤 | `a6a9971` | 半天 |
| **本周** | WorkBuddy deepseek 上限修正 | `0f04dca` | 1 小时 |
| **本周** | OTLP 加 Key 名属性 | `4314f4b` | 1 小时 |
| 下一批 | 更新下载重试 | `a46190e` | 2 小时 |
| 下一批 | 卸载时还原 agent 接线 | `92e8edd` | 半天 |
| 下一批 | 裸名的「自己订阅优先」 | `608c154` | 1 天 |
| 排期 | Key 并发上限（配合 budget） | `e215d33` | 与 budget 一起 |
| 排期 | Smart/首选 的 UI 语义核对 | `4436b7d` | 核对后定 |
| 排期 | ZCode 会话去重核对 | `a39b63e` | 核对后定 |

**注意**：前三个加起来不到一天，但都是「用户可见的正确性问题」，建议随下一批代码一起提交。

---

## 7. 一个观察：magpie 的节奏意味着什么

一天 122 个提交、149 个提交里 15 个是文档站——magpie 的迭代速度下，**任何「静态差距表」的保质期不超过一周**。

对 Jolu 的实际含义：

1. **不要追求「追平」这个目标本身**——它是移动的。按「用户可见的正确性」排优先级（本报告 §6 的前三条），而不是按「magpie 有我们没有」。
2. **每次做差距对比前先 `git fetch`**，别信上次的克隆。
3. **借修复比借功能划算**：149 个提交里最值钱的是 §3 那 8 个「真实用户踩坑后的修复」，每个都是别人替你付过的学费。
