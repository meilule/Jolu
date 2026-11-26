import { cn } from "@/lib/utils"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import claudeCodeLogo from "@/assets/platforms/claude-code.svg"
import codexLogo from "@/assets/platforms/codex-agent.svg"
import geminiCliLogo from "@/assets/platforms/gemini-cli.svg"
// 多色 / 位图类官方 logo（不适合内嵌单色 path）
import vscodeLogo from "@/assets/agents/vscode.svg"
import continueLogo from "@/assets/agents/continue.png"
import pencilLogo from "@/assets/agents/pencil.png"
import piLogo from "@/assets/agents/pi.svg"

/**
 * 工具标识 —— 设计稿 R1-08「标识 · <平台> · lg」（40×40、`jn-well` 底、
 * 圆角 13、图形 20）。
 *
 * 2026-11-25 对齐各工具当前官方品牌：
 * - Claude Code / Codex / Gemini CLI / OpenCode 用产品自身图标，不再拿母公司标代替；
 * - Antigravity / Kimi / Qwen / DeepSeek Harness / Qoder / WorkBuddy / ZCode 与供应商页共用官方 SVG；
 * - VS Code / Continue / Pencil / Pi 使用各自的官方资产；
 * - 只有自定义工具或没有正式图形的工具才用首字母，不以仿制图形冒充品牌。
 */

const PLATFORM_OF: Readonly<Record<string, string>> = {
  "claude-code": "claude",
  codex: "codex",
  gemini: "gemini",
  antigravity: "antigravity",
  opencode: "opencode",
  zcode: "zcode",
  kimi: "kimi",
  qwen: "qwen",
  cline: "cline",
  copilot: "copilot",
  // DeepSeek Harness 是 DeepSeek 家的编码 harness，用 DeepSeek 品牌图形。
  dsh: "deepseek",
  qoder: "qoder",
  "qoder-cn": "qoder-cn",
  workbuddy: "workbuddy",
}

/** 工具自身有官方图形时优先于平台通用标识。 */
const AGENT_IMAGES: Readonly<Record<string, string>> = {
  "claude-code": claudeCodeLogo,
  codex: codexLogo,
  gemini: geminiCliLogo,
  vscode: vscodeLogo,
  continue: continueLogo,
  pencil: pencilLogo,
  pi: piLogo,
}

export function AgentBadge({ agent, name, size }: { agent: string; name: string; size: 24 | 30 | 34 | 38 | 40 }) {
  const boxes: Record<number, string> = {
    24: "size-6 rounded-[7px]",
    30: "size-[30px] rounded-[10px]",
    34: "size-[34px] rounded-[9px]",
    38: "size-[38px] rounded-[10px]",
    40: "size-10 rounded-[13px]",
  }
  const inners: Record<number, number> = { 24: 20, 30: 22, 34: 26, 38: 28, 40: 30 }
  const box = boxes[size] ?? boxes[40]
  const inner = inners[size] ?? 30
  const platform = PLATFORM_OF[agent]
  const image = AGENT_IMAGES[agent]
  if (image) {
    return (
      <img
        src={image}
        alt=""
        aria-hidden
        draggable={false}
        width={size}
        height={size}
        data-agent-logo={agent}
        className={cn("shrink-0 object-contain", box)}
      />
    )
  }
  if (platform) {
    return (
      <span aria-hidden className={cn("flex shrink-0 items-center justify-center bg-jn-well", box)}>
        <PlatformLogo platform={platform} size={inner} />
      </span>
    )
  }
  const glyph = (name.trim()[0] ?? "?").toUpperCase()
  return (
    <span
      aria-hidden
      data-agent-logo="fallback"
      className={cn("flex shrink-0 items-center justify-center bg-jn-well font-bold text-jn-text-2", box, size === 24 ? "text-[10px]" : "text-[17px]")}
    >
      {glyph}
    </span>
  )
}
