import { SelectField } from "@/components/ui/SelectField"
import { useBrowserAuthorization } from "@/hooks/useBrowserAuthorization"
import { OPENCODE_SERVICES } from "@/lib/providerCatalog"
import { useEffect, useId, useRef, useState, type ReactNode } from "react"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { Check, ChevronDown, CircleAlert, CircleCheck, Eye, EyeOff, LoaderCircle, Search } from "lucide-react"
import { api, isTauri, type LoginDetectionDto } from "@/lib/api"
import { cn } from "@/lib/utils"
import {
  API_PROTOCOLS,
  canEnterManualModel,
  PROVIDERS,
  defaultMode,
  fallsBackToManualModel,
  filterModels,
  modelFetchErrorText,
  providerEntry,
  showsModeTabs,
  singleModeReason,
  protocolBaseUrlHint,
  protocolLabel,
  validateConnectionName,
  type ApiProtocol,
  type ConnectMode,
  type ProviderId,
} from "@/lib/providerCatalog"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { useToast } from "@/components/ui/Toast"
import { Icon } from "@/icons"
import { Button, Dialog as UiDialog, Tabs, Tag } from "@/ui"
import type { Connection, PlatformId } from "@/types"

export interface ConnectActions {
  connections: readonly Connection[]
  readLocal: (platform: string, kind: "auth" | "api", name?: string, edition?: string) => Promise<{ added: number; existing: number; warnings?: string[] }>
  add: (input: { platform: string; kind: "auth" | "api"; name: string; secret?: string | null; base_url?: string | null; model?: string | null; service?: string | null; protocol?: ApiProtocol | null }) => Promise<void>
}

/**
 * 新建连接弹窗（Jolu.pen 画板 11 订阅 / 13 仅订阅 / 15–17 API Key）。
 * 订阅：检测本机登录 → 显示账号 → 保存；API Key：填写地址与密钥 → 获取模型 → 选择 → 保存（后端验证后入库）。
 * 自定义供应商（D-11）复用 API Key 结构，多一个「协议」选择；模型接口不可用时改为手填模型 ID，
 * 后端以「获取模型成功」或「手填模型」作为接口可达的依据，不查余额。
 */
