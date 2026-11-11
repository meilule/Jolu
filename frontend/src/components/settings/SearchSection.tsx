import { useCallback, useEffect, useMemo, useState } from "react"
import { Globe, Plus, Plug, Search, Trash2 } from "lucide-react"
import { isTauri, listenEvent } from "@/lib/api"
import { settingsApi } from "@/lib/settingsApi"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import { SelectField } from "@/components/ui/SelectField"
import { cn } from "@/lib/utils"
import {
  SEARCH_PLUGIN_NAME, agentLabel, draftsChanged, nextVendor, pluginState, pluginStateText, toDrafts, toInputs, validateDrafts,
  type LibraryAgentDto, type SearchDraft, type SearchHitDto, type SearchVendorDto,
} from "@/lib/searchSettings"
import { ActionButton, Notice, SettingsGroup, SettingsRow, Switch } from "./settingsUi"

/**
 * 联网搜索与搜索插件。
 *
 * 待确认：`Jolu.pen` 模块 G（画板 86–103）没有这两块，按既有 `SettingsGroup` 布局做最小版，
 * 挂在「网络与代理」页尾（搜索请求走应用的联网方式）。画板补齐后再定稿。
 *
 * Key 只以掩码显示；编辑框里保留掩码 = 保留原 Key，删掉那一行 = 删除那把 Key，
 * 新填的 Key 原文只在保存时提交一次。
 */
