/**
 * 灵动岛页 —— 设计稿 `docs/ui/jolu-new.pen` R1-26（灵动岛）。
 *
 * 左右双栏：
 * - 左栏：显示灵动岛总开关卡、经典 / 胶囊样式切换卡、连接与分身表、联动设置（待确认）；
 * - 右栏：窗口行为、显示外观、刷新与位置三个设置组卡。
 *
 * 设置即时生效，真值在后端，与托盘、灵动岛右键菜单共用同一份。
 */
import { useEffect, useMemo, useState, type ReactNode } from "react"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { api, DEFAULT_ISLAND_LINKAGE, isTauri, type AppSettings, type DockEdgeDto, type IslandStyle } from "@/lib/api"
import { cn } from "@/lib/utils"
import { PLATFORMS } from "@/lib/platforms"
import { useToast } from "@/components/ui/Toast"
import { useSettings } from "@/hooks/useSettings"
import { useConnections } from "@/hooks/useConnections"
import { ContentTransition, LoadingSurface } from "@/components/ui/LoadingSurface"
import { Icon, type IconName } from "@/icons"
import {
  Banner,
  Button,
  Card,
  CardTitle,
  ConfirmDialog,
  IconButton,
  PageActions,
  Select,
  StatusPill,
  Switch,
  Tag,
  type SelectOption,
} from "@/ui"

/** 本体 + 分身最多 8 个（后端 island_clones 同一上限） */
const MAX_ISLANDS = 8

const POSITION_OPTIONS: SelectOption[] = [
  { value: "free", label: "自由移动" },
  { value: "top", label: "顶部停靠" },
  { value: "bottom", label: "底部停靠" },
  { value: "left", label: "左侧停靠" },
  { value: "right", label: "右侧停靠" },
]

