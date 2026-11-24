import { cn } from "@/lib/utils"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import opencodeLogo from "@/assets/platforms/opencode.svg"
// 多色 / 位图类官方 logo（不适合内嵌单色 path，与 opencode 同走 img）
import vscodeLogo from "@/assets/agents/vscode.svg"
import continueLogo from "@/assets/agents/continue.png"
import pencilLogo from "@/assets/agents/pencil.png"
import piLogo from "@/assets/agents/pi.svg"

/**
 * 工具标识 —— 设计稿 R1-08「标识 · <平台> · lg」（40×40、`jn-well` 底、
 * 圆角 13、图形 20）。
 *
 * 三条来源（鼠鼠 2026-11-13 要求「检测到的工具都要有官方 logo」）：
 * 1. 单色矢量走 `PlatformLogo`（kimi / qwen / dsh(DeepSeek) 复用厂商矢量；
 *    cline / copilot / goose / zed 见 PlatformLogo 的「Agent 工具标识」注释）；
 * 2. 多色 / 位图官方资产走 `<img>`（OpenCode 官网 favicon、VS Code 官方品牌包、
 *    Factory 与 pen.dev 站点图标、Continue 的 GitHub 组织头像、Pi 官方 mark）；
 * 3. 两者都没有（自定义工具、Crush）用首字母小方块。
 *
 * 首字母块用 `jn-well` + `jn-text-2`，不再用旧的紫色 `#615CED`：设计稿的
 * letter 变体就是浅底深字，紫色只在一处出现时会显得像品牌色，实际不是。
 */

const PLATFORM_OF: Readonly<Record<string, string>> = {
  "claude-code": "claude",
  codex: "codex",
  gemini: "gemini",
  zcode: "zcode",
  kimi: "kimi",
  qwen: "qwen",
  cline: "cline",
  copilot: "copilot",
  // DeepSeek Harness 是 DeepSeek 家的编码 harness，用 DeepSeek 品牌图形
  dsh: "deepseek",
  qoder: "qoder",
  "qoder-cn": "qoder-cn",
  workbuddy: "workbuddy",
}

/** 有官方位图 / 多色资产的工具：这些图形自带配色或底色，不适合做单色 path，直接 img（同 opencode） */
const AGENT_IMAGES: Readonly<Record<string, string>> = {
  vscode: vscodeLogo,
  continue: continueLogo,
  pencil: pencilLogo,
  pi: piLogo,
}

export function AgentBadge({ agent, name, size }: { agent: string; name: string; size: 24 | 30 | 34 | 38 | 40 }) {
  const boxes: Record<number, string> = { 24: "size-6 rounded-[7px]", 30: "size-[30px] rounded-[10px]", 34: "size-[34px] rounded-[9px]", 38: "size-[38px] rounded-[10px]", 40: "size-10 rounded-[13px]" }
  const inners: Record<number, number> = { 24: 20, 30: 22, 34: 26, 38: 28, 40: 30 }
  const box = boxes[size] ?? boxes[40]
  const inner = inners[size] ?? 30
  const platform = PLATFORM_OF[agent]
  const image = AGENT_IMAGES[agent]
  if (agent === "opencode") {
    return <img src={opencodeLogo} alt="" aria-hidden width={size} height={size} className={cn("shrink-0", box)} />
  }
  if (image) {
    return <img src={image} alt="" aria-hidden width={size} height={size} className={cn("shrink-0 object-contain", box)} />
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
    <span aria-hidden className={cn("flex shrink-0 items-center justify-center bg-jn-well font-bold text-jn-text-2", box, size === 24 ? "text-[10px]" : "text-[17px]")}>
      {glyph}
    </span>
  )
}