export function SearchSection() {
  const cfg = useSettings()
  const toast = useToast()
  const saved = cfg.settings.search_apis
  const [vendors, setVendors] = useState<SearchVendorDto[]>([])
  const [drafts, setDrafts] = useState<SearchDraft[]>(() => toDrafts(saved))
  const [saving, setSaving] = useState(false)
  const [query, setQuery] = useState("")
  const [trying, setTrying] = useState(false)
  const [hits, setHits] = useState<SearchHitDto[] | null>(null)
  const [tryError, setTryError] = useState<string | null>(null)
  const disabled = !cfg.ready || !isTauri

  const [vendorError, setVendorError] = useState<string | null>(null)
  useEffect(() => {
    if (!isTauri) return
    settingsApi.searchVendors().then(setVendors).catch((reason) => setVendorError(`读取搜索后端清单失败：${reason}`))
  }, [])

  // 后端广播新设置时（含本页保存），草稿与真值对齐
  useEffect(() => { setDrafts(toDrafts(saved)) }, [saved])

  const changed = draftsChanged(drafts, saved)
  const problem = useMemo(() => validateDrafts(drafts, vendors), [drafts, vendors])
  const vendorOptions = vendors.map((vendor) => ({ value: vendor.id, label: vendor.name }))
  const update = (index: number, patch: Partial<SearchDraft>) =>
    setDrafts((current) => current.map((draft, at) => (at === index ? { ...draft, ...patch } : draft)))

  const save = async () => {
    if (problem) return
    setSaving(true)
    try {
      await settingsApi.searchApisSet(toInputs(drafts))
      toast.success("搜索后端已保存", "Key 只以掩码显示")
    } catch (reason) {
      toast.danger("搜索后端未保存", String(reason))
    } finally {
      setSaving(false)
    }
  }

  const trySearch = async () => {
    if (!query.trim()) return
    setTrying(true)
    setTryError(null)
    setHits(null)
    try {
      setHits(await settingsApi.searchTry(query.trim()))
    } catch (reason) {
      setTryError(String(reason))
    } finally {
      setTrying(false)
    }
  }

  return (
    <>
      <SettingsGroup
        icon={Globe}
        title="联网搜索"
        desc="给不带搜索能力的模型补 web_search / web_fetch；按顺序尝试，前一个失败自动换下一个。待确认：暂无设计稿。"
      >
        {vendorError && <Notice tone="danger" className="mb-3">{vendorError}</Notice>}
        {drafts.length === 0 && (
          <p className="border-t border-jolu-border py-3.5 text-xs text-jolu-muted">还没有搜索后端。You.com 可不填 Key 先用（每天有次数限制）。</p>
        )}
        {drafts.map((draft, index) => {
          const vendor = vendors.find((item) => item.id === draft.vendor)
          return (
            <div key={index} className="flex flex-col gap-2 border-t border-jolu-border py-3.5" data-search-row>
              <div className="flex items-center gap-2">
                <span className="w-5 shrink-0 text-center text-[11px] text-jolu-muted">{index + 1}</span>
                <SelectField
                  label="搜索后端"
                  value={draft.vendor}
                  options={vendorOptions}
                  disabled={disabled}
                  onValueChange={(value) => update(index, { vendor: value })}
                  className="w-[200px]"
                />
                <span className="flex-1" />
                <span className="text-[11px] text-jolu-muted">暂停</span>
                <Switch label={`暂停 ${vendor?.name ?? draft.vendor}`} on={draft.off} disabled={disabled} onChange={(off) => update(index, { off })} />
                <ActionButton
                  tone="danger"
                  aria-label={`删除 ${vendor?.name ?? draft.vendor}`}
                  icon={<Trash2 aria-hidden className="size-3.5" />}
                  disabled={disabled}
                  onClick={() => setDrafts((current) => current.filter((_, at) => at !== index))}
                />
              </div>
              <textarea
                aria-label={`${vendor?.name ?? draft.vendor} 的 Key`}
                value={draft.keysText}
                disabled={disabled}
                spellCheck={false}
                rows={Math.min(4, Math.max(2, draft.keysText.split("\n").length))}
                placeholder={vendor?.key_optional ? "可不填；多把 Key 每行一把" : "每行一把 Key，被拒的会自动排到最后"}
                onChange={(event) => update(index, { keysText: event.target.value })}
                className="ml-7 min-w-0 resize-y rounded-lg border border-jolu-border bg-transparent px-2.5 py-2 font-mono text-xs text-jolu-text outline-none placeholder:font-sans placeholder:text-jolu-muted focus:border-jolu-accent disabled:opacity-45"
              />
              {(vendor?.needs_url || draft.url) && (
                <input
                  aria-label={`${vendor?.name ?? draft.vendor} 的地址`}
                  value={draft.url}
                  disabled={disabled}
                  spellCheck={false}
                  placeholder={vendor?.needs_url ? "实例地址，如 https://searx.example.com" : "自定义地址（留空用官方地址）"}
                  onChange={(event) => update(index, { url: event.target.value })}
                  className="ml-7 h-9 min-w-0 rounded-lg border border-jolu-border bg-transparent px-2.5 text-xs text-jolu-text outline-none placeholder:text-jolu-muted focus:border-jolu-accent disabled:opacity-45"
                />
              )}
            </div>
          )
        })}
        <div className="flex items-center gap-2 border-t border-jolu-border py-3.5">
          <ActionButton
            icon={<Plus aria-hidden className="size-3.5" />}
            disabled={disabled || vendors.length === 0}
            onClick={() => setDrafts((current) => [...current, { vendor: nextVendor(current, vendors), keysText: "", url: "", off: false }])}
          >
            添加后端
          </ActionButton>
          <span className={cn("flex-1 truncate text-[11px]", problem && changed ? "text-jolu-danger" : "text-jolu-muted")}>
            {changed ? problem ?? "有未保存的修改" : "保留编辑框里的掩码 = 保留原 Key"}
          </span>
          <ActionButton tone="secondary" disabled={disabled || !changed || saving} onClick={() => setDrafts(toDrafts(saved))}>撤销</ActionButton>
          <ActionButton tone="accent" busy={saving} disabled={disabled || !changed || !!problem} onClick={save}>保存</ActionButton>
        </div>
        <div className="flex flex-col gap-2 border-t border-jolu-border py-3.5">
          <div className="flex items-center gap-2">
            <label className="flex h-9 min-w-0 flex-1 items-center gap-2 rounded-lg border border-jolu-border px-2.5 focus-within:border-jolu-accent">
              <Search aria-hidden className="size-[15px] shrink-0 text-jolu-muted" />
              <input
                aria-label="试搜关键词"
                value={query}
                disabled={disabled}
                placeholder="用已保存的配置试搜一次"
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => { if (event.key === "Enter") trySearch() }}
                className="min-w-0 flex-1 bg-transparent text-xs text-jolu-text outline-none placeholder:text-jolu-muted disabled:opacity-45"
              />
            </label>
            <ActionButton busy={trying} disabled={disabled || !query.trim() || changed} onClick={trySearch}>试搜</ActionButton>
          </div>
          {changed && <span className="text-[11px] text-jolu-muted">先保存，再试搜。</span>}
          {tryError && <Notice tone="danger">{tryError}</Notice>}
          {hits && hits.length === 0 && <span className="text-[11px] text-jolu-muted">没有结果。</span>}
          {hits && hits.length > 0 && (
            <ol className="flex flex-col gap-2" aria-label="试搜结果">
              {hits.map((hit, index) => (
                <li key={`${hit.url}-${index}`} className="flex min-w-0 flex-col gap-0.5">
                  <span className="truncate text-xs font-medium text-jolu-text">{index + 1}. {hit.title}</span>
                  <span className="truncate font-mono text-[10.5px] text-jolu-muted">{hit.url}</span>
                  {hit.snippet && <span className="line-clamp-2 text-[11px] text-jolu-muted">{hit.snippet}</span>}
                </li>
              ))}
            </ol>
          )}
        </div>
      </SettingsGroup>
      <SearchPluginGroup />
    </>
  )
}