export function IslandView() {
  const cfg = useSettings()
  const s = cfg.settings
  const savePreferences = cfg.setDisplayPreferences
  const disabled = !cfg.ready
  const linkage = s.island_linkage ?? DEFAULT_ISLAND_LINKAGE
  const cloneCount = s.island_clones.length

  return (
    <div className="@container flex w-full flex-col gap-3.5">
      <PageActions>
        <Button
          kind="ghost"
          size="sm"
          icon="refresh"
          loading={cfg.loading && cfg.ready}
          disabled={cfg.loading || cfg.saving}
          onClick={() => void cfg.reload()}
        >
          重新读取
        </Button>
      </PageActions>

      {cfg.error && (
        <Banner
          tone="danger"
          action={
            <Button kind="ghost" size="sm" disabled={cfg.loading || cfg.saving} onClick={() => void cfg.reload()}>
              重新读取
            </Button>
          }
        >
          {cfg.error}
        </Banner>
      )}

      <ContentTransition identity={String(cfg.ready)}>
        {cfg.loading && !cfg.ready ? (
          <LoadingSurface layout="island" label="正在读取灵动岛配置" />
        ) : (
          <div className="grid grid-cols-1 gap-3.5 @[900px]:grid-cols-[minmax(0,1fr)_396px] @[900px]:items-start">
            {/* 左栏：总开关 + 样式 + 连接与分身 + 联动 */}
            <div className="flex min-w-0 flex-col gap-3.5">
              {/* 显示灵动岛总开关 */}
              <Card className="flex-row items-center gap-3.5 px-[22px] py-[18px]">
                <span className="grid size-11 shrink-0 place-items-center rounded-[14px] bg-jn-primary-wash text-jn-primary">
                  <Icon name="navIsland" size="lg" />
                </span>
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="text-[15px] font-bold text-jn-text">显示灵动岛</span>
                  <span className="text-xs text-jn-text-3">控制本体与全部分身；与右上角按钮、系统托盘同步</span>
                </div>
                <StatusPill tone={s.island_visible ? "success" : "neutral"}>
                  {s.island_visible ? (cloneCount > 0 ? `本体 + ${cloneCount} 个分身` : "本体已显示") : "已隐藏"}
                </StatusPill>
                <Switch
                  checked={s.island_visible}
                  disabled={disabled || cfg.isSaving("island_visible") || !isTauri}
                  label="显示灵动岛"
                  onChange={(on) => void cfg.setIslandVisible(on)}
                />
              </Card>

              {/* 灵动岛样式 */}
              <StyleSection
                value={s.island_style === "capsule" ? "capsule" : "classic"}
                disabled={disabled || cfg.isSaving("island_style")}
                onChange={(style) => void cfg.setIslandStyle(style)}
              />

              {/* 连接与分身 */}
              <ClonesSection settings={s} ready={cfg.ready} />

              {/* 4.2b 灵动岛联动（无 R1-26 对应画板，标「待确认」） */}
              <GroupCard icon="spark" title="联动" badge="待确认">
                <Row
                  title="阈值与事件动效"
                  desc="额度到 80%、用尽、网关拒绝、新会话开始时岛上闪一下；免打扰或减少动效时只改颜色"
                  control={
                    <Switch
                      checked={linkage.signals}
                      disabled={disabled || cfg.isSaving("island_linkage")}
                      label="阈值与事件动效"
                      onChange={(on) => void cfg.setIslandLinkage({ signals: on })}
                    />
                  }
                />
                <Row
                  title="严格模式"
                  desc="「正在写」只认运行中的会话或网关在途；只有最近写入记录时显示「不知道」，宁可不亮"
                  control={
                    <Switch
                      checked={linkage.strict_writing}
                      disabled={disabled || cfg.isSaving("island_linkage")}
                      label="严格模式"
                      onChange={(on) => void cfg.setIslandLinkage({ strict_writing: on })}
                    />
                  }
                />
                <Row
                  title="跟随主窗口"
                  desc="主窗口在看某个连接的用量或供应商时，本体岛临时显示那个连接；分身不跟随"
                  control={
                    <Switch
                      checked={linkage.follow_main}
                      disabled={disabled || cfg.isSaving("island_linkage")}
                      label="跟随主窗口"
                      onChange={(on) => void cfg.setIslandLinkage({ follow_main: on })}
                    />
                  }
                />
                <Row
                  title="按最近活跃自动选主"
                  desc="分身比本体明显更活跃（领先 1 分钟以上且 2 分钟内有请求）时自动与本体互换；同平台不比较"
                  control={
                    <Switch
                      checked={linkage.auto_focus}
                      disabled={disabled || cfg.isSaving("island_linkage")}
                      label="按最近活跃自动选主"
                      onChange={(on) => void cfg.setIslandLinkage({ auto_focus: on })}
                    />
                  }
                />
              </GroupCard>
            </div>

            {/* 右栏：窗口行为 / 显示外观 / 刷新与位置 */}
            <div className="flex min-w-0 flex-col gap-3.5">
              <GroupCard icon="navIsland" title="窗口行为">
                <Row
                  title="灵动岛置顶"
                  desc="本体与全部分身保持在其他窗口之上"
                  control={
                    <Switch
                      checked={s.always_on_top}
                      disabled={disabled || cfg.isSaving("always_on_top")}
                      label="灵动岛置顶"
                      onChange={(on) => void cfg.setTopmost(on)}
                    />
                  }
                />
                <Row
                  title="贴边停靠"
                  desc="拖到屏幕边缘自动吸附；关闭后解除停靠"
                  control={
                    <Switch
                      checked={s.dock_enabled}
                      disabled={disabled || cfg.isSaving("dock_enabled")}
                      label="贴边停靠"
                      onChange={(on) => void cfg.setDockEnabled(on)}
                    />
                  }
                />
                <Row
                  title="免打扰"
                  desc="暂停增量提示、水位提醒与通知；继续采集"
                  control={
                    <Switch
                      checked={s.dnd}
                      disabled={disabled || cfg.isSaving("dnd")}
                      label="免打扰"
                      onChange={(on) => void cfg.setDnd(on)}
                    />
                  }
                />
              </GroupCard>

              <GroupCard icon="sliders" title="显示外观">
                <Row
                  title="透明度"
                  desc="仅影响灵动岛，菜单保持清晰"
                  control={
                    <OpacitySlider
                      value={s.island_opacity}
                      disabled={disabled || cfg.isSaving("island_opacity")}
                      onCommit={(v) => void savePreferences({ island_opacity: v })}
                    />
                  }
                />
                <Row
                  title="大小"
                  desc="用于探出与展开内容"
                  control={
                    <Select
                      width="sm"
                      label="大小"
                      disabled={disabled || cfg.isSaving("island_scale")}
                      value={String(s.island_scale)}
                      options={[
                        { value: "85", label: "紧凑 · 85%" },
                        { value: "100", label: "标准 · 100%" },
                        { value: "115", label: "大号 · 115%" },
                      ]}
                      onChange={(v) => void savePreferences({ island_scale: Number(v) })}
                    />
                  }
                />
                <Row
                  title="缩小后大小"
                  desc="仅调整经典样式的停靠条；胶囊贴边后固定为 32px"
                  control={
                    <Select
                      width="sm"
                      label="缩小后大小"
                      disabled={disabled || cfg.isSaving("island_shrink_scale")}
                      value={String(s.island_shrink_scale)}
                      options={[
                        { value: "75", label: "迷你 · 75%" },
                        { value: "100", label: "标准 · 100%" },
                        { value: "125", label: "大号 · 125%" },
                      ]}
                      onChange={(v) => void savePreferences({ island_shrink_scale: Number(v) })}
                    />
                  }
                />
              </GroupCard>

              <GroupCard icon="refresh" title="刷新与位置">
                <Row
                  title="自动刷新间隔"
                  desc="额度、余额与组织用量共用；本机会话采集仍实时运行"
                  control={
                    <div className="flex items-center gap-2">
                      <Select
                        width="full"
                        className="w-[100px]"
                        label="自动刷新间隔"
                        disabled={disabled || cfg.isSaving("refresh_minutes")}
                        value={String(s.refresh_minutes)}
                        options={[1, 5, 15, 30].map((n) => ({ value: String(n), label: `${n} 分钟` }))}
                        onChange={(v) => void savePreferences({ refresh_minutes: Number(v) })}
                      />
                      <ActionButton action="refresh" icon="refresh" label="立即刷新" done="已开始刷新" />
                    </div>
                  }
                />
                <Row
                  title="重置窗口位置"
                  desc="本体及分身回到主显示器、解除停靠并重新显示"
                  control={<ActionButton action="reset_layout" icon="scan" label="重置位置" done="已重置窗口位置" />}
                />
              </GroupCard>
            </div>
          </div>
        )}
      </ContentTransition>
    </div>
  )
}

