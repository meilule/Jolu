import type { BotProp } from "@/lib/botExpression"

const CONFETTI_SPECS = [
  { x: -12, y: -10, c: "#3DD68C", r: -24 },
  { x: 12, y: -12, c: "#F5B83D", r: 30 },
  { x: -16, y: 2, c: "#5AC8FA", r: 45 },
  { x: 16, y: 1, c: "#FF6B6F", r: -35 },
  { x: -8, y: -16, c: "#A882FF", r: 12 },
  { x: 9, y: -15, c: "#3DD68C", r: -18 },
  { x: -14, y: 10, c: "#F5B83D", r: 60 },
  { x: 14, y: 10, c: "#5AC8FA", r: -52 },
] as const

/**
 * R2-I-05 bot 8 种轻量 SVG 道具覆盖层（SVG 兜底模式；Rive 交付 bot.riv 后由状态机 `prop` 接管）。
 * 严格遵循无嘴巴、克制硬件质感规范；减少动效时保留静态小道具，停掉彩带与飘动动画。
 */
export function BotPropsOverlay({
  prop,
  size,
  reduced = false,
}: {
  prop: BotProp
  size: number
  reduced?: boolean
}) {
  if (prop === "none") return null
  const badge = Math.max(14, Math.min(28, Math.round(size * 0.44)))

  if (prop === "confetti") {
    return (
      <span
        aria-hidden
        data-bot-prop="confetti"
        className="cap-prop-confetti"
        style={{ width: size, height: size }}
      >
        {CONFETTI_SPECS.map((p, idx) => (
          <span
            key={idx}
            className={reduced ? "cap-confetti-bit cap-confetti-static" : "cap-confetti-bit"}
            style={{
              background: p.c,
              ["--cx" as string]: `${Math.round((p.x / 16) * (size * 0.55))}px`,
              ["--cy" as string]: `${Math.round((p.y / 16) * (size * 0.55))}px`,
              ["--cr" as string]: `${p.r}deg`,
              animationDelay: `${idx * 35}ms`,
            }}
          />
        ))}
      </span>
    )
  }

  if (prop === "charge") {
    return (
      <span
        aria-hidden
        data-bot-prop="charge"
        className={reduced ? "cap-prop-charge cap-prop-static" : "cap-prop-charge"}
        style={{ width: size + 4, height: size + 4 }}
      />
    )
  }

  return (
    <span
      aria-hidden
      data-bot-prop={prop}
      className={reduced ? "cap-prop-badge cap-prop-static" : "cap-prop-badge"}
      style={{ width: badge, height: badge }}
    >
      <PropGlyph prop={prop} />
    </span>
  )
}

function PropGlyph({ prop }: { prop: Exclude<BotProp, "none" | "confetti" | "charge"> }) {
  switch (prop) {
    case "gift":
      return (
        <svg viewBox="0 0 20 20" className="size-full">
          <rect x="3" y="8" width="14" height="9" rx="2" fill="#FF6B6F" />
          <rect x="2" y="6" width="16" height="3.2" rx="1.2" fill="#FF8588" />
          <rect x="9" y="6" width="2" height="11" fill="#F5B83D" />
          <path d="M10 6 C8 2.5 5 3.5 6.5 6 M10 6 C12 2.5 15 3.5 13.5 6" fill="none" stroke="#F5B83D" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
      )
    case "piggy":
      return (
        <svg viewBox="0 0 20 20" className="size-full">
          <circle cx="10" cy="10" r="7" fill="#F5B83D" />
          <circle cx="10" cy="10" r="5" fill="none" stroke="#FFF2C2" strokeWidth="1.2" />
          <text x="10" y="13.2" textAnchor="middle" fontSize="9" fontWeight="700" fill="#1A1508">$</text>
        </svg>
      )
    case "utensils":
      return (
        <svg viewBox="0 0 20 20" className="size-full">
          <circle cx="10" cy="10" r="8" fill="#1F2522" stroke="#F5B83D" strokeWidth="1.4" />
          <path d="M7 5v5m-1.5-5v3a1.5 1.5 0 0 0 3 0V5M7 10v5M13.5 5v10m0-10c1.5 1 1.5 4 0 5" fill="none" stroke="#F5B83D" strokeWidth="1.3" strokeLinecap="round" />
        </svg>
      )
    case "flag":
      return (
        <svg viewBox="0 0 20 20" className="size-full">
          <path d="M5 16V4" stroke="#E5EBE8" strokeWidth="1.8" strokeLinecap="round" />
          <path d="M5.5 4.5h9.5l-2 3.2 2 3.3H5.5Z" fill="#3DD68C" />
        </svg>
      )
    case "coffee":
      return (
        <svg viewBox="0 0 20 20" className="size-full">
          <path d="M7 4.5c0-1 1-1.5 1-2.5M11 4.5c0-1 1-1.5 1-2.5" fill="none" stroke="#9CA6A1" strokeWidth="1.3" strokeLinecap="round" />
          <path d="M4.5 7h9v5.5a3.5 3.5 0 0 1-3.5 3.5H8a3.5 3.5 0 0 1-3.5-3.5Z" fill="#D49A6A" />
          <path d="M13.5 8.5h1.8a1.7 1.7 0 0 1 0 3.4h-1.8" fill="none" stroke="#D49A6A" strokeWidth="1.5" />
        </svg>
      )
    case "medal":
      return (
        <svg viewBox="0 0 20 20" className="size-full">
          <path d="M7 3l3 5 3-5" fill="none" stroke="#5AC8FA" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
          <circle cx="10" cy="12.5" r="4.5" fill="#3DD68C" />
          <path d="M8.5 12.5l1.2 1.2 2.2-2.4" fill="none" stroke="#0B0E0D" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      )
  }
}