/** 把「Jolu 搜索」以 MCP server 装进各 agent。只动 Jolu 自己写的条目。 */
function SearchPluginGroup() {
  const toast = useToast()
  const [rows, setRows] = useState<LibraryAgentDto[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)

  const load = useCallback(() => {
    if (!isTauri) return
    settingsApi.libraryList().then((next) => { setRows(next); setError(null) }).catch((reason) => setError(String(reason)))
  }, [])

  useEffect(() => {
    load()
    let stop = () => {}
    let stopped = false
    listenEvent("library-changed", load).then((unlisten) => { if (stopped) unlisten(); else stop = unlisten })
    return () => { stopped = true; stop() }
  }, [load])

  const act = async (row: LibraryAgentDto) => {
    const installed = pluginState(row) === "installed"
    setBusy(row.agent)
    try {
      if (installed) {
        await settingsApi.libraryRemove(row.agent, SEARCH_PLUGIN_NAME)
        toast.success(`已从 ${agentLabel(row.agent)} 移除`, "只删了 Jolu 装的条目")
      } else {
        await settingsApi.libraryInstall(row.agent)
        toast.success(`已装到 ${agentLabel(row.agent)}`, row.note ?? `重启 ${agentLabel(row.agent)} 后可用 web_search / web_fetch`)
      }
    } catch (reason) {
      toast.danger(installed ? "移除失败" : "安装失败", String(reason))
    } finally {
      setBusy(null)
      load()
    }
  }

  return (
    <SettingsGroup icon={Plug} title="搜索插件（MCP）" desc="把 Jolu 搜索装进各工具，工具自己就能联网搜索与读网页。同名的第三方条目不会被覆盖。待确认：暂无设计稿。">
      {error && <Notice tone="danger" className="mb-3">{error}</Notice>}
      {!isTauri && <p className="border-t border-jolu-border py-3.5 text-xs text-jolu-muted">浏览器预览不读取本机工具配置。</p>}
      {rows?.map((row) => {
        const state = pluginState(row)
        return (
          <SettingsRow
            key={row.agent}
            className="min-h-[58px]"
            title={agentLabel(row.agent)}
            desc={<>{pluginStateText(row)}{row.note && <> · {row.note}</>}</>}
            control={state === "unsupported" ? undefined : (
              <ActionButton
                tone={state === "installed" ? "danger" : "soft"}
                busy={busy === row.agent}
                disabled={busy !== null && busy !== row.agent}
                onClick={() => act(row)}
              >
                {state === "installed" ? "移除" : "安装"}
              </ActionButton>
            )}
          />
        )
      })}
    </SettingsGroup>
  )
}
