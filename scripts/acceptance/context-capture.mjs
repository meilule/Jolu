// V10 上下文观察：抓四个 CLI 的真实请求体，作为解析器测试样本的原料。
// 依次在独立主目录 output/acceptance/context-capture/home/<工具> 里运行 Claude Code / Codex / OpenCode / Gemini CLI，
// 全部指向本机回环的 context-capture-upstream.py（它会先回一次读文件的工具调用，再回文字），不读写真实配置。
// 演示项目在 output/acceptance/context-capture/project，带 CLAUDE.md / AGENTS.md / GEMINI.md、一个 Skill 和 README.md。
// 结果：output/acceptance/context-capture/raw/<工具>/<序号>-<协议>.json（真实系统提示词，未脱敏，output/ 不进 Git）。
// 用法：node scripts/acceptance/context-capture.mjs [claude codex opencode gemini]
import { execFileSync, spawn } from "node:child_process"
import { cpSync, existsSync, mkdirSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const ROOT = join(REPO, "output/acceptance/context-capture")
const PROJECT = join(ROOT, "project")
const RAW = join(ROOT, "raw")
const PORT = 18767
const BASE = `http://127.0.0.1:${PORT}`
const KEY = "capture-dummy-key"
const ASK = "读一下 README.md，告诉我这个项目是做什么的"
const only = process.argv.slice(2)

const write = (path, text) => { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, text) }
const skill = (name) => `---\nname: ${name}\ndescription: Demo skill used by Jolu context capture. Use when asked to format a changelog.\n---\n\n# ${name}\n\nFormat changelog entries as a bullet list grouped by type.\n`

// 演示项目：每次重建，内容是自编的，不含真实信息
rmSync(PROJECT, { recursive: true, force: true })
write(join(PROJECT, "README.md"), "# demo-shop\n\nA tiny demo web shop used to exercise coding agents. It lists products and has a cart.\n")
write(join(PROJECT, "CLAUDE.md"), "# Project rules\n\n- Use TypeScript strict mode.\n- Keep components small.\n")
write(join(PROJECT, "AGENTS.md"), "# Project rules\n\n- Use TypeScript strict mode.\n- Run tests before finishing.\n")
write(join(PROJECT, "GEMINI.md"), "# Project rules\n\n- Use TypeScript strict mode.\n- Prefer small pure functions.\n")
write(join(PROJECT, "src/cart.ts"), "export const total = (items: { price: number }[]) => items.reduce((s, i) => s + i.price, 0)\n")
// 演示项目自成一个 Git 仓库，免得 CLI 向上找到本仓库的 AGENTS.md 与 git 状态
execFileSync("git", ["init", "-q"], { cwd: PROJECT })
for (const dir of [".claude/skills", ".agents/skills", ".codex/skills", ".opencode/skill", ".gemini/skills"]) write(join(PROJECT, dir, "changelog-format/SKILL.md"), skill("changelog-format"))

const toolVar = /^(ANTHROPIC_|CLAUDE_|OPENAI_|CODEX_|GEMINI_|GOOGLE_|OPENCODE_|AZURE_OPENAI)/i
const baseEnv = { ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !toolVar.test(k))), NO_COLOR: "1", CI: "1" }
const home = (name) => { const dir = join(ROOT, "home", name); mkdirSync(dir, { recursive: true }); return dir }
const isolatedEnv = (dir, extra) => ({ ...baseEnv, USERPROFILE: dir, HOME: dir, APPDATA: join(dir, "AppData/Roaming"), LOCALAPPDATA: join(dir, "AppData/Local"), XDG_CONFIG_HOME: join(dir, ".config"), XDG_DATA_HOME: join(dir, ".local/share"), XDG_CACHE_HOME: join(dir, ".cache"), XDG_STATE_HOME: join(dir, ".local/state"), ...extra })
const userRules = "# My global rules\n\n- Answer in Chinese.\n- Conclusion first.\n"

const claudeHome = home("claude")
write(join(claudeHome, ".claude/CLAUDE.md"), userRules)
write(join(claudeHome, ".claude/skills/commit-helper/SKILL.md"), skill("commit-helper"))

const codexHome = home("codex")
write(join(codexHome, ".codex/config.toml"), [
  'model = "test-model"', 'model_provider = "capture"', 'approval_policy = "never"', 'sandbox_mode = "danger-full-access"', "",
  "[model_providers.capture]", 'name = "capture"', `base_url = "${BASE}/v1"`, 'env_key = "CAPTURE_KEY"', 'wire_api = "responses"', "",
].join("\n"))
write(join(codexHome, ".codex/AGENTS.md"), userRules)
write(join(codexHome, ".codex/skills/commit-helper/SKILL.md"), skill("commit-helper"))