export function ConnectDialog({ platform: initialPlatform, actions, onClose, onSaved }: {
  platform: ProviderId
  actions: ConnectActions
  onClose: () => void
  onSaved?: () => void
}) {
  const toast = useToast()
  const [provider, setProvider] = useState(initialPlatform)
  const entry = providerEntry(provider)
  const platform = entry.platformId
  const [service,setService]=useState<string>("opencode-go")
  const [mode, setMode] = useState<ConnectMode>(initialPlatform==="opencode"?"apiKey":defaultMode(entry))
  const [name, setName] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const savingRef = useRef(false)
  // 订阅
  const [detecting, setDetecting] = useState(false)
  const [detection, setDetection] = useState<LoginDetectionDto | null>(null)
  const edition = entry.edition ?? "default"
  const loginRequest = useRef(0)
  const browser = useBrowserAuthorization(platform, edition, mode === "subscription")
  const actualDetection = browser.detection ?? detection
  // API Key
  const [baseUrl, setBaseUrl] = useState(entry.baseUrl?.value ?? "")
  const [secret, setSecret] = useState("")
  const [secretVisible, setSecretVisible] = useState(false)
  const [models, setModels] = useState<string[]>([])
  const [model, setModel] = useState("")
  const [protocol, setProtocol] = useState<ApiProtocol>(entry.baseUrlChoices?.[0].protocol ?? "openai")
  const [fetching, setFetching] = useState(false)
  /** 每次地址 / 密钥变化或重新获取都递增；旧请求回来时直接丢弃，防止串到新地址 */
  const modelRequest = useRef(0)
  const [pendingPlatform, setPendingPlatform] = useState<ProviderId | null>(null)
  /** 模型列表为空或接口不支持枚举时，改为手动填写模型 ID（画板 27） */
  const [manualModel, setManualModel] = useState(false)
  const [modelError, setModelError] = useState<string | null>(null)
  const [askVerify, setAskVerify] = useState(false)
  const [verifying, setVerifying] = useState(false)
  const verifyingRef = useRef(false)
  const [verifyResult, setVerifyResult] = useState<{ ok: boolean; message: string } | null>(null)
  const [confirmDiscard, setConfirmDiscard] = useState(false)

  // 对话框卸载后，正在返回的获取 / 验证结果不再写入草稿。
  useEffect(() => () => { modelRequest.current++ }, [])

  const resetVerification = () => {
    verifyingRef.current = false
    setVerifying(false)
    setAskVerify(false)
    setVerifyResult(null)
  }

  const custom = platform === "custom"
  const tabs = platform!=="opencode" && showsModeTabs(entry)
  const reason = singleModeReason(entry)
  const kind = mode === "subscription" ? "auth" : "api"
  const hasDraft = Boolean(name.trim() || secret.trim() || actualDetection || browser.pending)

  const switchPlatform = (next: ProviderId) => {
    if (savingRef.current) return
    const nextEntry = providerEntry(next)
    // 更换平台不携带原平台的密钥与检测结果（画板 29）
    modelRequest.current++
    setFetching(false)
    resetVerification()
    setProvider(next)
    ++loginRequest.current
    setDetecting(false)
    browser.cancel()
    setMode(next==="opencode"?"apiKey":defaultMode(nextEntry))
    setService("opencode-go")
    setName("")
    setDetection(null)
    setBaseUrl(nextEntry.baseUrl?.value ?? "")
    setSecret("")
    setSecretVisible(false)
    setModels([])
    setModel("")
    setProtocol(nextEntry.baseUrlChoices?.[0].protocol ?? "openai")
    setManualModel(false)
    setModelError(null)
    setError(null)
    setPendingPlatform(null)
  }

  const detect = async () => {
    if (detecting || savingRef.current) return
    const request = ++loginRequest.current
    browser.cancel()
    setDetecting(true)
    setError(null)
    try {
      const result = await api.detectLocalLogin(platform, entry.edition)
      if (request === loginRequest.current) setDetection(result)
    } catch (reason) {
      if (request === loginRequest.current) { setDetection(null); setError(`检测失败：${String(reason)}`) }
    } finally {
      if (request === loginRequest.current) setDetecting(false)
    }
  }

  const invalidateModels = () => {
    modelRequest.current++
    resetVerification()
    setModels([])
    if (!manualModel) setModel("")
    setModelError(null)
    setFetching(false)
  }

  const cancelFetch = () => {
    modelRequest.current++
    setFetching(false)
  }

  const fetchModels = async () => {
    const request = ++modelRequest.current
    resetVerification()
    setFetching(true)
    setModelError(null)
    try {
      const list = await api.fetchRemoteModels(platform, baseUrl.trim(), secret.trim(), custom || platform === "zcode" ? protocol : null)
      if (request !== modelRequest.current) return
      setModels(list)
      setManualModel(false)
      // 空列表或新列表不含原选择时不自动选中（画板 29）
      setModel((current) => (list.includes(current) ? current : ""))
      toast.success(`已获取 ${list.length} 个模型`)
    } catch (reason) {
      if (request !== modelRequest.current) return
      // 失败就地说明，已填写内容保留（画板 27）
      setModelError(String(reason))
      // 模型接口不可用时改为手填（画板 27）；密钥被拒时不改。ZCode 手填后还须验证模型。
      if ((custom || platform === "zcode") && !manualModel && fallsBackToManualModel(String(reason))) {
        setManualModel(true)
        setModel("")
      }
    } finally {
      if (request === modelRequest.current) setFetching(false)
    }
  }

  const verifyModel = async () => {
    if (verifyingRef.current || fetching || !model.trim() || !secret.trim() || !baseUrl.trim()) return
    const request = ++modelRequest.current
    verifyingRef.current = true
    setVerifying(true)
    setAskVerify(false)
    setVerifyResult(null)
    try {
      await api.verifyCustomModel(baseUrl.trim(), secret.trim(), protocol, model.trim())
      if (request === modelRequest.current) {
        setVerifyResult({ ok: true, message: "验证通过：服务已接受该模型的请求。" })
        setModelError(null)
      }
    } catch (reason) {
      if (request === modelRequest.current) setVerifyResult({ ok: false, message: String(reason) })
    } finally {
      if (request === modelRequest.current) {
        verifyingRef.current = false
        setVerifying(false)
      }
    }
  }

  const nameError = mode === "apiKey" ? validateConnectionName(name, actions.connections, provider, kind) : null
  const urlError = baseUrl.trim() && !/^https?:\/\/[^\s/]+/i.test(baseUrl.trim()) ? "请输入完整地址，例如 https://api.example.com/v1" : null
  const canSave = !saving && !nameError && (mode === "subscription"
    ? Boolean(actualDetection?.found) && !browser.pending && !detecting
    : Boolean(baseUrl.trim() && !urlError && secret.trim() && model.trim())
      && (platform !== "zcode" || (!fetching && !verifying && (!manualModel || verifyResult?.ok === true))))

  const save = async () => {
    if (!canSave || savingRef.current) return
    savingRef.current = true
    setSaving(true)
    setError(null)
    try {
      if (mode === "subscription") {
        if (browser.detection?.found) {
          await browser.save("")
          onSaved?.()
          toast.success(`已连接 ${entry.name}`, "已保存账号，可继续添加同一供应商的其他账号")
          onClose()
          return
        }
        const summary = await actions.readLocal(platform, "auth", undefined, entry.edition)
        if (!summary.added && !summary.existing) throw new Error("未读取到可保存的登录，请重新检测")
        toast.success(summary.added ? `已连接 ${entry.name}` : "该账号已连接，无需重复添加", summary.warnings?.join("；"))
      } else {
        await actions.add({ platform, service:platform==="opencode"?service:null, kind: "api", name: name.trim(), secret: secret.trim(), base_url: baseUrl.trim(), model: model.trim(), protocol: custom || platform === "zcode" ? protocol : null })
        toast.success(`已连接 ${entry.name}`, custom
          ? (manualModel ? "模型 ID 为手动填写，未经模型接口验证；余额将在账号管理中独立查询" : "已通过模型接口验证；余额将在账号管理中独立查询")
          : "接口可达已验证；模型调用权限与额度分别展示")
      }
      onSaved?.()
      onClose()
    } catch (reason) {
      // 失败保留输入（画板 29）
      setError(String(reason))
    } finally {
      savingRef.current = false
      setSaving(false)
    }
  }

  return (
    <UiDialog
      open
      onOpenChange={(open) => {
        if (open || savingRef.current) return
        if (hasDraft) setConfirmDiscard(true)
        else onClose()
      }}
      preventClose={saving}
      width="lg"
      title={custom ? "新建自定义供应商连接" : mode === "subscription" ? "新建订阅连接" : "新建 API Key 连接"}
      description={
        mode === "subscription"
          ? "读取官方客户端的当前登录，或在浏览器授权页切换账号；验证通过后即可保存。"
          : "填写地址与密钥后获取模型，再从列表选择；保存时后端会验证。"
      }
      actions={
        <>
          <Button kind="ghost" size="md" disabled={saving} onClick={() => (hasDraft ? setConfirmDiscard(true) : onClose())}>取消</Button>
          <Button kind="primary" size="md" icon="check" loading={saving} disabled={!canSave} onClick={() => void save()}>
            {saving ? (mode === "apiKey" ? "正在验证" : "正在保存") : "保存连接"}
          </Button>
        </>
      }
    >
      <fieldset disabled={saving} className="flex min-w-0 flex-col gap-3.5 border-0 p-0">
        <div className="flex h-10 items-center gap-2.5">
          <span className="grid size-8 place-items-center rounded-[10px] bg-jn-well"><PlatformLogo platform={provider} size={20} /></span>
          <span className="text-sm font-semibold text-jn-text">{entry.name}</span>
          <span className="flex-1" />
          {!tabs && !custom && <Tag>{mode === "subscription" ? "当前仅订阅" : "当前仅 API Key"}</Tag>}
          <PlatformSwitcher
            current={provider}
            onPick={(next) => {
              if (savingRef.current) return
              if (next === provider) return
              if (hasDraft) setPendingPlatform(next)
              else switchPlatform(next)
            }}
          />
        </div>
        {pendingPlatform && (
          <div role="alert" className="flex items-center gap-3 rounded-[14px] border border-jn-line bg-jn-well px-4 py-2.5 text-xs text-jn-text">
            <span className="min-w-0 flex-1">更换为 {providerEntry(pendingPlatform).name} 会清空已填写的内容与检测结果。</span>
            <Button size="sm" kind="ghost" disabled={saving} onClick={() => setPendingPlatform(null)}>继续填写</Button>
            <Button size="sm" kind="soft" disabled={saving} onClick={() => switchPlatform(pendingPlatform)}>更换平台</Button>
          </div>
        )}

        {tabs ? (
          <Tabs
            style="segment"
            className="self-start"
            value={mode}
            onChange={(value) => {
              if (!savingRef.current && value !== mode) {
                invalidateModels()
                setMode(value)
                setError(null)
              }
            }}
            items={[
              { key: "subscription", label: "订阅" },
              { key: "apiKey", label: "API Key" },
            ]}
          />
        ) : reason && <p className="text-xs text-jn-text-3">{reason}</p>}

            {mode === "subscription" ? (
              <Field label="账号" hint={actualDetection?.found ? "账号来自本机登录或官方授权，保存时自动识别。" : "读取本地登录或完成浏览器授权后显示账号。"}>
                {(id) => <TextInput id={id} value={actualDetection?.account ?? ""} onChange={() => {}} readOnly placeholder={actualDetection?.found ? "未获取账号标识" : "尚未获取账号"} />}
              </Field>
            ) : (
              <Field label={custom ? "名称 *" : "连接名称 *"} hint="用于区分多个账号或服务，不影响供应商配置。" error={name && nameError ? nameError : null}>
                {(id) => <TextInput id={id} value={name} onChange={setName} placeholder={custom ? "例如：公司内部网关" : "例如：工作 API"} />}
              </Field>
            )}

            {platform==="opencode" && <label className="text-xs text-jn-text">模型服务<SelectField width="full" label="模型服务" className="mt-2 h-10" value={service} options={OPENCODE_SERVICES.map(s=>({value:s.id,label:s.label}))} onValueChange={value=>{const selected=OPENCODE_SERVICES.find(s=>s.id===value)!;setService(selected.id);setMode(selected.mode);setBaseUrl(selected.baseUrl);setSecret("");setDetection(null);invalidateModels()}} /></label>}
            {mode === "subscription" ? (
              <>
              <SubscriptionSection
                platform={platform}
                providerName={entry.name}
                detecting={detecting || browser.pending}
                detection={actualDetection}
                onDetect={() => void detect()}
                browserPending={browser.pending}
                browserMessage={browser.message}
                browserSupported={browser.supported}
                browserReady={!!browser.detection?.found}
                onBrowser={() => { if (savingRef.current) return; ++loginRequest.current; setDetection(null); setError(null); void browser.start() }}
                tabs={tabs}
              />
              </>
            ) : (
              <>
                {custom && (
                  <ProtocolPicker
                    value={protocol}
                    onChange={(next) => {
                      if (next === protocol) return
                      // 协议决定鉴权头，切换后已获取的模型作废
                      setProtocol(next)
                      invalidateModels()
                    }}
                  />
                )}
                {entry.baseUrlChoices && <label className="text-xs text-jn-text">服务区域与协议<SelectField width="full" label="服务区域与协议" className="mt-2 h-10" value={baseUrl} options={entry.baseUrlChoices.map(c=>({value:c.value,label:c.label}))} onValueChange={value=>{const choice=entry.baseUrlChoices!.find(c=>c.value===value)!;setBaseUrl(choice.value);setProtocol(choice.protocol);invalidateModels()}} /></label>}
                <Field label="Base URL *" hint={entry.baseUrl?.locked ? "该服务限定官方地址，不开放编辑。" : undefined} error={urlError}>
                  {(id) => (
                    <TextInput
                      id={id}
                      value={baseUrl}
                      readOnly={platform==="opencode"?service!=="openai-compatible":entry.baseUrl?.locked}
                      onChange={(value) => { setBaseUrl(value); invalidateModels() }}
                      placeholder={`请输入完整地址，例如 ${custom ? protocolBaseUrlHint(protocol) : "https://api.example.com/v1"}`}
                      mono
                    />
                  )}
                </Field>
                <Field label="API Key *" hint="默认隐藏；点击眼睛临时显示。">
                  {(id) => (
                    <TextInput
                      id={id}
                      value={secret}
                      onChange={(value) => { setSecret(value); invalidateModels() }}
                      // 离开输入区后恢复遮罩（画板 29）
                      onBlur={() => setSecretVisible(false)}
                      type={secretVisible ? "text" : "password"}
                      placeholder="粘贴 API Key"
                      mono
                      trailing={
                        <button
                          type="button"
                          aria-label={secretVisible ? "隐藏 API Key" : "显示 API Key"}
                          onMouseDown={(event) => event.preventDefault()}
                          onClick={() => setSecretVisible((value) => !value)}
                          className="grid size-6 place-items-center rounded text-jn-text-3 hover:text-jn-text"
                        >
                          {secretVisible ? <Eye aria-hidden className="size-4" /> : <EyeOff aria-hidden className="size-4" />}
                        </button>
                      }
                    />
                  )}
                </Field>
                <div className="flex flex-col gap-2">
                  {custom && manualModel && modelError && !fetching && (
                    <div role="alert" className="flex items-center gap-2.5 rounded-lg bg-jn-danger-soft px-2.5 py-2 text-[12px] text-jn-danger">
                      <CircleAlert aria-hidden className="size-3.5 shrink-0" />
                      <span className="min-w-0 flex-1 break-all">{modelFetchErrorText(modelError)}</span>
                    </div>
                  )}
                  <span className="text-xs font-medium text-jn-text">{manualModel ? "模型 ID *" : "默认模型 *"}</span>
                  <div className="flex gap-2">
                    {manualModel ? (
                      <div className="min-w-0 flex-1">
                        <TextInput id="manual-model" value={model} disabled={fetching || verifying} onChange={(value) => { modelRequest.current++; resetVerification(); setModel(value) }} placeholder="输入服务支持的完整模型 ID" mono />
                      </div>
                    ) : (
                      <ModelPicker models={models} value={model} onChange={setModel} />
                    )}
                    {fetching ? (
                      <Button kind="ghost" size="md" onClick={cancelFetch}>取消获取</Button>
                    ) : (
                      <Button
                        kind="soft"
                        size="md"
                        icon="refresh"
                        disabled={verifying || !baseUrl.trim() || Boolean(urlError) || !secret.trim() || !isTauri}
                        onClick={() => void fetchModels()}
                      >
                        {custom ? (models.length ? "重新获取" : "获取模型") : models.length || modelError ? "重新获取" : "获取模型"}
                      </Button>
                    )}
                    {platform === "zcode" && manualModel && (
                      <Button kind="ghost" size="md" icon="success" loading={verifying} disabled={fetching || verifying || !model.trim() || !secret.trim() || !isTauri} onClick={() => setAskVerify(true)}>
                        {verifying ? "正在验证" : "验证模型"}
                      </Button>
                    )}
                  </div>
                  {platform === "zcode" && manualModel && (
                    <>
                      {askVerify && (
                        <div role="alertdialog" aria-label="验证模型" className="flex items-center gap-3 rounded-[14px] border border-jn-line bg-jn-well px-4 py-2.5 text-xs text-jn-text">
                          <span className="min-w-0 flex-1">验证会向服务发送一次最小请求，可能产生少量 API 用量。</span>
                          <Button size="sm" kind="ghost" onClick={() => setAskVerify(false)}>取消</Button>
                          <Button size="sm" kind="soft" disabled={fetching || verifying} onClick={() => void verifyModel()}>继续验证</Button>
                        </div>
                      )}
                      <span role={verifyResult?.ok === false ? "alert" : "status"} className={cn("flex items-center gap-1.5 text-[12px]", verifyResult ? verifyResult.ok ? "text-jn-primary-ink" : "text-jn-danger" : "text-jn-text-3")}>
                        {verifyResult?.ok && <CircleCheck aria-hidden className="size-3.5 shrink-0" />}
                        {verifying ? "正在验证模型，请等待服务返回…" : verifyResult?.message ?? "未验证 · 验证模型通过后才能保存连接。"}
                      </span>
                    </>
                  )}
                  {fetching ? (
                    <span role="status" className="flex items-center gap-1.5 text-[12px] text-jn-text-3">
                      <LoaderCircle aria-hidden className="size-3 animate-spin" />正在获取可用模型…等待接口返回，请勿重复提交。
                    </span>
                  ) : modelError && !(custom && manualModel) ? (
                    <div role="alert" className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px]">
                      <span className="text-jn-danger">获取失败：{modelError}</span>
                      {!manualModel && canEnterManualModel(platform, modelError) && (
                        <button type="button" onClick={() => { setManualModel(true); setModel("") }} className="font-medium text-jn-primary-ink hover:underline">
                          手动填写模型
                        </button>
                      )}
                    </div>
                  ) : (
                    <span className="text-[12px] text-jn-text-3">
                      {custom
                        ? (manualModel ? "手填模型后即可保存，作为接口可达的依据；可再次点击「获取模型」改回从列表选择。" : "填写地址与密钥后点击「获取模型」，再从下拉框选择；模型接口不可用时改为手填模型 ID。")
                        : manualModel ? platform === "zcode" ? "模型接口不支持枚举时手动填写；验证当前模型后再保存。" : "模型接口不支持枚举时手动填写；保存时会验证地址与密钥。" : "填写地址与密钥后点击「获取模型」，再从下拉框选择；修改后需重新获取。"}
                    </span>
                  )}
                </div>
              </>
            )}
          {(error || browser.error) && <p role="alert" className="text-xs text-jn-danger">{error || browser.error}</p>}
      </fieldset>
      {confirmDiscard && (
        <div role="alertdialog" aria-label="放弃未保存的更改？" className="flex items-center gap-3 rounded-[14px] border border-jn-line bg-jn-well px-4 py-3 text-xs text-jn-text">
          <span className="min-w-0 flex-1"><b className="font-semibold">放弃未保存的更改？</b> 已填写的内容不会保存。</span>
          <Button size="sm" kind="ghost" disabled={saving} onClick={() => setConfirmDiscard(false)}>继续编辑</Button>
          <Button size="sm" kind="danger" disabled={saving} onClick={onClose}>放弃更改</Button>
        </div>
      )}
    </UiDialog>
  )
}

