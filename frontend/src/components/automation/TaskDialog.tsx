import * as Menu from "@radix-ui/react-dropdown-menu"
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { createPortal } from "react-dom"
import { cn } from "@/lib/utils"
import { useGlidePill } from "@/motion"
import { api, isTauri, type SessionItemDto } from "@/lib/api"
import {
  WEEKDAYS, accountText, dayTime, nextFire, projectOf, quotaTip, sessionTaskName, shortId, toSchedule, toolName, validateForm, type FormErrors, type TaskForm,
} from "@/lib/automation"
import {
  automationApi, type AutomationAccountDto, type AutomationPlatform, type QuotaVerdictDto, type TaskDto, type TaskInputDto, type ToolInfoDto,
} from "@/lib/automationApi"
import { sessionTitle } from "@/lib/sessionThreads"
import { dateKey, localZone, zoneName } from "@/lib/usageStats"
import { DEMO_MODELS, demoAccounts, demoQuota } from "@/mock/automation"
import { demoSessions } from "@/mock/sessionThreads"
import { Tip } from "@/components/ui/Tip"
import { SkillPicker } from "./SkillPicker"
import { Icon, type IconName } from "@/icons"
import { Collapse } from "@/motion"
import { Banner, Button, Checkbox as UiCheckbox, Dialog, IconButton, OptionTile } from "@/ui"

export interface TaskDialogRequest {
  mode: "create" | "edit"
  task?: TaskDto
  /** 来自会话线程的新建 */
  session?: SessionItemDto
  nonce: number
}

const PLATFORMS: AutomationPlatform[] = ["claude", "codex"]
const INPUT = "jn-interactive flex h-10 w-full items-center gap-2.5 rounded-[14px] border-[1.5px] border-transparent bg-jn-well px-3.5 text-xs text-jn-text outline-none focus-within:border-jn-primary focus-within:bg-jn-surface focus-visible:border-jn-primary focus-visible:bg-jn-surface"
const MENU_CLASS = "jn-popover z-[160] min-w-[var(--radix-dropdown-menu-trigger-width)] max-w-[360px] overflow-y-auto rounded-[16px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface"
const ITEM_CLASS = "relative flex min-h-8 cursor-pointer select-none items-center gap-2 rounded-[10px] py-1.5 pl-2.5 pr-7 text-xs text-jn-text outline-none data-[highlighted]:bg-jn-well data-[disabled]:opacity-40 data-[state=checked]:text-jn-primary-ink"

