import { useEffect, useRef, useState } from "react"
import { cn } from "@/lib/utils"
import { Bot } from "@/components/island/Bot"
import { BOT_NAMES, BOT_TRIGGERS, SHOT_MS, type BotExpr, type BotShot } from "@/lib/botExpression"

/**
 * bot 表情库预览（设计画板 A3-5 / A3-6 的浏览器版本）。
 *
 * 只用于在浏览器里逐个查看表情：点任意一格 → 岛上按该表情预览，同时显示它的触发条件。
 * 表情仍然「只由真实状态触发」——这里只是把 expr 直接喂给 Bot，不改变任何状态推导逻辑；
 * Tauri 真机不渲染这个面板（由调用方用 isTauri 兜住）。
 *
 * 一次性表情（happy / surprise / greet / reset）按真实停留时长自动结束，
 * 持续表情（idle / work / worry …）保持到点下一个为止。
 */

const ONESHOT = new Set<BotExpr>(["happy", "surprise", "greet", "reset"])

/** 表情分组：按语义聚成四类，扫视时比一条长列表好找 */
const GROUPS: { title: string; exprs: BotExpr[] }[] = [
  { title: "常态", exprs: ["idle", "look", "work", "peek"] },
  { title: "进行中", exprs: ["refresh", "relay", "wait", "drag"] },
  { title: "一次性", exprs: ["happy", "surprise", "greet", "reset"] },
  { title: "告警与离线", exprs: ["worry", "sad", "error", "sleep"] },
]

/** 每个表情的预览时长：一次性用真实 SHOT_MS，持续的给一个够看清的窗口 */
const previewMs = (expr: BotExpr): number => (ONESHOT.has(expr) ? SHOT_MS[expr as BotShot] : 2600)

export function IslandExprLab({
  onPreview,
}: {
  /** 把当前预览的表情交给岛（null = 结束预览，回到真实状态） */
  onPreview?: (expr: BotExpr | null) => void
}) {
  const [active, setActive] = useState<BotExpr | null>(null)
  const timer = useRef(0)

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
    <section className="mt-6 w-full max-w-[640px]">
      <div className="mb-2 flex items-baseline justify-between gap-3">
        <h2 className="text-[13px] font-semibold text-text-primary">bot 表情库</h2>
        <span className="text-[11px] text-text-tertiary">
          点一格在岛上预览 · {active ? `正在预览「${BOT_NAMES[active]}」` : "当前跟随真实状态"}
        </span>
      </div>

      {/* 触发条件：点选后显示，扫视时也能读到「它为什么会出现」 */}
      <div className="mb-3 min-h-[38px] rounded-[10px] border border-line bg-surface px-3 py-2 text-[11px] leading-[1.55] text-text-secondary">
        {active ? (
          <>
            <b className="text-text-primary">{BOT_NAMES[active]}</b>
            <span className="mx-1.5 text-text-tertiary">·</span>
            {BOT_TRIGGERS[active]}
            <span className="ml-1.5 text-text-tertiary">
              （{ONESHOT.has(active) ? `${previewMs(active)}ms 后自动结束` : "持续到点下一个表情"}）
            </span>
          </>
        ) : (
          <span className="text-text-tertiary">点任意表情，这里显示它的触发条件；岛上的眼睛也会同步切换。</span>
        )}
      </div>

      <div className="flex flex-col gap-3">
        {GROUPS.map((group) => (
          <div key={group.title}>
            <div className="mb-1.5 text-[10px] font-semibold tracking-wide text-text-tertiary">{group.title}</div>
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
                        ? "border-accent-blue bg-accent-blue-soft"
                        : "border-line bg-surface hover:bg-surface-2",
                    )}
                  >
                    {/* 静态小样：与岛上同一个 Bot 组件，但关掉跟随（interactive 默认 false 不绑指针） */}
                    <Bot size={44} ringWidth={3} ringGap={2} inset={7} expr={expr} />
                    <span className={cn("text-[11px] font-medium", on ? "text-accent-blue" : "text-text-secondary")}>
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