function SubscriptionSection({ providerName, detecting, detection, onDetect, tabs, browserPending, browserMessage, browserSupported, browserReady, onBrowser }: {
  platform: PlatformId
  providerName: string
  detecting: boolean
  detection: LoginDetectionDto | null
  onDetect: () => void
  tabs: boolean
  browserPending: boolean
  browserMessage: string | null
  browserSupported: boolean
  browserReady: boolean
  onBrowser: () => void
}) {
  const detectButton = (
    <Button kind="primary" size="md" icon="refresh" loading={detecting} disabled={detecting || !isTauri} onClick={onDetect}>
      读取本地登录信息
    </Button>
  )
  const browserButton = (
    <Button aria-label="浏览器授权" kind="soft" size="md" icon="external" loading={browserPending} disabled={detecting || browserPending || !browserSupported || !isTauri} onClick={onBrowser}>
      浏览器授权
    </Button>
  )

  if (detection?.found) {
    const account = detection.account ?? (detection.sources.length ? detection.sources.join("、") : "未获取账号")
    return (
      <>
        <div className="flex flex-col gap-3 rounded-[16px] bg-jn-primary-wash p-5">
          <span className="flex items-center gap-2 text-[13px] font-semibold text-jn-primary-ink">
            <Icon name="success" size="md" />
            {browserReady ? "浏览器账号已验证" : detection.sources.length && !detection.account ? "已检测到本机来源" : "已检测到账号"}
          </span>
          <div className="flex flex-wrap items-center gap-3">
            <span className="break-all text-base font-bold text-jn-text">{account}</span>
            {detection.plan && <Tag>{detection.plan}</Tag>}
          </div>
          <div className="flex gap-6 text-xs">
            <Info label="凭证标识" value={detection.masked ?? "未获取"} />
            <Info label="套餐" value={detection.plan ?? "未获取"} />
          </div>
          <span className="text-xs text-jn-text-3">{browserReady ? "账号已通过官方身份验证；套餐与额度按平台实际能力显示。" : "账号与套餐以本机客户端记录为准，未记录时显示「未获取」。"}</span>
          {browserReady && detection.message && <span className="text-xs leading-relaxed text-jn-text-3">{detection.message}</span>}
        </div>
        <div className="flex flex-wrap gap-2.5">
          {detectButton}
          {browserButton}
        </div>
        {!tabs && <p className="text-xs text-jn-text-3">只提供订阅连接</p>}
      </>
    )
  }

  return (
    <>
      <div className="flex flex-col gap-3.5 rounded-[16px] bg-jn-well p-5">
        <Icon name="connect" size="lg" className="text-jn-primary-ink" />
        <span className="text-base font-bold text-jn-text">使用 {providerName} 账号连接</span>
        <span className="text-xs leading-relaxed text-jn-text-3">{browserSupported ? "读取官方客户端的当前登录，或在浏览器授权页切换账号，分别保存多个账号。" : "读取官方客户端的本地登录；此供应商的浏览器授权待接入。"}</span>
        <div className="flex flex-wrap gap-2.5">
          {detectButton}
          {browserButton}
        </div>
      </div>
      <div className="flex items-center gap-3 rounded-[16px] border border-jn-line p-4">
        <Icon name="loading" size="md" className={cn("shrink-0 text-jn-text-3", detecting && "animate-spin text-jn-primary")} />
        <div className="flex min-w-0 flex-col gap-1">
          <span className="text-[13px] font-medium text-jn-text">{browserPending ? "等待浏览器授权与额度验证" : detecting ? "正在读取本地登录" : "尚未获取有效登录"}</span>
          <span role="status" className="text-xs text-jn-text-3">{browserPending ? browserMessage ?? "请在系统默认浏览器完成官方登录，验证成功后会自动返回。" : detection?.message ?? "读取或授权成功后才能保存连接。"}</span>
        </div>
      </div>
    </>
  )
}

