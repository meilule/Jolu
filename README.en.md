<div align="center">

# Jolu

**All your AI coding tools, managed in one place.**

Quota always on your desktop island · Daily rewards in one click · Tasks that wait out quota limits and resume on their own · One local address for every model

[![Release](https://img.shields.io/github/v/release/meilule/Jolu?label=release&logo=github)](https://github.com/meilule/Jolu/releases/latest)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue)

[Download](https://github.com/meilule/Jolu/releases/latest) · [Report an issue](https://github.com/meilule/Jolu/issues)

[简体中文](./README.md) | English

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./docs/assets/readme/island-collapsed-dark.png" />
  <img src="./docs/assets/readme/island-collapsed.png" alt="Jolu desktop island" width="448" />
</picture>

</div>

> Screenshots come from the frontend preview. Accounts, projects and numbers in them are examples, not your real usage. Actual data depends on local records, connection authorization and what each provider returns.

## Highlights

### Island: quota and sessions always on your desktop

No window switching, no web dashboards. The island at the top of your screen shows how much quota the current account has left, when it resets, how many sessions are running and how many tokens this round has used.

| Collapsed | Expanded |
| --- | --- |
| <picture><source media="(prefers-color-scheme: dark)" srcset="./docs/assets/readme/island-collapsed-dark.png" /><img src="./docs/assets/readme/island-collapsed.png" alt="Collapsed island" /></picture> | <picture><source media="(prefers-color-scheme: dark)" srcset="./docs/assets/readme/island-expanded-dark.png" /><img src="./docs/assets/readme/island-expanded.png" alt="Expanded island" /></picture> |
| 5-hour / 7-day quota, reset countdown and session deltas | Subscription quota, running sessions and today's local tokens |

- **Double-click** to expand or collapse; **drag to a screen edge** to dock out of the way.
- Open up to **8 clones**, each showing a different account or tool, so every quota is visible at once.
- The **right-click menu** switches connections, changes position, keeps it on top, toggles do-not-disturb and opens or destroys clones. At least one island always remains.
- The dot on the tray icon mirrors the quota: green normal, yellow near the limit, red exhausted, grey unknown.
- Left-click the tray icon to open the main window. Closing the main window keeps the island, collection and automation scheduling running; quit from the tray menu.

<img src="./docs/assets/readme/context-menu.png" alt="Island right-click menu" width="268" />

### Check-in: claim every daily reward at once

![Check-in](./docs/assets/readme/panel-checkin.png)

- WorkBuddy, Qoder and Trae each have a China and an international card — six in total — showing today's reward, check-in days this week / month and the account quota.
- **Check in all** claims every eligible account in turn; you can also claim one at a time and retry only the failures. The same account is never claimed twice.
- To add an account, pick the provider first and follow its authorization flow; several accounts can be added from the same browser, and **Batch manage** removes them in bulk.
- History and period totals mark whether a record came from the provider or from Jolu. The eye button hides account names for screenshots.
- Manual check-in currently covers WorkBuddy (China), Qoder (China and international) and Trae (China). WorkBuddy AI settles rewards automatically after valid tasks, and Trae international has no official daily check-in yet.

### Out of quota? It waits and resumes on its own

![Automation: waiting for quota and resuming](./docs/assets/readme/automation-wait.png)

Hand tasks to Claude Code or Codex without wasting a run when quota runs out:

- **Quota runs out mid-run**: finished file changes and session progress are kept, the task moves to "Waiting for quota", and the page shows the expected recovery time and the next check.
- **Quota recovers**: Jolu re-checks the 5-hour and weekly quota and, once available, continues in the **same session** instead of starting over. If it cannot resume safely, the task is marked for manual follow-up.
- **Start modes**: run now, scheduled (once / daily / weekly), or "after quota recovers" to line heavy work up for the moment quota resets.
- **Per-account queue**: tasks for the same account run one after another; run records track execution time and waiting time separately.

![Automation task list](./docs/assets/readme/panel-automation.png)

- When creating a task, choose the tool, account, model, project directory and session; a session and its project can be brought over from Sessions in one click.
- The account must be the one currently signed in to the tool — Jolu does not switch logins for you, and file permissions follow the tool's own configuration.
- Jolu must keep running in the background; nothing is scheduled while the computer sleeps, and missed schedules are marked "missed" so you can decide whether to run them.

## Contents

- [Highlights](#highlights)
- [Features](#features)
- [Supported platforms and quotas](#supported-platforms-and-quotas)
- [Download and install](#download-and-install)
- [Getting started](#getting-started)
- [Using each module](#using-each-module)
- [Understanding the numbers](#understanding-the-numbers)
- [Updates and data retention](#updates-and-data-retention)
- [Privacy and local storage](#privacy-and-local-storage)
- [FAQ](#faq)
- [Development](#development)
- [Feedback and license](#feedback-and-license)
- [Community](#community)
- [Star History](#star-history)

## Features

| Module | What it does |
| --- | --- |
| Island | Shows the current connection's quota, reset time and session deltas on your desktop; stays on top, docks to screen edges and supports up to 8 clones |
| Providers | Connect Claude Code, Codex, Gemini, Grok and more; manage accounts and see subscription quotas and API key usage |
| Check-in | Claim official daily rewards for WorkBuddy, Qoder and Trae in batch or one by one, and review check-in history and credit totals |
| Usage | Overview, analysis and call details; filter by time, provider, account and model, and export calls to CSV |
| Sessions | Browse local sessions by project, search titles, session IDs or message text, read messages and continue in the original tool |
| Automation | Let Claude Code / Codex run tasks now, on a schedule or once quota recovers; progress is kept when quota runs out and resumed later |
| Skills | Scan, view, create, import, copy and enable/disable Skills of Claude Code, Codex, Gemini, OpenCode and more, with a backup before every write |
| Agent | Lists the AI tools detected on this computer as cards; 11 of them wire to model routing in one click, with a change preview before and a restore after |
| Model routing | One local address for official subscriptions, official keys and third-party keys, addressed as `source/model`; overview and per-session context observation, route log, routing groups, model pool, external access and middleware |
| Settings | Launch at login, theme, network and privacy masking, data import/export and cleanup, notifications and do-not-disturb; shared instructions and subagents, WSL wiring, cost tuning advice |

![Jolu main window · Usage](./docs/assets/readme/panel-usage.png)

Local statistics and online quotas are independent: **having local sessions does not mean the account quota can be queried, and having a quota does not mean sessions were recorded locally.**

## Supported platforms and quotas

### AI tools

| Platform | Main capabilities | Limits |
| --- | --- | --- |
| Claude Code | Local sessions and messages, tokens and cost estimates, official subscription and supported gateway quotas, continue in tool, automation, Skills | Quotas need valid authorization; API key connections through the local proxy can be attributed per connection |
| Codex | Local sessions and messages, tokens and cost estimates, official subscription and supported gateway quotas, continue in tool, automation, Skills | Usage outside the local proxy is merged per platform; OAuth is not attributed per connection |
| Gemini | Gemini CLI local sessions and token deltas (including thinking and tool tokens), cost estimates at official prices | Online remaining quota not connected; costs are estimates at API list prices |
| Grok | Remaining SuperGrok subscription percentage for the local OAuth login | No local session statistics |
| ZCode | Local usage records, token deltas, session done / failed / running states, BigModel Coding Plan key quotas | Account OAuth quota not connected |
| Qoder | Local records for the China and international editions, recorded credits | Online remaining credits not connected |
| WorkBuddy | Local usage, cache and reported credits | Consumed credits are not remaining credits |
| Trae | Local source detection | No local session statistics |

Custom providers support explicit URLs, protocols and API keys. ZCode and OpenCode connection flows are implemented; live ZCode key and OpenCode login validation remain pending; local OpenCode usage collection has been verified on this machine. Qoder and WorkBuddy keys stay managed in their original clients. Capabilities follow what the app actually shows; never infer login, balance, Skills or automation support from an icon or plan name.

### Quota providers

A platform is the tool you use; a quota provider is who supplies the quota. For example, if you use a Kimi coding plan inside Claude Code, the platform is still Claude Code.

| Provider | Detection | Shown |
| --- | --- | --- |
| Claude / Codex official subscription | Reads the CLI's local login credentials | Subscription windows and reset times returned by the service |
| Zhipu GLM / Z.ai | Detects the China or international edition from the base URL | Personal plan window usage |
| Kimi | Detects the Coding base URL | 5-hour and weekly windows |
| MiniMax | Detects the China or international base URL | 5-hour window and the weekly window when returned |
| ZenMux | Queries supported base URLs | Quota windows and amounts provided by the API |
| OpenCode Go | Detects the Go base URL | Rolling, weekly and monthly windows |
| Grok | Uses the local OAuth login | Remaining SuperGrok subscription percentage |

Volcengine Ark plan quotas require a control-plane AccessKey. Jolu does not store such high-privilege credentials, so they show as "not supported". Actual windows depend on your plan and what the API returns; unknown, unsupported or failed queries are shown as such and do not mean "zero remaining".

## Download and install

Download the package for your platform from [GitHub Releases](https://github.com/meilule/Jolu/releases/latest):

| Platform | File | Notes |
| --- | --- | --- |
| Windows 10 / 11 x64 | `Windows-x86_64-Setup.exe` | Installer, with in-app updates |
| Windows 10 / 11 x64 | `Windows-x86_64-Portable.zip` | Portable, unzip and run; needs WebView2, update by downloading a new zip |
| macOS (Apple silicon / Intel) | `macOS-arm64.dmg` / `macOS-x86_64.dmg` | Not signed with an Apple developer ID; allow it in System Settings on first launch |
| Linux x86_64 | `Linux-x86_64.AppImage` / `Linux-x86_64.deb` | — |

- Windows is the primary platform: features are built and verified there. The macOS and Linux builds compile and install but have not been tested systematically on real machines, and some features that read local tool data may not work.
- Every version has its own release page; download older versions from their own pages.
- The Windows installer creates a Start menu entry and can create a desktop shortcut; uninstalling keeps app data by default.

## Getting started

### 1. Launch

A startup screen appears first and the main window opens as soon as local data is ready, with no artificial delay. The island appears at the top of the screen and can be dragged or docked. Clicking the shortcut again brings back the running instance instead of starting a second one.

### 2. Prepare data sources

Sign in to or configure the AI tools you want to use and run at least one session. Jolu reads existing local records; it never invents usage data.

### 3. Connect tools on the Providers page

![Providers: connect your AI tools](./docs/assets/readme/panel-providers.png)

1. Open **Providers → Provider connections** in the main window.
2. Click connect on a tool's card and follow the prompts to read the local login or enter an API key.
3. Switch to **Accounts** to see connected accounts, subscription quotas and connection status.

### 4. Choose what the island shows

![Island settings](./docs/assets/readme/panel-island.png)

Under **Connections and clones**, pick the connection for the main island, or open clones to show different accounts. The choice is saved locally. If the connection is removed or its credentials expire, choose again — Jolu never silently switches to another account on the same platform.

### 5. Confirm on the Usage page

Make one more request in your AI tool, then open **Usage → Overview** to see today's numbers. If nothing shows up, check the time and provider filters first, then see the [FAQ](#faq).

## Using each module

Island, Check-in and Automation are covered in [Highlights](#highlights).

### Usage: from totals to single calls

- **Overview**: total tokens, calls, estimated cost and input cache hit rate; the trend for today or the chosen range; the top 5 models; usage per provider and the most expensive sessions.
- **Analysis**: a year-long activity calendar, today's active time and active days this week and month.
- **Calls**: model, tokens, latency and status per call; filter by status and keyword, and export to CSV.
- Click a period on the trend or a model to jump to the matching calls; click a session to open it in Sessions.

Latency, time to first token and status codes are shown only when the source provides them; missing fields do not mean a request failed.

### Sessions: read and resume local sessions

![Sessions](./docs/assets/readme/panel-sessions.png)

- Sessions are grouped by project on the left; filter by provider, account, project and time, and sort by most recent or most tokens.
- Search titles, session IDs or message text (press Enter).
- Read messages and tool calls on the right; "Session info" shows the project directory, models, tokens and estimated cost.
- **Continue in tool** checks that Claude Code / Codex is installed and the project directory is reachable, then resumes the session in a new console.
- **View usage** opens this session's calls; **Create automation** carries the session and project into a new task. Session compaction supports incremental summaries, refinement and Markdown handoff. Exploratory short-session evaluation is complete; long-session quality still requires suitable samples.

### Skills: manage every tool's Skills in one place

![Skills](./docs/assets/readme/panel-skills.png)

- Scans the global and project directories of Claude Code and Codex; projects are picked up from session records automatically and can also be added by hand.
- Browse files with Markdown preview; create, edit or import a Skill, or copy it to another tool.
- Enabling or disabling affects only that copy, never other copies with the same name; every write, overwrite or removal is backed up first.
- Gemini, OpenCode, Qoder, Trae and ZCode directories are supported. WorkBuddy uses `.codebuddy/skills` in read-only mode; custom scan directories are also read-only. HTTPS Git import and draft-to-disk diffs are supported. Gemini/OpenCode toggles are name-based, unlike Claude/Codex per-copy toggles.

### Agent: wire a tool to model routing in one click

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./docs/assets/readme/panel-agents-dark.png" />
  <img src="./docs/assets/readme/panel-agents.png" alt="Agent page: AI tools detected on this computer as cards, with a switch to wire or unwire each" />
</picture>

- **Agent** in the sidebar lists the AI tools found on this computer as cards, counted as wired / available, with search; the switch on each card wires or unwires it.
- **11 tools wire in one click**: Claude Code, Codex, Gemini CLI, OpenCode, ZCode, Kimi CLI, DeepSeek Harness, Pencil, Qoder, Qoder CN and Crush. You can preview the config changes before wiring; Jolu writes only the fields it manages and restores the pre-wiring snapshot when you unwire. Values you changed in the tool yourself are left alone.
- **Detail page**: for Claude Code, pick a model per tier (opus / sonnet / haiku …) and a subagent model; other tools get a default model. The "Changes" tab lists exactly which lines Jolu changed.
- Tools that are detected but have no dedicated wiring yet (such as Cline or GitHub Copilot CLI) show "Not supported yet"; you can also **Add a tool** yourself by giving its JSON / TOML / env config path and key names.
- **Tools inside WSL**: under **Settings → Data and storage** you can wire Claude Code, Codex and OpenCode inside a WSL distro. Mirrored networking uses 127.0.0.1; NAT networking needs LAN sharing turned on first under "External access", and Jolu issues that tool its own key.
- Pencil, Qoder, Crush and WSL wiring have not been verified with the real tools yet.

### Model routing: one local address for every account

**Model routing** in the sidebar puts official subscriptions, official keys and third-party keys behind one local address; every tool asks for models as `<source>/<model>`, so you don't enter an address and key in each tool. Nothing is forwarded by default: Jolu listens on a loopback port only after you start relaying, and tools go back to direct connections when you stop.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./docs/assets/readme/gateway-overview-dark.png" />
  <img src="./docs/assets/readme/gateway-overview.png" alt="Model routing overview: today's requests, success rate, reroutes and failures, a 12-hour trend, sessions needing attention and context usage of active sessions" />
</picture>

- **Overview**: today's requests, success rate, in flight, reroutes, failures and response latency; the request trend over the last 12 hours; "Needs attention" flags sessions close to their context window; the active-session table shows how much context each session's latest request used and how much came from the cache.
- **Session observation**: click "View context" on an active session to see what one request's context is made of — system prompt, tool definitions, Skills, memory, files, tool results and conversation — scaled to the upstream's real usage. Tool results are shown only as redacted summaries; their content is not stored.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./docs/assets/readme/gateway-session-dark.png" />
  <img src="./docs/assets/readme/gateway-session.png" alt="Session observation: request path, context usage, prompt cache and the context broken down by category" />
</picture>

- **Route log**: one row per forwarded request — time, tool, requested model, source it was routed to, duration and result.
- **Routing groups**: `group/<name>` bundles several `source/model` members, with order, rotate, by usage, smart, pace and by intent (a small model judges how hard each question is and sends it to a strong or light member); members that used up their quota move to the back, and single members can be paused.
- **Model pool**: which sources and models are served; one platform can have several accounts, de-duplicated by platform / region / account identity.
- **External access**: LAN sharing is off by default; issue named client keys to other devices or tools with expiry, model allow-list, read-only, and daily request / token limits.
- **Middleware**: model-name mapping, parameter overrides, extra system prompt, blocked words, think-tag handling, or your own JS hook; each is dry-run before saving.
- **Protocol translation**: third-party models that only offer a Chat API also work for Codex (Responses) and Claude Code (Anthropic Messages). When Codex keeps its ChatGPT sign-in and uses a third-party model, Jolu performs its remote compaction.
- **Failing sources and safety**: paused, signed-out or exhausted sources stay listed with the reason, and requests get 401 / 429 / 503 accordingly; Jolu listens on loopback only (unless LAN sharing is on) and checks `Host` / `Origin`; after repeated failures it switches tools back to direct connections, or you can do so by hand; unwiring never deletes accounts, keys or route history.
- Grok, OpenCode, ZCode, Qoder, Trae and both WorkBuddy editions have subscription adapters; **Qoder and Trae have not been verified with real accounts yet**.

### Settings

![Settings](./docs/assets/readme/panel-settings.png)

General (launch at login, silent start, auto refresh), Appearance (light / dark / system), Network and proxy (connection mode, privacy masking, web search plugin), Data and storage (import/export, retention, cleanup, open data folder), Notifications (do-not-disturb, balance alerts) and About.

At the bottom of "Data and storage" are a few features without a final design yet:

- **Operations**: keep the system awake, a daily warm-up, OTLP usage export, and each tool's version.
- **Shared instructions and subagents**: one block of instructions written into the user-level instruction files of Claude Code, Codex, Gemini, OpenCode, Qwen and Droid (inside a Jolu-marked block; everything outside it stays yours); subagents written as files in each tool's own format. Files you changed are never overwritten.
- **Tools inside WSL**: see the Agent section above.
- **Cost tuning advice**: replays this computer's recent calls one by one and suggests when Claude Code / Codex should compact and whether the prompt cache should last 5 minutes or an hour to spend the fewest tokens. Advice only; nothing is changed.

"About" also lists how each platform capability has been verified and on what evidence.

## Understanding the numbers

| Metric | Meaning |
| --- | --- |
| Local tokens | From local session records, aggregated per platform; not a per-account or per-key bill |
| Fresh input | Input excluding recognizable cache reads, to see new input consumption |
| Cache read / write | Reuse of existing cache and cache creation; fields vary by source |
| Island delta | Fresh token deltas from supported sources; a different measure from totals that include cache reads |
| Subscription quota | Account or plan windows returned by the provider, possibly including usage from other devices |
| Estimated cost | Estimated from recognized models and price lists; not a final bill and not a subscription charge |
| Credits | Consumed credits reported by the source; not remaining credits unless stated |
| `—` / not supported / query failed | Read each as its own state; never treat them as zero |

Local tokens, island deltas and official quotas therefore do not have to match. Cross-device use, missing history, cache accounting and provider refresh delays can all cause differences.

## Updates and data retention

Jolu checks for updates at launch and then every 30 minutes. A new version is downloaded and signature-checked in the background; a green version button then appears in the page header, and clicking it installs the update and restarts the app. You can also download the new installer from [GitHub Releases](https://github.com/meilule/Jolu/releases) and install over the existing version.

Settings and statistics live in the app data folder, which an in-place install keeps using. Island connections, display preferences, the main window position, automation tasks and run records all persist.

**Statistics export is not a full backup:**

- **Import / export** in Settings covers statistics records and session titles, with de-duplication on import; connections and credentials are not included.
- For a full backup, open the data folder from Settings, quit the app from the tray, then copy the whole folder.
- The full folder may contain credentials; keep it somewhere trusted. After restoring on another device you may still need to sign in to the original tools again.

See [versioning rules](./docs/release-versioning.md) for how releases are numbered.

## Privacy and local storage

```text
Local session files ──incremental read, de-dup──→ Local SQLite ──→ Main window / island
Provider APIs       ──query quota with the current connection──→ Quota windows
Automation tasks    ──run claude / codex locally──────────────→ Run records
```

- Statistics are stored in the local SQLite database `usage.db`; preferences in `settings.json`. The default Windows data folder is `%APPDATA%\dev.ningz.jolu`, which you can open from Settings.
- Quota checks, credential refresh and model queries contact the corresponding services and send authorization as required by their APIs; "local storage" does not mean "never online".
- Automation runs the original tools on your machine and never goes through a Jolu server; writing Skills changes the tool's folders and configuration, with a backup in the app data folder first.
- If you enable the local proxy or model routing, model requests are forwarded through this computer to the configured upstream, and the wired tools' configuration changes; unwiring restores it from a snapshot.
- With **privacy masking** on, secrets and other private values in requests sent through model routing are replaced with placeholders and restored in the reply; the mapping lives in memory only.
- API keys are handled only by the backend and shown masked in lists; never share the full data folder or credential files.

## FAQ

<details>
<summary>No statistics after installing?</summary>

Make sure the AI tool has produced local sessions and that the Usage page uses the right time range and provider. An online account without local records will not backfill cross-device history. Platforms without local session support, such as Trae, do not show full token statistics.

</details>

<details>
<summary>Why are there tokens but no subscription quota?</summary>

Local statistics and online quotas are independent. Check whether the platform supports quota queries, whether the authorization is valid and whether the base URL is recognized. A working API key does not grant subscription quota access.

</details>

<details>
<summary>Why didn't my automation task run?</summary>

Check the task status: if the account is not the one signed in to the tool, the run stops at "needs attention"; without quota it waits for recovery; nothing is scheduled while Jolu is fully quit or the computer sleeps. Also make sure the tool is installed and the project directory is reachable.

</details>

<details>
<summary>"Continue in tool" doesn't open?</summary>

Only Claude Code and Codex are supported. The dialog checks that the tool is installed and the project directory still exists; if either check fails, copy the session ID and resume it manually in the tool.

</details>

<details>
<summary>Can't find the island or the main window?</summary>

Open the main window from the system tray and make sure the island is shown on the Island page. After changing monitors or resolution, use "Reset window position". On Windows the tray icon may be hidden in the overflow area.

</details>

<details>
<summary>Why does the browser preview show different data?</summary>

The browser preview uses example data and never reads your real credentials or statistics. Run the desktop app to see real usage.

</details>

## Development

Stack: **Tauri 2 · Rust · React 19 · TypeScript · Vite · Tailwind CSS 4 · SQLite**. The only UI source of truth is [`docs/ui/Jolu.pen`](./docs/ui/Jolu.pen); see [AGENTS.md](./AGENTS.md) for conventions.

From the repository root:

```bash
pnpm install
pnpm --dir frontend install

# Browser preview with example data
pnpm dev

# Desktop development with the Rust backend and local data
pnpm tauri dev

# Frontend type check and unit tests
pnpm --dir frontend typecheck
node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs

# Backend tests
cargo test --manifest-path backend/Cargo.toml

# Build a local Windows installer (NSIS, without signed updater artifacts)
pnpm tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

The installer is written to `backend/target/release/bundle/nsis/`. The first Rust build takes a while; if the dev port conflicts, check that the Vite address matches Tauri's `devUrl`.

### Command line: `jolu`

The CLI is not shipped in the installer; build it from source with `cargo build --release --manifest-path backend/Cargo.toml --bin jolu_cli`. It uses the same data folder as the desktop app (override with `JOLU_DATA_DIR`); `jolu help` lists everything. A few common ones:

```bash
jolu status                       # model routing overview and each tool's wiring
jolu catalog                      # model catalog
jolu tune --days 30               # cost tuning advice (read-only)
jolu claude run --model kimi/k2   # run Claude Code through model routing once, without touching ~/.claude/settings.json
echo sk-... | jolu provider add custom my-gateway --base-url https://… --protocol openai --model …
jolu group add main rotate kimi/k2 zcode/glm-5   # create a routing group, effective immediately
jolu settings set dnd on          # change an allow-listed setting; the desktop app picks it up within 2 seconds
jolu use daily                    # switch Profile (carried out by the running desktop app)
```

Keys are read only from standard input, never from arguments; `save` / `use` / `serve` need the desktop app running.

Session handoff supports incremental summaries, draft refinement and Markdown preview/export. Exploratory prompt evaluation covered five short sessions; it does not establish long-session quality. OpenCode JSON/SQLite V1/V2 usage parsing keeps unknown fields unknown and adds visible output and separate reasoning while preferring the source total. See [unverified integrations](docs/todo/2026-10-22-unverified-integrations.md). Update checks are on by default; set both `JOLU_UPDATES_ENABLED=0` and `VITE_JOLU_UPDATES_ENABLED=0` at build time to turn them off.

### Project structure

```text
jolu/
├─ frontend/          Multi-window React frontend and browser example data
│  └─ src/
│     ├─ views/       Main window modules, island, startup screen, tray entries
│     ├─ components/  automation, providers, sessions, settings, skills, usage and shared components
│     ├─ hooks/       Settings, connection, quota and query hooks
│     └─ lib/         API bridge, platform catalog and pure logic
├─ backend/           Rust / Tauri backend
│  └─ src/            Collection, database, connections, quotas, sessions, automation, Skills, windows and tray
├─ docs/              Design file, brand assets, screenshots and versioning rules
└─ scripts/           Icon generation, release versioning and acceptance scripts
```

The collector reads local records incrementally and de-duplicates them into SQLite; the quota module queries the service behind the current connection; the automation scheduler runs local tools in the background, queued per account. Platform capabilities are defined in the [platform catalog](./frontend/src/lib/platformCatalog.json).

## Feedback and license

- [Open an issue](https://github.com/meilule/Jolu/issues): include the version, OS, steps to reproduce, expected and actual results; hide credentials in screenshots and logs.
- [Releases](https://github.com/meilule/Jolu/releases) · [Roadmap](./docs/plans/2026-10-21-jolu-development-roadmap.md)
- The original repository description says MIT; a separate LICENSE file has not been added yet. Improvements with a clear problem description and verification notes are welcome.

## Community

[linux.do](https://linux.do/) - A thriving developer community.

## Star History

<a href="https://www.star-history.com/?repos=meilule%2FJolu&amp;type=date&amp;legend=top-left">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=meilule/Jolu&amp;type=date&amp;legend=top-left&amp;theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=meilule/Jolu&amp;type=date&amp;legend=top-left" />
    <img alt="Jolu Star History" src="https://api.star-history.com/chart?repos=meilule/Jolu&amp;type=date&amp;legend=top-left" width="100%" />
  </picture>
</a>