const STYLE_OPTIONS: { value: IslandStyle; title: string; desc: string }[] = [
  { value: "classic", title: "经典卡片", desc: "宽 400 的方角卡片，额度用进度条显示" },
  { value: "capsule", title: "胶囊", desc: "高 40 的胶囊，bot 的眼睛表示状态，额度环套在 bot 外" },
]

/** 灵动岛样式：本体与全部分身共用，切换即时生效，两种样式读同一份数据 */
function StyleSection({ value, disabled, onChange }: { value: IslandStyle; disabled: boolean; onChange: (style: IslandStyle) => void }) {
  return (
    <Card
      header={
        <>
          <Icon name="palette" size="md" className="shrink-0 text-jn-text-2" />
          <CardTitle>灵动岛样式</CardTitle>
          <span className="flex-1" />
          <span className="text-xs text-jn-text-3">本体与全部分身共用 · 切换即时生效</span>
        </>
      }
    >
      <div role="radiogroup" aria-label="灵动岛样式" className="grid grid-cols-1 gap-3 @[540px]:grid-cols-2">
        {STYLE_OPTIONS.map((option) => {
          const active = option.value === value
          return (
            <button
              key={option.value}
              type="button"
              role="radio"
              aria-checked={active}
              disabled={disabled}
              onClick={() => { if (!active) onChange(option.value) }}
              className={cn(
                "jn-interactive flex flex-col gap-2.5 rounded-[18px] border p-3.5 text-left",
                active
                  ? "border-[1.5px] border-jn-primary bg-jn-primary-wash"
                  : "border-jn-line bg-jn-well hover:bg-jn-track/60",
              )}
            >
              <StylePreview style={option.value} />
              <span className="flex w-full items-center gap-2">
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className={cn("text-[13.5px] font-semibold", active ? "text-jn-primary-ink" : "text-jn-text")}>
                    {option.title}
                  </span>
                  <span className="whitespace-normal text-[11.5px] leading-snug text-jn-text-3">{option.desc}</span>
                </span>
                <span
                  aria-hidden
                  className={cn(
                    "grid size-[18px] shrink-0 place-items-center rounded-full border",
                    active ? "border-jn-primary bg-jn-primary text-jn-on-primary" : "border-jn-text-3/40 text-transparent",
                  )}
                >
                  <Icon name="check" size="xs" />
                </span>
              </span>
            </button>
          )
        })}
      </div>
      <p className="text-[11.5px] text-jn-text-3">缩略图是形状示意，不是真实数据。两种样式读同一份数据。</p>
    </Card>
  )
}