/** 自定义供应商的接口协议（D-11）：OpenAI 兼容用 Bearer，Anthropic 兼容用 x-api-key */
function ProtocolPicker({ value, onChange }: { value: ApiProtocol; onChange: (protocol: ApiProtocol) => void }) {
  const labelId = useId()
  return (
    <div className="flex flex-col gap-[7px]">
      <span id={labelId} className="text-xs font-medium text-jn-text">协议 *</span>
      <div role="radiogroup" aria-labelledby={labelId} className="inline-flex items-center gap-0.5 rounded-full bg-jn-well p-[3px] h-[38px] w-full">
        {API_PROTOCOLS.map((item) => (
          <button
            key={item}
            type="button"
            role="radio"
            aria-checked={value === item}
            onClick={() => onChange(item)}
            className={cn(
              "jn-interactive inline-flex h-[30px] flex-1 items-center justify-center gap-1.5 rounded-full text-[13px] font-semibold",
              value === item ? "bg-jn-surface text-jn-text" : "text-jn-text-2 hover:text-jn-text",
            )}
          >
            {protocolLabel(item)}
          </button>
        ))}
      </div>
    </div>
  )
}

function Info({ label, value }: { label: string; value: string }) {
  return (
    <span className="flex min-w-0 flex-1 gap-2">
      <span className="shrink-0 text-jn-text-3">{label}</span>
      <span className="truncate font-medium text-jn-text">{value}</span>
    </span>
  )
}