function samePath(a: string | null | undefined, b: string | null | undefined): boolean {
  const n = (p: string | null | undefined) => (p ?? "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase()
  return !!n(a) && n(a) === n(b)
}

function initialForm(req: TaskDialogRequest): TaskForm {
  const today = dateKey(new Date())
  const base: TaskForm = {
    name: "", platform: "claude", connectionId: "", model: "", cwd: "", sessionId: null, prompt: "", launch: "now",
    freq: "daily", time: "09:00", weekday: 1, date: today, timezone: localZone(), autoResume: true, deadline: "", serialQueue: true, skills: [],
  }
  if (req.task) {
    const t = req.task
    const pad = (n: number) => String(n).padStart(2, "0")
    const deadline = t.wait_deadline ? (() => { const d = new Date(t.wait_deadline); return `${dateKey(d)}T${pad(d.getHours())}:${pad(d.getMinutes())}` })() : ""
    return {
      ...base, name: t.name, platform: t.platform, connectionId: t.connection_id, model: t.model ?? "", cwd: t.cwd, sessionId: t.session_id,
      prompt: t.prompt, launch: t.launch, freq: t.schedule?.freq ?? "daily", time: t.schedule?.time ?? "09:00", weekday: t.schedule?.weekday ?? 1,
      date: t.schedule?.date ?? today, autoResume: t.auto_resume, deadline, serialQueue: t.serial_queue, skills: t.skills,
    }
  }
  if (req.session) {
    const s = req.session
    const platform: AutomationPlatform = s.platform === "codex" ? "codex" : "claude"
    return { ...base, name: sessionTaskName(s.title), platform, connectionId: s.connection_id ?? "", cwd: s.cwd ?? "", sessionId: s.session_id }
  }
  return base
}

function Field({ label, error, note, children }: { label: string; error?: string; note?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <span className="text-xs font-medium text-jn-text-2">{label}</span>
      {children}
      {error ? <span role="alert" className="text-[11px] text-jn-danger">{error}</span> : note && <span className="text-[11px] text-jn-text-3">{note}</span>}
    </div>
  )
}

interface Option { value: string; label: string; note?: string; disabled?: boolean; tag?: string }

function FieldSelect({ value, options, onChange, placeholder, label, disabled, invalid }: {
  value: string
  options: Option[]
  onChange: (v: string) => void
  placeholder: string
  label: string
  disabled?: boolean
  invalid?: boolean
}) {
  const glide = useGlidePill()
  const selected = options.find((o) => o.value === value)
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild disabled={disabled}>
        <button type="button" aria-label={label} className={cn(INPUT, "justify-between text-left disabled:cursor-not-allowed disabled:opacity-45 data-[state=open]:border-jn-primary data-[state=open]:bg-jn-surface", invalid && "border-jn-danger")}>
          <span className={cn("truncate", !selected && "text-jn-text-3")}>{selected?.label ?? placeholder}</span>
          <Icon name="expand" size="sm" className="jn-chevron shrink-0 text-jn-text-3" />
        </button>
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content ref={glide} align="start" sideOffset={6} collisionPadding={12} className={MENU_CLASS} style={{ maxHeight: "min(300px, var(--radix-dropdown-menu-content-available-height))" }}>
          {options.length ? (
            <Menu.RadioGroup value={value} onValueChange={onChange}>
              {options.map((o) => (
                <Menu.RadioItem key={o.value} value={o.value} disabled={o.disabled} className={ITEM_CLASS}>
                  <span className="flex min-w-0 flex-col">
                    <span className="flex items-center gap-1.5">
                      <span className="truncate">{o.label}</span>
                      {o.tag && <span className="rounded-full bg-jn-primary-wash px-1.5 py-0.5 text-[10.5px] font-medium text-jn-primary-ink">{o.tag}</span>}
                    </span>
                    {o.note && <span className="truncate text-[11px] text-jn-text-3">{o.note}</span>}
                  </span>
                  <Menu.ItemIndicator className="absolute right-2.5"><Icon name="check" size="xs" /></Menu.ItemIndicator>
                </Menu.RadioItem>
              ))}
            </Menu.RadioGroup>
          ) : <div className="px-2.5 py-1.5 text-xs text-jn-text-3">没有可选项</div>}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

function CheckRow({ checked, disabled, onChange, title, note }: { checked: boolean; disabled?: boolean; onChange?: (v: boolean) => void; title: string; note?: string }) {
  return (
    <div className="flex items-start gap-2.5">
      <div className="pt-0.5">
        <UiCheckbox checked={checked} disabled={disabled} label={title} onChange={(v) => onChange?.(v)} />
      </div>
      <button type="button" disabled={disabled} onClick={() => onChange?.(!checked)} className="flex min-w-0 flex-col gap-0.5 text-left outline-none disabled:cursor-not-allowed">
        <span className="text-xs font-medium text-jn-text">{title}</span>
        {note && <span className="text-[11px] text-jn-text-3">{note}</span>}
      </button>
    </div>
  )
}

const LAUNCH_OPTIONS: readonly { value: TaskForm["launch"]; label: string; icon: IconName }[] = [
  { value: "now", label: "立即执行", icon: "play" },
  { value: "schedule", label: "定时执行", icon: "clock" },
  { value: "quota", label: "额度恢复后执行", icon: "hourglass" },
]

/** 新建 / 编辑任务弹窗（R1-27）：统一 `@/ui` `Dialog` + `OptionTile` 启动方式 + `Collapse` 高级设置。 */
export function TaskDialog({ request, onClose, onSaved, onBackToSession }: {
  request: TaskDialogRequest
  onClose: () => void
  onSaved: (id: number, launch: TaskInputDto["launch"]) => void
  onBackToSession: (session: SessionItemDto) => void
}) {
  const [form, setForm] = useState<TaskForm>(() => initialForm(request))
  const [submitted, setSubmitted] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [accounts, setAccounts] = useState<AutomationAccountDto[] | null>(null)
  const [models, setModels] = useState<string[]>([])
  const [tools, setTools] = useState<ToolInfoDto[] | null>(null)
  const [quota, setQuota] = useState<QuotaVerdictDto | null>(null)
  const [quotaLoading, setQuotaLoading] = useState(false)
  const [advanced, setAdvanced] = useState(false)
  const [pickerOpen, setPickerOpen] = useState(false)
  const [skillsOpen, setSkillsOpen] = useState(false)
  const [root, setRoot] = useState<HTMLDivElement | null>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const now = Date.now()
  const set = <K extends keyof TaskForm>(key: K, value: TaskForm[K]) => setForm((f) => ({ ...f, [key]: value }))
  const fromSession = request.mode === "create" && !!request.session

  useEffect(() => {
    if (!isTauri) return setTools(PLATFORMS.map((platform) => ({ platform, found: true })))
    automationApi.tools().then(setTools, () => setTools(null))
  }, [])

  useEffect(() => {
    let alive = true
    setAccounts(null)
    const load = isTauri ? automationApi.accounts(form.platform) : Promise.resolve(demoAccounts(form.platform))
    load.then((list) => {
      if (!alive) return
      setAccounts(list)
      setForm((f) => (f.connectionId || request.mode === "edit" || fromSession ? f : { ...f, connectionId: list.find((a) => a.current)?.id ?? "" }))
    }, () => alive && setAccounts([]))
    return () => { alive = false }
  }, [form.platform, request.mode, fromSession])

  useEffect(() => {
    if (!form.connectionId) return setModels([])
    let alive = true
    const load = isTauri ? automationApi.models(form.platform, form.connectionId) : Promise.resolve(DEMO_MODELS[form.platform])
    load.then((list) => alive && setModels(list), () => alive && setModels([]))
    return () => { alive = false }
  }, [form.platform, form.connectionId])

  useEffect(() => {
    if (!form.connectionId) return setQuota(null)
    let alive = true
    setQuotaLoading(true)
    const load = isTauri ? automationApi.quota(form.connectionId, false) : Promise.resolve(demoQuota(Date.now()))
    load.then((v) => alive && setQuota(v), (e: unknown) => alive && setQuota({ state: "unknown", reason: String(e), resets_at: null, windows: [] }))
      .finally(() => alive && setQuotaLoading(false))
    return () => { alive = false }
  }, [form.connectionId])

  const account = accounts?.find((a) => a.id === form.connectionId)
  const accountName = account?.name ?? "所选账号"
  const tip = quota ? quotaTip(quota, accountName, now) : null
  const quotaDisabled = tip?.disabled ? tip.body : null
  const errors: FormErrors = submitted ? validateForm(form, now, quotaDisabled) : {}
  const tool = tools?.find((t) => t.platform === form.platform)
  const next = form.launch === "schedule" ? nextFire(toSchedule(form), now) : null

  const save = async () => {
    setSubmitted(true)
    const e = validateForm(form, Date.now(), quotaDisabled)
    if (Object.keys(e).length) return
    const input: TaskInputDto = {
      name: form.name.trim(), platform: form.platform, connection_id: form.connectionId, model: form.model || null, cwd: form.cwd.trim(),
      session_id: form.sessionId, prompt: form.prompt.trim(), skills: form.skills, launch: form.launch,
      schedule: form.launch === "schedule" ? toSchedule(form) : null, auto_resume: form.autoResume,
      wait_deadline: form.deadline ? new Date(form.deadline).getTime() : null, serial_queue: form.serialQueue,
    }
    if (!isTauri) return onSaved(request.task?.id ?? 0, input.launch)
    setSaving(true)
    setSaveError(null)
    try {
      if (request.mode === "edit" && request.task) {
        await automationApi.update(request.task.id, input)
        onSaved(request.task.id, input.launch)
      } else {
        const id = await automationApi.create(input)
        onSaved(id, input.launch)
      }
    } catch (reason) {
      setSaveError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setSaving(false)
    }
  }

  const pickDir = async () => {
    if (!isTauri) return set("cwd", "D:/develop/jolu")
    try {
      const dir = await automationApi.pickDir()
      if (dir) setForm((f) => ({ ...f, cwd: dir, sessionId: samePath(dir, f.cwd) ? f.sessionId : null }))
    } catch (reason) {
      setSaveError(String(reason))
    }
  }

  const title = request.mode === "edit" ? "编辑任务" : fromSession ? "新建任务 · 来自会话" : "新建任务"
  const primary = request.mode === "edit" ? "保存修改" : form.launch === "quota" ? "创建并等待" : "创建任务"
  const accountOptions: Option[] = (accounts ?? []).map((a) => ({
    value: a.id,
    label: accountText(a.name, a.kind),
    tag: a.current ? "当前登录" : undefined,
    note: a.current ? undefined : `不是 ${toolName(form.platform)} 当前登录的账号，执行前需在供应商页启用`,
  }))
  const modelOptions: Option[] = [{ value: "", label: `跟随 ${toolName(form.platform)} 默认`, note: "不指定模型参数" }, ...models.map((m) => ({ value: m, label: m }))]

  return (
    <Dialog
      open
      onOpenChange={(open) => { if (!open && !saving) onClose() }}
      preventClose={saving}
      panelRef={setRoot}
      bodyRef={scrollRef}
      width="lg"
      title={title}
      description="配置执行工具、账号、项目目录与触发方式；本地任务由 Jolu 后台调度。"
      footerStart={saveError ? <span role="alert" className="text-jn-danger">{saveError}</span> : undefined}
      actions={
        <>
          {fromSession && request.session ? (
            <Button kind="ghost" size="md" disabled={saving} onClick={() => onBackToSession(request.session!)}>返回会话</Button>
          ) : (
            <Button kind="ghost" size="md" disabled={saving} onClick={onClose}>取消</Button>
          )}
          <Button kind="primary" size="md" icon="add" loading={saving} disabled={saving} onClick={() => void save()}>
            {saving ? "正在保存…" : primary}
          </Button>
        </>
      }
    >
      <Field label="任务名称 *" error={errors.name}>
        <input value={form.name} onChange={(e) => set("name", e.target.value)} maxLength={80} placeholder="例如：完善登录页面" className={cn(INPUT, "placeholder:text-jn-text-3", errors.name && "border-jn-danger")} />
      </Field>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Field label="执行工具 *" note={tool && !tool.found ? `未检测到 ${toolName(form.platform)}，执行时会提示安装` : undefined}>
          <FieldSelect
            label="执行工具"
            value={form.platform}
            placeholder="选择执行工具"
            onChange={(v) => setForm((f) => ({ ...f, platform: v as AutomationPlatform, connectionId: "", model: "", sessionId: null }))}
            options={PLATFORMS.map((p) => ({ value: p, label: toolName(p), tag: "已适配", note: tools && !tools.find((t) => t.platform === p)?.found ? "本机未检测到" : undefined }))}
          />
        </Field>
        <Field label="执行账号 *" error={errors.connectionId} note={account && !account.current ? "执行时会检查账号；Jolu 不会自动切换原工具账号" : undefined}>
          <FieldSelect
            label="执行账号"
            value={form.connectionId}
            placeholder={accounts === null ? "正在读取账号…" : accounts.length ? "请选择可用账号" : "没有可用账号"}
            onChange={(v) => set("connectionId", v)}
            options={accountOptions}
            invalid={!!errors.connectionId}
          />
        </Field>
      </div>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Field label="模型 *">
          <FieldSelect label="模型" value={form.model} placeholder="选择账号后获取模型" disabled={!form.connectionId} onChange={(v) => set("model", v)} options={form.connectionId ? modelOptions : []} />
        </Field>
        <Field label="项目目录 *" error={errors.cwd}>
          <div className={cn(INPUT, "pr-1.5", errors.cwd && "border-jn-danger")}>
            <input value={form.cwd} onChange={(e) => set("cwd", e.target.value)} placeholder="选择项目目录" className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-jn-text-3" />
            <Tip label="选择目录">
              <IconButton icon="openFolder" label="选择项目目录" size="sm" kind="ghost" onClick={() => void pickDir()} />
            </Tip>
          </div>
        </Field>
      </div>

      <SessionPicker
        form={form}
        open={pickerOpen}
        onOpenChange={setPickerOpen}
        accountName={account?.name ?? null}
        fromSession={fromSession}
        onPick={(session) => {
          setForm((f) => ({ ...f, sessionId: session?.session_id ?? null, cwd: session?.cwd && !f.cwd ? session.cwd : f.cwd }))
          setPickerOpen(false)
        }}
        initial={request.session ?? null}
        root={root}
        scroller={scrollRef.current}
      />

      <Field label="执行内容 *" error={errors.prompt}>
        <textarea
          value={form.prompt}
          onChange={(e) => set("prompt", e.target.value)}
          placeholder={form.sessionId ? "描述希望 AI 在这个会话中继续完成的任务……" : "描述希望 AI 完成的任务……"}
          className={cn("jn-interactive h-24 w-full resize-none rounded-[14px] border-[1.5px] border-transparent bg-jn-well p-3.5 text-xs leading-[1.6] text-jn-text outline-none placeholder:text-jn-text-3 focus:border-jn-primary focus:bg-jn-surface", errors.prompt && "border-jn-danger")}
        />
      </Field>

      <SkillPicker
        platform={form.platform}
        cwd={form.cwd}
        value={form.skills}
        onChange={(names) => set("skills", names)}
        open={skillsOpen}
        onOpenChange={setSkillsOpen}
        root={root}
        scroller={scrollRef.current}
      />

      <Field label="启动方式" error={errors.launch}>
        <div role="radiogroup" aria-label="启动方式" className="grid grid-cols-1 gap-2 sm:grid-cols-3">
          {LAUNCH_OPTIONS.map((item) => {
            const checked = form.launch === item.value
            const disabled = item.value === "quota" && !!quotaDisabled
            return (
              <OptionTile
                key={item.value}
                active={checked}
                label={item.label}
                icon={item.icon}
                disabled={disabled && !checked}
                onClick={() => set("launch", item.value)}
              />
            )
          })}
        </div>
      </Field>

      {form.launch === "schedule" && (
        <div className="flex flex-col gap-3 rounded-[16px] border border-jn-line bg-jn-surface p-3.5">
          <div className={cn("grid gap-3", form.freq === "daily" ? "grid-cols-2" : "grid-cols-3")}>
            <Field label="执行频率">
              <FieldSelect label="执行频率" value={form.freq} placeholder="选择频率" onChange={(v) => set("freq", v as TaskForm["freq"])} options={[{ value: "once", label: "单次" }, { value: "daily", label: "每天" }, { value: "weekly", label: "每周" }]} />
            </Field>
            {form.freq === "weekly" && (
              <Field label="星期">
                <FieldSelect label="星期" value={String(form.weekday)} placeholder="选择星期" onChange={(v) => set("weekday", Number(v))} options={WEEKDAYS.map((w, i) => ({ value: String(i + 1), label: w }))} />
              </Field>
            )}
            {form.freq === "once" && (
              <Field label="执行日期">
                <input type="date" value={form.date} min={dateKey(new Date())} onChange={(e) => set("date", e.target.value)} className={INPUT} />
              </Field>
            )}
            <Field label="执行时间" error={errors.time}>
              <input type="time" value={form.time} onChange={(e) => set("time", e.target.value)} className={cn(INPUT, errors.time && "border-jn-danger")} />
            </Field>
          </div>
          <Field label="时区" note="按本机时区调度">
            <FieldSelect label="时区" value={form.timezone} placeholder="本机时区" onChange={(v) => set("timezone", v)} options={[{ value: localZone(), label: localZone(), note: "本机时区" }]} />
          </Field>
          {next && <span className="text-xs font-medium text-jn-primary-ink">下次运行：{dayTime(next, now)}（{zoneName(form.timezone)}时间）</span>}
        </div>
      )}

      {form.launch === "quota" && (
        form.connectionId ? (
          quotaLoading && !quota ? (
            <Banner tone="info">正在读取额度…</Banner>
          ) : tip && (
            <Banner tone={tip.disabled ? "danger" : "info"}>
              <span className="font-semibold">{tip.title}</span> — {tip.body}
            </Banner>
          )
        ) : <Banner tone="info">选择账号后读取额度：仅已适配且可读取额度的账号可选择此启动方式。</Banner>
      )}

      {fromSession && (
        <Banner tone="info">
          <span className="font-semibold text-jn-primary-ink">已带入当前会话</span>：
          {request.session?.connection_id
            ? "请填写任务内容与启动方式；创建前将验证账号与原会话是否兼容。"
            : "请先选择执行账号、填写任务内容。当前会话未归属账号，创建前将验证账号与原会话是否兼容。"}
        </Banner>
      )}

      <CheckRow checked={form.autoResume} onChange={(v) => set("autoResume", v)} title="执行中额度耗尽时，等待恢复后自动继续" note="续跑能力以所选工具为准；不能安全续跑时保留记录并提示处理。" />

      <div>
        <button type="button" aria-expanded={advanced} onClick={() => setAdvanced((v) => !v)} className="jn-interactive flex h-8 items-center gap-2 rounded-[10px] px-1.5 text-left outline-none hover:bg-jn-well">
          <Icon name="next" size="sm" className={cn("text-jn-text-3 transition-transform", advanced && "rotate-90")} />
          <span className="text-xs font-medium text-jn-text">高级设置</span>
          <span className="text-[11px] text-jn-text-3">{form.deadline ? `等待至 ${dayTime(new Date(form.deadline).getTime(), now)}` : "不限等待时间"}{form.serialQueue ? " · 同账号顺序执行" : " · 同账号可并行"}</span>
        </button>
        <Collapse open={advanced}>
          <div className="mt-2 flex flex-col gap-3 rounded-[16px] border border-jn-line bg-jn-surface p-3.5">
            <Field label="等待截止时间" error={errors.deadline}>
              <div className="grid grid-cols-2 gap-2">
                <FieldSelect
                  label="等待截止时间"
                  value={form.deadline ? "until" : "none"}
                  placeholder="不限制"
                  onChange={(v) => set("deadline", v === "none" ? "" : `${dateKey(new Date(Date.now() + 86_400_000))}T09:00`)}
                  options={[{ value: "none", label: "不限制" }, { value: "until", label: "指定截止时间" }]}
                />
                {form.deadline && <input type="datetime-local" value={form.deadline} onChange={(e) => set("deadline", e.target.value)} className={INPUT} />}
              </div>
            </Field>
            <CheckRow
              checked={form.serialQueue}
              onChange={(v) => set("serialQueue", v)}
              title="同一账号按队列顺序执行"
              note={form.serialQueue ? "默认开启，暂停当前任务后再处理后续任务。" : "同账号任务可能同时运行并争用额度；同一工作目录仍按顺序执行。"}
            />
            <span className="text-[11px] text-jn-text-3">到期仍不可用会标记「等待超时」并保留记录，等额度恢复。</span>
          </div>
        </Collapse>
      </div>

      <span className="text-[11px] text-jn-text-3">
        {fromSession
          ? "来源会话不会被替换为新会话。账号或项目发生变化时重新校验；需要用户重新选择时明确提示。"
          : form.launch === "quota"
            ? "额度恢复任务默认执行一次。仅已适配且可读取额度的账号可选择此启动方式。"
            : "本地任务需要 Jolu 后台运行；所选账号必须是原工具当前登录的账号。"}
      </span>
    </Dialog>
  )
}

/** 执行会话搜索选择：新建会话 + 当前项目里可继续的会话；仅用量记录的会话不可选 */
function SessionPicker({ form, open, onOpenChange, accountName, fromSession, onPick, initial, root, scroller }: {
  form: TaskForm
  open: boolean
  onOpenChange: (open: boolean) => void
  accountName: string | null
  fromSession: boolean
  onPick: (session: SessionItemDto | null) => void
  initial: SessionItemDto | null
  root: HTMLElement | null
  scroller: HTMLElement | null
}) {
  const [query, setQuery] = useState("")
  const [sessions, setSessions] = useState<SessionItemDto[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [known, setKnown] = useState<SessionItemDto | null>(initial)
  const ref = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const [place, setPlace] = useState<{ top: number; left: number; width: number; maxHeight: number } | null>(null)

  useEffect(() => {
    if (!open) return
    let alive = true
    setError(null)
    const timer = window.setTimeout(() => {
      const load = isTauri
        ? api.sessionsList({ platform: form.platform, connection: null, project: null, since: null, search: query.trim() || null })
        : Promise.resolve(demoSessions(Date.now()))
      load.then((list) => alive && setSessions(list.sessions.filter((s) => s.platform === form.platform)), (e: unknown) => alive && setError(String(e)))
    }, query ? 250 : 0)
    return () => { alive = false; window.clearTimeout(timer) }
  }, [open, query, form.platform])

  useLayoutEffect(() => {
    if (!open || !root) return setPlace(null)
    const update = () => {
      const trigger = triggerRef.current
      if (!trigger) return
      const box = root.getBoundingClientRect()
      const r = trigger.getBoundingClientRect()
      const top = r.bottom - box.top + 6
      setPlace({ top, left: r.left - box.left, width: r.width, maxHeight: Math.max(160, box.height - top - 12) })
    }
    update()
    scroller?.addEventListener("scroll", update)
    window.addEventListener("resize", update)
    return () => {
      scroller?.removeEventListener("scroll", update)
      window.removeEventListener("resize", update)
    }
  }, [open, root, scroller])

  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (!ref.current?.contains(target) && !panelRef.current?.contains(target)) onOpenChange(false)
    }
    document.addEventListener("mousedown", onDown)
    return () => document.removeEventListener("mousedown", onDown)
  }, [open, onOpenChange])

  const matches = useMemo(() => (sessions ?? []).filter((s) => !form.cwd || samePath(s.cwd, form.cwd)).slice(0, 30), [sessions, form.cwd])
  const selected = form.sessionId ? (matches.find((s) => s.session_id === form.sessionId) ?? sessions?.find((s) => s.session_id === form.sessionId) ?? (known?.session_id === form.sessionId ? known : null)) : null
  const project = projectOf(form.cwd || null)

  return (
    <div ref={ref} className="relative flex flex-col gap-1.5">
      <span className="text-xs font-medium text-jn-text-2">执行会话</span>
      <button
        ref={triggerRef}
        type="button"
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
        className={cn("jn-interactive flex h-[54px] w-full items-center gap-2.5 rounded-[14px] border-[1.5px] bg-jn-well px-3.5 text-left outline-none focus-visible:border-jn-primary", open ? "border-jn-primary bg-jn-surface" : "border-transparent")}
      >
        <Icon name="navSessions" size="md" className="shrink-0 text-jn-primary-ink" />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-xs font-semibold text-jn-text">{form.sessionId ? (selected ? sessionTitle(selected) : `会话 ${shortId(form.sessionId)}`) : "新建会话"}</span>
          <span className="truncate text-[11px] text-jn-text-3">
            {form.sessionId
              ? `${project} · ${selected ? dayTime(selected.last_ts, Date.now()) : toolName(form.platform)} · ${shortId(form.sessionId)}`
              : "从新的上下文开始"}
          </span>
        </span>
        <Icon name={open ? "collapse" : "expand"} size="sm" className="shrink-0 text-jn-text-3" />
      </button>
      <span className="text-[11px] text-jn-text-3">
        {form.sessionId ? (fromSession ? "已从会话线程带入；执行时验证是否可在此会话继续。" : "将在此会话追加任务指令，保留已有上下文。") : "将新建会话执行任务。"}
      </span>

      {open && root && place && createPortal(
        <div
          ref={panelRef}
          style={{ top: place.top, left: place.left, width: place.width, maxHeight: place.maxHeight }}
          className="jn-popover absolute z-20 flex flex-col gap-1.5 overflow-hidden rounded-[16px] border border-jn-line bg-jn-surface p-2 shadow-jn-surface"
        >
          <label className="flex h-9 items-center gap-2 rounded-full border border-jn-primary bg-jn-surface px-3">
            <Icon name="search" size="sm" className="text-jn-text-3" />
            <input autoFocus value={query} onChange={(e) => setQuery(e.target.value)} placeholder="搜索会话标题、ID 或正文" aria-label="搜索会话" className="min-w-0 flex-1 bg-transparent text-xs text-jn-text outline-none placeholder:text-jn-text-3" />
            {query && (
              <button type="button" aria-label="清除搜索" onClick={() => setQuery("")} className="text-jn-text-3 hover:text-jn-text">
                <Icon name="close" size="xs" />
              </button>
            )}
          </label>
          <button type="button" onClick={() => onPick(null)} className={cn("jn-interactive flex h-9 items-center gap-2.5 rounded-[10px] px-2.5 text-left outline-none hover:bg-jn-well", !form.sessionId && "bg-jn-primary-wash")}>
            <Icon name="add" size="sm" className="text-jn-primary-ink" />
            <span className="text-xs font-medium text-jn-primary-ink">新建会话</span>
            <span className="text-[11px] text-jn-text-3">从新的上下文开始</span>
          </button>
          <span className="px-2.5 text-[11px] text-jn-text-3">{form.cwd ? `${project} · ` : ""}{sessions === null && !error ? "正在读取会话…" : `匹配 ${matches.length} 条会话`}</span>
          {error && <span role="alert" className="px-2.5 text-xs text-jn-danger">会话读取失败：{error}</span>}
          <div className="flex max-h-[240px] min-h-0 shrink flex-col gap-0.5 overflow-y-auto">
            {matches.map((s) => {
              const usable = s.has_source
              const checked = s.session_id === form.sessionId
              return (
                <button
                  key={s.session_id}
                  type="button"
                  disabled={!usable}
                  onClick={() => { setKnown(s); onPick(s) }}
                  className={cn("jn-interactive flex h-14 shrink-0 items-center gap-2.5 rounded-[10px] px-2.5 text-left outline-none hover:bg-jn-well disabled:cursor-not-allowed disabled:hover:bg-transparent", checked && "bg-jn-primary-wash hover:bg-jn-primary-wash")}
                >
                  <Icon name={usable ? "navSessions" : "fileText"} size="sm" className="shrink-0 text-jn-text-3" />
                  <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className={cn("truncate text-xs", checked ? "font-semibold text-jn-primary-ink" : usable ? "text-jn-text" : "text-jn-text-3")}>{sessionTitle(s)}</span>
                    <span className="truncate text-[11px] text-jn-text-3">{usable ? `${dayTime(s.last_ts, Date.now())} · ${shortId(s.session_id)}` : "仅用量记录 · 无法继续此会话"}</span>
                  </span>
                  {checked && <Icon name="check" size="sm" className="text-jn-primary-ink" />}
                  {!usable && <Icon name="info" size="sm" className="text-jn-text-3" />}
                </button>
              )
            })}
          </div>
          <span className="border-t border-jn-line px-2.5 pt-2 text-[11px] text-jn-text-3">{toolName(form.platform)}{accountName ? ` · ${accountName}` : ""}{form.cwd ? ` · ${project}` : ""}</span>
        </div>,
        root,
      )}
    </div>
  )
}