/** 样式缩略图（R1-26）：高对比深色硬件胶囊与卡片示意 */
function StylePreview({ style }: { style: IslandStyle }) {
  return (
    <span aria-hidden className="grid h-[78px] w-full place-items-center rounded-xl bg-jn-surface">
      {style === "capsule" ? (
        <span className="flex h-[34px] w-[150px] items-center gap-2 rounded-full bg-jn-inverse pl-[5px] pr-3 shadow-sm">
          <span className="grid size-[26px] shrink-0 place-items-center rounded-full border-[2.5px] border-jn-primary bg-jn-inverse-chip">
            <span className="flex gap-1">
              <i className="h-1.5 w-1 rounded-full bg-jn-on-inverse" />
              <i className="h-1.5 w-1 rounded-full bg-jn-on-inverse" />
            </span>
          </span>
          <span className="h-1.5 w-[70px] rounded-full bg-jn-inverse-chip" />
        </span>
      ) : (
        <span className="flex h-12 w-[170px] flex-col justify-center gap-[7px] rounded-lg bg-jn-inverse px-3 shadow-sm">
          <span className="flex items-center gap-1.5">
            <i className="size-2 rounded-full bg-jn-primary" />
            <i className="h-1.5 w-[60px] rounded-full bg-jn-inverse-chip" />
          </span>
          <span className="h-[5px] w-full overflow-hidden rounded-full bg-jn-inverse-chip">
            <b className="block h-full w-[56%] rounded-full bg-jn-primary" />
          </span>
        </span>
      )}
    </span>
  )
}