/** 「更换平台」：列出全部供应商，待接入的不可选 */
function PlatformSwitcher({ current, onPick }: { current: ProviderId; onPick: (platform: ProviderId) => void }) {
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild>
        <Button kind="ghost" size="sm">更换平台</Button>
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="end" sideOffset={6} collisionPadding={12} style={{ maxHeight: "var(--radix-dropdown-menu-content-available-height)" }} className="jn-popover z-[160] w-[220px] overflow-y-auto rounded-[16px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface">
          {PROVIDERS.map((item) => {
            const pending = item.subscription !== "ready" && item.apiKey !== "ready"
            return (
              <Menu.Item
                key={item.id}
                disabled={pending}
                onSelect={() => onPick(item.id)}
                className="flex h-8 cursor-pointer select-none items-center gap-2 rounded-[10px] px-2.5 text-xs text-jn-text outline-none data-[highlighted]:bg-jn-well data-[disabled]:opacity-45"
              >
                <PlatformLogo platform={item.id} />
                <span className="min-w-0 flex-1">{item.name}</span>
                {pending ? <span className="text-[11px] text-jn-text-3">待接入</span> : item.id === current && <Icon name="check" size="xs" className="text-jn-primary-ink" />}
              </Menu.Item>
            )
          })}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

/** 模型下拉（画板 17）：可搜索，支持方向键与回车选择；空列表不自动选中 */
export function ModelPicker({ models, value, onChange }: { models: string[]; value: string; onChange: (model: string) => void }) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState("")
  const [active, setActive] = useState(0)
  const rootRef = useRef<HTMLDivElement>(null)
  const listId = useId()
  const visible = filterModels(models, query)

  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener("pointerdown", onPointerDown)
    return () => document.removeEventListener("pointerdown", onPointerDown)
  }, [open])

  useEffect(() => { setActive(Math.max(0, visible.indexOf(value))) }, [query, open]) // eslint-disable-line react-hooks/exhaustive-deps

  const pick = (model: string) => {
    onChange(model)
    setOpen(false)
    setQuery("")
  }

  return (
    <div ref={rootRef} className="relative min-w-0 flex-1">
      <button
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listId}
        disabled={!models.length}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex h-10 w-full items-center gap-2.5 rounded-lg border bg-jn-stage px-3 text-left text-[13px] outline-none focus-visible:border-jn-primary",
          open ? "border-jn-primary" : "border-jn-line",
          "disabled:cursor-not-allowed",
        )}
      >
        <span className={cn("min-w-0 flex-1 truncate", value ? "font-mono text-jn-text" : "text-jn-text-3")}>
          {value || (models.length ? "选择模型" : "请先获取模型")}
        </span>
        <ChevronDown aria-hidden className={cn("size-4 shrink-0 text-jn-text-3 transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <div className="motion-popover absolute inset-x-0 top-full z-10 mt-1.5 flex flex-col gap-1.5 rounded-xl border border-jn-line bg-jn-surface p-1.5 shadow-popover">
          <label className="flex h-8 items-center gap-2 rounded-md bg-jn-surface px-2">
            <Search aria-hidden className="size-3.5 shrink-0 text-jn-text-3" />
            <input
              autoFocus
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "ArrowDown") { event.preventDefault(); setActive((i) => Math.min(i + 1, visible.length - 1)) }
                else if (event.key === "ArrowUp") { event.preventDefault(); setActive((i) => Math.max(i - 1, 0)) }
                else if (event.key === "Enter" && visible[active]) { event.preventDefault(); pick(visible[active]) }
                else if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setOpen(false) }
              }}
              placeholder="搜索模型"
              aria-label="搜索模型"
              aria-activedescendant={visible[active] ? `${listId}-${active}` : undefined}
              className="min-w-0 flex-1 bg-transparent text-[12px] text-jn-text outline-none placeholder:text-jn-text-3"
            />
          </label>
          <ul id={listId} role="listbox" aria-label="模型" className="max-h-[196px] overflow-y-auto">
            {visible.map((item, index) => (
              <li
                key={item}
                id={`${listId}-${index}`}
                role="option"
                aria-selected={item === value}
                onMouseEnter={() => setActive(index)}
                onClick={() => pick(item)}
                className={cn(
                  "flex min-h-[34px] cursor-default items-center gap-2 rounded-[7px] px-2 py-1.5 font-mono text-xs",
                  item === value ? "bg-jn-primary-wash text-jn-primary-ink" : index === active ? "bg-jn-surface text-jn-text" : "text-jn-text",
                )}
              >
                <span className="min-w-0 flex-1 truncate">{item}</span>
                {item === value && <Check aria-hidden className="size-[15px] shrink-0" />}
              </li>
            ))}
            {!visible.length && <li className="px-2 py-1.5 text-[12px] text-jn-text-3">没有匹配的模型</li>}
          </ul>
        </div>
      )}
    </div>
  )
}

