import { CircleCheck, MessageSquare, Monitor, Moon, Palette, Sparkles, Sun, TextCursorInput, type LucideIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import { useSettings } from "@/hooks/useSettings"
import { useSystemReducedMotion } from "@/hooks/useSystemReducedMotion"
import { DENSITY_LABEL, ZOOM_STEPS, appearancePreview, normalizeDensity, normalizeZoom, reduceMotionHint, type Density } from "@/lib/appearance"
import { Segmented, SettingsGroup, SettingsRow, Switch } from "./settingsUi"

type Cfg = ReturnType<typeof useSettings>
type Theme = "light" | "dark" | "system"

/** 缩略界面配色取自画板 88（固定色，不随当前主题变化） */
const THEMES: { value: Theme; label: string; icon: LucideIcon; bg: string; side: string; line: string }[] = [
  { value: "light", label: "浅色", icon: Sun, bg: "#FFFFFF", side: "#F3F4F5", line: "#F3F4F5" },
  { value: "dark", label: "深色", icon: Moon, bg: "#181A19", side: "#2A2D2B", line: "#2A2D2B" },
  { value: "system", label: "跟随系统", icon: Monitor, bg: "#FFFFFF", side: "#2A2D2B", line: "#2A2D2B" },
]

/** 外观（画板 88）：界面主题、界面缩放、列表密度与减少动效；后三项只作用于主窗口 */
export function AppearanceSection({ cfg }: { cfg: Cfg }) {
  const theme = cfg.settings.theme as Theme
  const disabled = !cfg.ready
  const zoom = normalizeZoom(cfg.settings.main_zoom)
  const density = normalizeDensity(cfg.settings.list_density)
  const motionHint = reduceMotionHint(useSystemReducedMotion())

  return (
    <>
      <SettingsGroup icon={Palette} title="界面主题">
        <div role="radiogroup" aria-label="界面主题" className="grid grid-cols-3 gap-3 pb-4">
          {THEMES.map((t) => {
            const active = theme === t.value
            const Icon = t.icon
            return (
              <button
                key={t.value}
                type="button"
                role="radio"
                aria-checked={active}
                disabled={disabled || cfg.isSaving("theme")}
                onClick={() => void cfg.setTheme(t.value)}
                className={cn(
                  "flex min-w-0 flex-col gap-2.5 rounded-[10px] border p-2.5 text-left outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jn-primary disabled:cursor-not-allowed",
                  active ? "border-jn-primary bg-jn-primary-wash" : "border-jn-line bg-jn-surface hover:bg-jn-stage",
                )}
              >
                <span aria-hidden className="flex h-[72px] gap-2 rounded-md p-2" style={{ background: t.bg }}>
                  <span className="w-6 rounded-[3px]" style={{ background: t.side }} />
                  <span className="flex flex-1 flex-col gap-[7px]">
                    <span className="h-4 rounded-[3px] bg-jn-primary" />
                    <span className="h-2 rounded-sm" style={{ background: t.line }} />
                    <span className="h-2 rounded-sm" style={{ background: t.line }} />
                  </span>
                </span>
                <span className="flex items-center gap-[7px]">
                  <Icon aria-hidden className={cn("size-4", active ? "text-jn-primary-ink" : "text-jn-text-3")} />
                  <span className="flex-1 text-xs text-jn-text">{t.label}</span>
                  <CircleCheck aria-hidden className={cn("size-[15px]", active ? "text-jn-primary-ink" : "text-transparent")} />
                </span>
              </button>
            )
          })}
        </div>
      </SettingsGroup>

      <SettingsGroup icon={TextCursorInput} title="阅读与布局">
        <SettingsRow
          title="界面缩放"
          desc="灵动岛保持独立大小"
          control={
            <Segmented<string>
              label="界面缩放"
              value={String(zoom)}
              disabled={disabled || cfg.isSaving("main_zoom")}
              onChange={(value) => void cfg.setAppearance({ main_zoom: Number(value) })}
              items={ZOOM_STEPS.map((step) => ({ value: String(step), label: `${step}%` }))}
            />
          }
        />
        <SettingsRow
          title="列表密度"
          desc="会话、用量记录与 Skills 列表"
          control={
            <Segmented<Density>
              label="列表密度"
              value={density}
              disabled={disabled || cfg.isSaving("list_density")}
              onChange={(value) => void cfg.setAppearance({ list_density: value })}
              items={[{ value: "comfortable", label: DENSITY_LABEL.comfortable }, { value: "compact", label: DENSITY_LABEL.compact }]}
            />
          }
        />
        <div className="mb-4 flex flex-col rounded-[14px] bg-jn-well px-3.5 py-3">
          <div className="flex h-[26px] items-center justify-between text-[11px] text-jn-text-3">
            <span>效果预览</span>
            <span>{appearancePreview(zoom, density)}</span>
          </div>
          {/* 紧凑态行高 40px → 32px（D-22，画板 88 未画紧凑态，待确认） */}
          {[["优化登录流程", "12.8K Token"], ["整理接口文档", "8.4K Token"]].map(([title, tokens]) => (
            <div key={title} className="flex h-10 items-center gap-2 border-t border-jn-line [.density-compact_&]:h-8">
              <MessageSquare aria-hidden className="size-3.5 text-jn-text-3" />
              <span className="flex-1 text-xs text-jn-text">{title}</span>
              <span className="text-[12px] text-jn-primary-ink">{tokens}</span>
            </div>
          ))}
        </div>
      </SettingsGroup>

      <SettingsGroup icon={Sparkles} title="动效偏好">
        <SettingsRow
          title="减少动效"
          /* 只在系统偏好生效时给提示（画板 88 未画此状态，D-23） */
          desc={motionHint || undefined}
          control={<Switch liquid label="减少动效" on={cfg.settings.reduce_motion} disabled={disabled || cfg.isSaving("reduce_motion")} onChange={(on) => void cfg.setAppearance({ reduce_motion: on })} />}
        />
      </SettingsGroup>
    </>
  )
}