const opencodeHome = home("opencode")
// 复用网关验收已下载的 provider 包，省掉首次下载
const oldCache = join(REPO, "output/acceptance/cli-home/opencode/.cache")
if (existsSync(oldCache) && !existsSync(join(opencodeHome, ".cache"))) cpSync(oldCache, join(opencodeHome, ".cache"), { recursive: true })
const opencodeConfig = join(opencodeHome, "opencode.json")
write(opencodeConfig, JSON.stringify({
  $schema: "https://opencode.ai/config.json", autoupdate: false, share: "disabled",
  provider: { capture: { npm: "@ai-sdk/openai-compatible", name: "Capture", options: { baseURL: `${BASE}/v1`, apiKey: KEY }, models: { "test-model": { name: "test-model" } } } },
  model: "capture/test-model", small_model: "capture/test-model",
}, null, 2))
write(join(opencodeHome, ".config/opencode/AGENTS.md"), userRules)

const geminiHome = home("gemini")
write(join(geminiHome, ".gemini/settings.json"), JSON.stringify({ security: { auth: { selectedType: "gemini-api-key" } }, general: { disableAutoUpdate: true }, privacy: { usageStatisticsEnabled: false } }))
write(join(geminiHome, ".gemini/GEMINI.md"), userRules)

const runs = {
  claude: ["claude", ["-p", ASK, "--model", "claude-sonnet-4-5", "--dangerously-skip-permissions"], isolatedEnv(claudeHome, {
    ANTHROPIC_BASE_URL: BASE, ANTHROPIC_API_KEY: KEY, ANTHROPIC_SMALL_FAST_MODEL: "claude-haiku-4-5", ANTHROPIC_DEFAULT_HAIKU_MODEL: "claude-haiku-4-5",
    CLAUDE_CONFIG_DIR: join(claudeHome, ".claude"), CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1", DISABLE_AUTOUPDATER: "1",
  })],
  codex: ["codex", ["exec", "--skip-git-repo-check", ASK], isolatedEnv(codexHome, { CODEX_HOME: join(codexHome, ".codex"), CAPTURE_KEY: KEY })],
  opencode: ["opencode", ["run", ASK, "-m", "capture/test-model"], isolatedEnv(opencodeHome, { OPENCODE_CONFIG: opencodeConfig, OPENCODE_DISABLE_AUTOUPDATE: "1" })],
  gemini: ["gemini", ["-p", ASK, "-m", "gemini-2.5-pro", "--yolo"], isolatedEnv(geminiHome, { GEMINI_API_KEY: KEY, GOOGLE_GEMINI_BASE_URL: BASE, GEMINI_CLI_TRUST_WORKSPACE: "true" })],
}

const run = (cmd, args, env) => new Promise((done) => {
  // Windows 上 npm 全局命令是 .cmd，经 shell 启动；参数自行加引号
  const quoted = args.map((a) => (/[\s"]/.test(a) ? `"${a.replaceAll('"', '\\"')}"` : a))
  const child = spawn([cmd, ...quoted].join(" "), { env, cwd: PROJECT, shell: true, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] })
  let output = ""
  child.stdout.on("data", (d) => { output += d })
  child.stderr.on("data", (d) => { output += d })
  const timer = setTimeout(() => child.kill(), 300000)
  child.on("close", (code) => { clearTimeout(timer); done({ code, output: output.slice(-1500) }) })
})

mkdirSync(RAW, { recursive: true })
for (const f of readdirSync(RAW).filter((f) => f.endsWith(".json"))) rmSync(join(RAW, f))
const upstream = spawn("python", ["-I", join(REPO, "scripts/acceptance/context-capture-upstream.py"), String(PORT)], { env: { ...baseEnv, JOLU_CAPTURE_CWD: PROJECT, PYTHONIOENCODING: "utf-8" }, stdio: ["ignore", "pipe", "pipe"], windowsHide: true })
await new Promise((r) => upstream.stdout.once("data", r))
try {
  for (const [name, [cmd, args, env]] of Object.entries(runs)) {
    if (only.length && !only.includes(name)) continue
    rmSync(join(RAW, name), { recursive: true, force: true })
    const result = await run(cmd, args, env)
    // 上游把请求体平铺在 raw/ 下；按工具归档（工具串行运行，所以平铺的文件都属于刚跑完的这个工具）
    const dest = join(RAW, name)
    mkdirSync(dest, { recursive: true })
    const files = existsSync(RAW) ? readdirSync(RAW).filter((f) => f.endsWith(".json")) : []
    for (const f of files) renameSync(join(RAW, f), join(dest, f))
    console.log(`== ${name}: exit ${result.code}, ${files.length} 个请求体 ${files.join(" ")}\n${result.output.trim()}\n`)
  }
} finally {
  upstream.kill()
}