function Field({ label, hint, error, children }: { label: string; hint?: string; error?: string | null; children: (id: string) => ReactNode }) {
  const id = useId()
  return (
    <div className="flex flex-col gap-[7px]">
      <label htmlFor={id} className="text-xs font-medium text-jn-text">{label}</label>
      {children(id)}
      {error ? <span className="text-[12px] text-jn-danger">{error}</span> : hint && <span className="text-[12px] text-jn-text-3">{hint}</span>}
    </div>
  )
}

function TextInput({ id, value, onChange, onBlur, placeholder, type = "text", readOnly, disabled, mono, trailing }: {
  id: string
  value: string
  onChange: (value: string) => void
  onBlur?: () => void
  placeholder?: string
  type?: "text" | "password"
  readOnly?: boolean
  disabled?: boolean
  mono?: boolean
  trailing?: ReactNode
}) {
  return (
    <div className={cn(
      "flex h-10 items-center gap-2.5 rounded-lg border border-jn-line px-3 focus-within:border-jn-primary",
      readOnly ? "bg-jn-surface" : "bg-jn-stage",
    )}>
      <input
        id={id}
        type={type}
        value={value}
        readOnly={readOnly}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        onBlur={onBlur}
        placeholder={placeholder}
        spellCheck={false}
        autoComplete="off"
        className={cn("min-w-0 flex-1 bg-transparent text-[13px] text-jn-text outline-none placeholder:text-jn-text-3", mono && "font-mono", readOnly && "text-jn-text-3")}
      />
      {trailing}
    </div>
  )
}