/** 「连接与分身」：本体在第一行，分身依次排列（R1-26） */
function ClonesSection({ settings, ready }: { settings: AppSettings; ready: boolean }) {
  const conn = useConnections()
  const toast = useToast()
  const cfg = useSettings()
  const [source, setSource] = useState("")
  const [creating, setCreating] = useState(false)
  const [destroying, setDestroying] = useState<{ id: string | null; name: string } | null>(null)
  const [destroyingBusy, setDestroyingBusy] = useState(false)
  const count = 1 + settings.island_clones.length
  const hidden = !settings.island_visible

  const rows = useMemo(() => [
    { id: null as string | null, name: "本体", connectionId: settings.island_connection_id, connectionName: settings.island_connection_name, edge: settings.dock.edge },
    ...settings.island_clones.map((clone, index) => ({ id: clone.id as string | null, name: `分身 ${index + 1}`, connectionId: clone.connection_id, connectionName: clone.connection_name, edge: clone.dock.edge })),
  ], [settings])

  const connectionOptions = useMemo<SelectOption[]>(() => conn.connections.map((c) => ({
    value: c.id,
    label: `${PLATFORMS.find((p) => p.id === c.platformId)?.name ?? c.platformId} · ${c.name}`,
  })), [conn.connections])

  const sourceRow = rows.find((row) => (row.id ?? "") === source) ?? rows[0]
  const sourceOptions: SelectOption[] = rows.map((row) => ({ value: row.id ?? "", label: `来源：${row.name}` }))

  const create = () => {
    setCreating(true)
    void api.islandAction("clone_create", sourceRow.id)
      .then(() => toast.success("已开启分身", `沿用${sourceRow.name}的连接`))
      .catch((reason) => toast.danger("开启分身失败", String(reason)))
      .finally(() => setCreating(false))
  }

  const confirmDestroy = async () => {
    if (!destroying) return
    setDestroyingBusy(true)
    try {
      await api.islandAction("clone_destroy", destroying.id)
      toast.success(`已销毁${destroying.name}`)
      setDestroying(null)
    } catch (reason) {
      toast.danger("销毁失败", String(reason))
    } finally {
      setDestroyingBusy(false)
    }
  }

  return (
    <Card
      className="gap-2 px-4 pt-4 pb-3"
      header={
        <>
          <Icon name="copy" size="md" className="shrink-0 text-jn-text-2" />
          <CardTitle>连接与分身</CardTitle>
          <span className="tnum text-xs text-jn-text-3">{count} / {MAX_ISLANDS}</span>
          <span className="flex-1" />
          <Select
            variant="filter"
            width="full"
            className="h-8 w-[124px]"
            label="新分身来源"
            disabled={!ready}
            value={sourceRow.id ?? ""}
            options={sourceOptions}
            onChange={setSource}
          />
          <Button
            kind="soft"
            size="sm"
            icon="add"
            loading={creating}
            disabled={!ready || !isTauri || creating || count >= MAX_ISLANDS}
            onClick={create}
          >
            开启分身
          </Button>
        </>
      }
    >
      <div role="table" aria-label="灵动岛窗口" className="flex flex-col gap-1">
        <div role="row" className="grid h-[30px] grid-cols-[112px_minmax(0,1fr)_124px_36px] items-center gap-3 px-2.5 text-[11.5px] font-medium text-jn-text-3">
          <span role="columnheader">灵动岛窗口</span>
          <span role="columnheader">显示连接</span>
          <span role="columnheader">显示位置</span>
          <span role="columnheader" className="text-right">操作</span>
        </div>
        {rows.map((row, index) => (
          <div
            key={row.id ?? "primary"}
            role="row"
            className={cn(
              "grid min-h-[52px] grid-cols-[112px_minmax(0,1fr)_124px_36px] items-center gap-3 rounded-[14px] px-2.5 py-1.5",
              index % 2 === 0 && "bg-jn-well",
            )}
          >
            <span role="cell" className="flex min-w-0 items-center gap-1.5">
              <span className="truncate text-[13px] font-semibold text-jn-text">{row.name}</span>
              {row.id === null && <Tag>本体</Tag>}
            </span>
            <span role="cell" className="min-w-0">
              <Select
                variant="field"
                width="full"
                className="h-9 bg-jn-surface"
                label={`${row.name}的显示连接`}
                disabled={!ready || hidden || conn.loading}
                value={row.connectionId}
                placeholder={row.connectionName ?? (conn.loading ? "读取连接中…" : "未选择连接")}
                options={connectionOptions}
                onChange={(id) => void cfg.setIslandConnection(id, row.id)}
              />
            </span>
            <span role="cell" className="min-w-0">
              <Select
                variant="field"
                width="full"
                className="h-9 bg-jn-surface"
                label={`${row.name}的显示位置`}
                disabled={!ready || hidden || !isTauri}
                value={row.edge ?? "free"}
                options={POSITION_OPTIONS}
                onChange={(value) => {
                  void api.islandPosition(value === "free" ? null : (value as DockEdgeDto), row.id)
                    .catch((reason) => toast.danger("调整显示位置失败", String(reason)))
                }}
              />
            </span>
            <span role="cell" className="flex justify-end">
              <RowMenu name={row.name} canDestroy={count > 1 && isTauri} onDestroy={() => setDestroying({ id: row.id, name: row.name })} />
            </span>
          </div>
        ))}
      </div>

      {conn.error && <p role="status" className="px-1 text-xs text-jn-danger">连接列表读取失败：{conn.error}</p>}
      <p className="px-1 pt-1 text-[11.5px] text-jn-text-3">
        新分身沿用来源窗口的连接；关闭显示时，连接与位置操作禁用。最多 {MAX_ISLANDS} 个，最后一个不可销毁。
      </p>

      <ConfirmDialog
        open={Boolean(destroying)}
        onOpenChange={(open) => { if (!open && !destroyingBusy) setDestroying(null) }}
        title={destroying ? `销毁「${destroying.name}」？` : ""}
        description="将关闭这个灵动岛并移除它的窗口配置。账号连接和历史统计会保留。"
        confirmText="销毁灵动岛"
        kind="danger"
        loading={destroyingBusy}
        onConfirm={() => void confirmDestroy()}
      />
    </Card>
  )
}

