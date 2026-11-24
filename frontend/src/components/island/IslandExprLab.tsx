import { useEffect, useRef, useState } from "react"
import { cn } from "@/lib/utils"
import { Bot } from "@/components/island/Bot"
import { hasRiveBotAsset } from "@/components/island/RiveBotFace"
import { BOT_EXPR_INDEX, BOT_NAMES, BOT_TRIGGERS, SHOT_MS, type BotExpr, type BotShot } from "@/lib/botExpression"

const ONESHOT = new Set<BotExpr>(["happy", "surprise", "greet", "reset"])

const GROUPS: { title: string; exprs: BotExpr[] }[] = [
  { title: "常态", exprs: ["idle", "look", "work", "peek"] },
  { title: "进行中", exprs: ["refresh", "relay", "wait", "drag"] },
  { title: "一次性", exprs: ["happy", "surprise", "greet", "reset"] },
  { title: "告警与离线", exprs: ["worry", "sad", "error", "sleep"] },
]

const previewMs = (expr: BotExpr): number => (ONESHOT.has(expr) ? SHOT_MS[expr as BotShot] : 2600)

export function IslandExprLab({
  onPreview,
}: {
  onPreview?: (expr: BotExpr | null) => void
}) {
  const [active, setActive] = useState<BotExpr | null>(null)
  const timer = useRef(0)
  const riveAvailable = hasRiveBotAsset()

  useEffect(() => () => window.clearTimeout(timer.current), [])

  const play = (expr: BotExpr) => {
    window.clearTimeout(timer.current)
    setActive(expr)
    onPreview?.(expr)
    timer.current = window.setTimeout(() => {
      setActive(null)
      onPreview?.(null)
    }, previewMs(expr))
  }

  return (
    <section className="w-full max-w-[640px]">
      <div className="mb-2 flex items-center justify-between gap-2">
        <h2 className="text-[13px] font-semibold text-jn-text">bot 表情库（16 态）</h2>
        <span className={cn(
          "rounded-full px-2 py-0.5 text-[10px] font-semibold",
          riveAvailable ? "bg-jn-primary-wash text-jn-primary-ink" : "bg-jn-well text-jn-text-3",
        )}>
          {riveAvailable ? "Rive 已就绪" : "SVG 兜底（待交付 bot.riv）"}
        </span>
      </div>

      <div className="mb-3 min-h-[38px] rounded-[10px] border border-jn-line bg-jn-well px-3 py-2 text-[11px] leading-[1.55] text-jn-text-2">
        {active ? (
          <>
            <b className="text-jn-text">{BOT_NAMES[active]}</b>
            <span className="ml-1 font-mono text-[10px] text-jn-text-3">#{BOT_EXPR_INDEX[active]}</span>
            <span className="mx-1.5 text-jn-text-3">·</span>
            {BOT_TRIGGERS[active]}
          </>
        ) : (
          <span className="text-jn-text-3">点任意表情预览；单击岛上 bot 眨眼，连戳 3 次挤眼（告警态屏蔽）。</span>
        )}
      </div>

      <div className="flex flex-col gap-3">
        {GROUPS.map((group) => (
          <div key={group.title}>
            <div className="mb-1.5 text-[10px] font-semibold tracking-wide text-jn-text-3">{group.title}</div>
            <div className="grid grid-cols-4 gap-2">
              {group.exprs.map((expr) => {
                const on = active === expr
                return (
                  <button
                    key={expr}
                    type="button"
                    onClick={() => play(expr)}
                    aria-pressed={on}
                    className={cn(
                      "flex flex-col items-center gap-1.5 rounded-[12px] border px-1 py-2.5 transition-colors",
                      on
                        ? "border-jn-primary bg-jn-primary-wash"
                        : "border-jn-line bg-jn-surface hover:bg-jn-well",
                    )}
                  >
                    <Bot size={44} ringWidth={3} ringGap={2} inset={7} expr={expr} />
                    <span className={cn("text-[11px] font-medium", on ? "text-jn-primary-ink" : "text-jn-text-2")}>
                      {BOT_NAMES[expr]}
                    </span>
                  </button>
                )
              })}
            </div>
          </div>
        ))}
      </div>
    </section>
  )
}
