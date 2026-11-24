import { ContentTransition, LoadingSurface } from "@/components/ui/LoadingSurface"
import { useCallback, useEffect, useRef, useState } from "react"
import { cn } from "@/lib/utils"
import { SAVE_STATE_LABEL, SETTINGS_TABS, saveState, tabForSection, type SettingsTab } from "@/lib/settingsPage"
import { useSettings } from "@/hooks/useSettings"
import { Icon, type IconName } from "@/icons"
import { SlideIndicator } from "@/motion"
import { Banner, Button } from "@/ui"
import { GeneralSection } from "@/components/settings/GeneralSection"
import { AppearanceSection } from "@/components/settings/AppearanceSection"
import { NetworkSection } from "@/components/settings/NetworkSection"
import { PrivacySection } from "@/components/settings/PrivacySection"
import { OpsSection } from "@/components/settings/OpsSection"
import { LibrarySharedSection } from "@/components/settings/LibrarySharedSection"
import { WslWiringSection } from "@/components/settings/WslWiringSection"
import { TuneSection } from "@/components/settings/TuneSection"
import { SearchSection } from "@/components/settings/SearchSection"
import { DataSection } from "@/components/settings/DataSection"
import { NotificationSection } from "@/components/settings/NotificationSection"
import { AboutSection } from "@/components/settings/AboutSection"

const ICONS: Record<SettingsTab, IconName> = {
  general: "navSettings",
  appearance: "light",
  network: "external",
  data: "database",
  notifications: "bell",
  about: "info",
}

/**
 * 设置 —— 设计稿 R1-12～17（常规 / 外观 / 网络与代理 / 数据与存储 / 通知 / 关于与更新）。
 *
 * 左侧 200 宽的分区卡（白底、6 个 38 高行、选中项浅绿底 + 主色图标），
 * 右侧内容按组排列（`settingsUi` 的 `SettingsGroup` / `SettingsRow`）。
 * 页面标题由主窗口页头给出，这里不再重复 h1 / h2。
 *
 * `section` 来自托盘 / 菜单的跳转意图（旧分区 id），每次 navigationRequest 变化时重新定位。
 */
export function SettingsPage({ section, navigationRequest, onOpenSkills }: {
  section: string | null
  navigationRequest: number
  onOpenSkills: () => void
}) {
  const cfg = useSettings()
  const [tab, setTab] = useState<SettingsTab>(() => tabForSection(section))
  const [dirtyKeys, setDirtyKeys] = useState<ReadonlySet<string>>(new Set())

  useEffect(() => { if (navigationRequest > 0) setTab(tabForSection(section)) }, [navigationRequest, section])
  // 切换分类后回到页面顶部，避免停在上一分类的滚动位置
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(() => { rootRef.current?.closest<HTMLElement>("[data-panel-scroll]")?.scrollTo({ top: 0 }) }, [tab])

  const setDirty = useCallback((key: string, dirty: boolean) => {
    setDirtyKeys((prev) => {
      if (prev.has(key) === dirty) return prev
      const next = new Set(prev)
      if (dirty) next.add(key)
      else next.delete(key)
      return next
    })
  }, [])

  const state = saveState({ saving: cfg.saving, error: cfg.error, dirty: dirtyKeys.size > 0 })
  const current = SETTINGS_TABS.find((t) => t.id === tab) ?? SETTINGS_TABS[0]

  return (
    <div ref={rootRef} className="flex w-full flex-col gap-3">
      <div className="flex min-h-6 items-center justify-end gap-3">
        <SaveStatus state={state} />
      </div>

      {cfg.error && (
        <Banner tone="danger" action={
          <Button kind="ghost" size="sm" disabled={cfg.loading || cfg.saving} onClick={() => void cfg.reload()}>重新读取</Button>
        }>
          {cfg.error}。已保留原值。
        </Banner>
      )}

      <div className="flex min-h-[560px] items-start gap-4">
        <nav aria-label="设置分类" className="sticky top-0 flex w-[200px] shrink-0 flex-col gap-0.5 rounded-[20px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface">
          {SETTINGS_TABS.map((entry) => {
            const active = entry.id === tab
            return (
              <button
                key={entry.id}
                type="button"
                aria-current={active ? "page" : undefined}
                onClick={() => setTab(entry.id)}
                className={cn(
                  "jn-interactive relative isolate flex h-[38px] items-center gap-2.5 rounded-[13px] px-3 text-left text-[13px]",
                  active ? "font-semibold text-jn-primary-ink" : "text-jn-text hover:bg-jn-well",
                )}
              >
                {active && <SlideIndicator layoutId="jn-settings-nav-active" className="-z-10 bg-jn-primary-wash" />}
                <Icon name={ICONS[entry.id]} size="md" className={active ? "text-jn-primary" : "text-jn-text-2"} />
                {entry.label}
              </button>
            )
          })}
        </nav>

        <section aria-label={current.label} className="flex min-w-0 flex-1 flex-col gap-3.5">
          <ContentTransition identity={`${tab}-${cfg.ready}`} className="flex flex-col gap-3.5">
            {cfg.loading && !cfg.ready ? <LoadingSurface layout="settings" label="正在读取设置" /> : <>
              {tab === "general" && <GeneralSection cfg={cfg} />}
              {tab === "appearance" && <AppearanceSection cfg={cfg} />}
              {tab === "network" && (
                <>
                  <NetworkSection cfg={cfg} setDirty={setDirty} />
                  {/* 待确认：隐私脱敏在 Jolu.pen 模块 G 没有画板，暂挂「网络与代理」页尾 */}
                  <PrivacySection />
                  {/* 待确认：联网搜索与搜索插件在 Jolu.pen 模块 G 没有画板，暂挂「网络与代理」页尾 */}
                  <SearchSection />
                </>
              )}
              {tab === "data" && (
                <>
                  <DataSection cfg={cfg} onOpenSkills={onOpenSkills} />
                  {/* 待确认：下列运维项都没有画板，同挂「数据与存储」页尾 */}
                  <OpsSection />
                  <LibrarySharedSection />
                  <WslWiringSection />
                  <TuneSection />
                </>
              )}
              {tab === "notifications" && <NotificationSection cfg={cfg} setDirty={setDirty} />}
              {tab === "about" && <AboutSection />}
            </>}
          </ContentTransition>
        </section>
      </div>
    </div>
  )
}

/** 保存状态（R1-12 画板未画，沿用旧行为：保存中 / 未保存 / 失败时显示）。 */
function SaveStatus({ state }: { state: ReturnType<typeof saveState> }) {
  if (state === "saved") return null
  const icon: IconName = state === "saving" ? "loading" : state === "dirty" ? "info" : "danger"
  return (
    <span role="status" className={cn("flex items-center gap-1.5 text-[12px]", state === "failed" ? "text-jn-danger" : "text-jn-text-3")}>
      <Icon name={icon} size="sm" className={state === "saving" ? "animate-spin" : undefined} />
      {SAVE_STATE_LABEL[state]}
    </span>
  )
}