/** 分身「更多」菜单：危险操作收起在这里 */
function RowMenu({ name, canDestroy, onDestroy }: { name: string; canDestroy: boolean; onDestroy: () => void }) {
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild>
        <IconButton icon="more" label={`${name}的更多操作`} size="sm" kind="ghost" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="end" sideOffset={6} collisionPadding={12} className="jn-popover z-[140] min-w-[140px] rounded-[14px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface">
          <Menu.Item
            disabled={!canDestroy}
            onSelect={onDestroy}
            className="flex h-8 cursor-pointer select-none items-center gap-2 rounded-[10px] px-2.5 text-xs font-medium text-jn-danger outline-none data-[highlighted]:bg-jn-danger-soft data-[disabled]:pointer-events-none data-[disabled]:text-jn-text-3 data-[disabled]:opacity-50"
          >
            <Icon name="delete" size="sm" />
            销毁此灵动岛
          </Menu.Item>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

/** 立即刷新 / 重置位置按钮 */
function ActionButton({ action, icon, label, done }: { action: "refresh" | "reset_layout"; icon: IconName; label: string; done: string }) {
  const toast = useToast()
  const [busy, setBusy] = useState(false)
  const run = () => {
    setBusy(true)
    return api.islandAction(action)
      .then(() => toast.success(done))
      .catch((reason) => toast.show({ tone: "danger", title: `${label}失败`, detail: String(reason), action: { label: "重试", onClick: run } }))
      .finally(() => setBusy(false))
  }
  return (
    <Button kind="ghost" size="sm" icon={icon} loading={busy} disabled={busy || !isTauri} onClick={() => void run()}>
      {label}
    </Button>
  )
}

/** R1-26 右栏设置组卡片：圆角 24、白底、组标题带浅绿图标底 */
function GroupCard({ icon, title, badge, children }: { icon: IconName; title: string; badge?: string; children: ReactNode }) {
  return (
    <section className="flex w-full flex-col rounded-jn-card border border-jn-line bg-jn-surface px-5 py-1 shadow-jn-surface">
      <header className="flex h-[50px] items-center gap-2.5">
        <span className="grid size-[30px] shrink-0 place-items-center rounded-[10px] bg-jn-primary-wash text-jn-primary">
          <Icon name={icon} size="sm" />
        </span>
        <h2 className="text-[14px] font-bold text-jn-text">{title}</h2>
        {badge && <Tag>{badge}</Tag>}
      </header>
      <div className="flex flex-col [&>*]:border-t [&>*]:border-jn-line">{children}</div>
    </section>
  )
}

/** 透明度滑块（R1-26）：60%–100%，拖动即时显示数值，松手提交给后端。 */
function OpacitySlider({
  value,
  disabled,
  onCommit,
}: {
  value: number
  disabled: boolean
  onCommit: (next: number) => void
}) {
  const clamped = Math.max(60, Math.min(100, Math.round(value)))
  const [draft, setDraft] = useState(clamped)
  useEffect(() => { setDraft(clamped) }, [clamped])
  const ratio = ((draft - 60) / 40) * 100
  const commit = (next: number) => {
    if (next !== clamped) onCommit(next)
  }

  return (
    <div className={cn("flex w-[154px] items-center gap-2.5", disabled && "pointer-events-none opacity-45")}>
      <div className="relative flex h-5 flex-1 items-center">
        <span aria-hidden className="block h-1.5 w-full overflow-hidden rounded-full bg-jn-track">
          <span className="block h-full rounded-full bg-jn-primary" style={{ width: `${ratio}%` }} />
        </span>
        <span
          aria-hidden
          className="pointer-events-none absolute top-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-jn-primary bg-jn-surface shadow-sm"
          style={{ left: `${ratio}%` }}
        />
        <input
          type="range"
          min={60}
          max={100}
          step={2}
          disabled={disabled}
          aria-label="透明度"
          value={draft}
          onChange={(e) => setDraft(Number(e.target.value))}
          onPointerUp={() => commit(draft)}
          onKeyUp={() => commit(draft)}
          onBlur={() => commit(draft)}
          className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
        />
      </div>
      <span className="tnum w-9 shrink-0 text-right font-mono text-xs font-semibold text-jn-text">
        {draft}%
      </span>
    </div>
  )
}

function Row({ title, desc, control }: { title: string; desc: string; control: ReactNode }) {
  return (
    <div className="flex min-h-[62px] items-center gap-3.5 py-2.5">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-[13.5px] font-semibold text-jn-text">{title}</span>
        <span className="text-xs leading-snug text-jn-text-3">{desc}</span>
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  )
}
