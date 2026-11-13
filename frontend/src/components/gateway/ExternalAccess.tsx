import { useEffect, useState } from "react"
import { Copy, Network, Pencil } from "lucide-react"
import { cn } from "@/lib/utils"
import { gatewayApi } from "@/lib/gatewayApi"
import { isTauri } from "@/lib/api"
import { useToast } from "@/components/ui/Toast"
import { SharedAccessDialog } from "./SharedAccessDialog"

type Protocol = "openai" | "anthropic" | "gemini"
type Shell = "powershell" | "shell" | "curl" | "python" | "node"

/**
 * 模型路由 · 对外访问（画板 V5-4）：五种 API 一个地址；仅本机 / 局域网共享切换、地址卡、
 * 客户端 Key（沿用局域网共享弹窗的内容）、接入示例。改端口还没有命令，按钮置灰标「待确认」。
 */
export function ExternalAccess({ port, shared, busy, onSharedChange, run, models }: {
  port: number
  shared: boolean
  busy: boolean
  onSharedChange: (on: boolean) => void
  run: (work: () => Promise<unknown>, successTitle?: string) => Promise<boolean>
  models: string[]
}) {
  const toast = useToast()
  const [lan, setLan] = useState<string | null>(null)
  const [protocol, setProtocol] = useState<Protocol>("anthropic")
  const [shell, setShell] = useState<Shell>("powershell")
  const [model, setModel] = useState("")
  useEffect(() => {
    if (isTauri) void gatewayApi.lanAddress().then(setLan).catch(() => setLan(null))
  }, [])
  const local = `http://127.0.0.1:${port}`
  const lanUrl = lan ? `http://${lan}:${port}` : null
  const base = shared && lanUrl ? lanUrl : local
  const chosen = model || models[0] || "group/你的分组"
  const copy = (text: string) => {
    void navigator.clipboard.writeText(text).then(() => toast.success("已复制")).catch(() => toast.danger("复制失败", "请手动选中复制"))
  }
  const code = example(protocol, shell, base, chosen, shared)
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-7 py-5">
      <div className="flex items-center gap-3">
        <span className="flex size-10 shrink-0 items-center justify-center rounded-[10px] bg-jolu-side"><Network aria-hidden className="size-4 text-jolu-muted" /></span>
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="text-[17px] font-semibold text-jolu-text">对外访问</span>
          <span className="truncate text-meta text-jolu-muted">五种 API，一个地址：OpenAI Chat / Responses、Anthropic Messages、Gemini 原生</span>
        </span>
        <div className="flex items-center gap-0.5 rounded-lg bg-jolu-track p-0.5" role="group" aria-label="共享范围">
          {([false, true] as const).map((on) => (
            <button key={String(on)} type="button" aria-pressed={shared === on} disabled={busy} onClick={() => shared !== on && onSharedChange(on)}
              className={cn("h-7 rounded-md px-3 text-meta outline-none", shared === on ? "bg-jolu-bg font-medium text-jolu-text shadow-sm" : "text-jolu-muted hover:text-jolu-text")}>
              {on ? "局域网共享" : "仅本机"}
            </button>
          ))}
        </div>
      </div>

      <section className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
        <header className="flex items-baseline gap-2 border-b border-jolu-table-divider bg-jolu-side px-4 py-2.5">
          <span className="text-[13px] font-semibold text-jolu-text">地址</span>
          <span className="text-meta text-jolu-muted">本机回环不需要 Key；局域网请求必须带一个启用的客户端 Key</span>
        </header>
        <AddressRow label="端口" value={String(port)} note="改端口后，已接入的 Agent 会一起改写"
          action={<RowButton icon={Pencil} label="修改（待确认）" disabled onClick={() => undefined} />} />
        <AddressRow label="本机" value={local} mono action={<RowButton icon={Copy} label="复制" onClick={() => copy(local)} />} />
        <AddressRow label="局域网" value={lanUrl ?? "没有检测到局域网地址"} mono={!!lanUrl} note={shared ? undefined : "开启局域网共享后可用"}
          action={lanUrl ? <RowButton icon={Copy} label="复制" onClick={() => copy(lanUrl)} /> : null} />
      </section>

      <SharedAccessDialog inline shared={shared} busy={busy} onSharedChange={onSharedChange} onClose={() => undefined} run={run} />

      <section className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
        <header className="flex items-center gap-2 border-b border-jolu-table-divider bg-jolu-side px-4 py-2.5">
          <span className="text-[13px] font-semibold text-jolu-text">接入示例</span>
          <span className="min-w-0 flex-1 truncate text-meta text-jolu-muted">点一个模型，示例跟着变</span>
          <div className="flex items-center gap-0.5 rounded-lg bg-jolu-track p-0.5" role="group" aria-label="协议">
            {(["openai", "anthropic", "gemini"] as const).map((value) => (
              <button key={value} type="button" aria-pressed={protocol === value} onClick={() => setProtocol(value)}
                className={cn("h-6 rounded-md px-2.5 text-note outline-none", protocol === value ? "bg-jolu-bg font-medium text-jolu-text shadow-sm" : "text-jolu-muted")}>
                {value === "openai" ? "OpenAI" : value === "anthropic" ? "Anthropic" : "Gemini"}
              </button>
            ))}
          </div>
        </header>
        <div className="flex flex-wrap items-center gap-3 px-4 pt-3">
          {(["powershell", "shell", "curl", "python", "node"] as const).map((value) => (
            <button key={value} type="button" aria-pressed={shell === value} onClick={() => setShell(value)}
              className={cn("text-meta outline-none", shell === value ? "font-semibold text-jolu-on" : "text-jolu-muted hover:text-jolu-text")}>
              {SHELL_LABELS[value]}
            </button>
          ))}
          <span className="flex-1" />
          <select aria-label="示例模型" value={model} onChange={(event) => setModel(event.target.value)}
            className="h-7 max-w-[220px] rounded-md border border-jolu-border bg-jolu-bg px-2 text-note text-jolu-text outline-none">
            {models.length === 0 && <option value="">（模型池为空）</option>}
            {models.map((entry) => <option key={entry} value={entry}>{entry}</option>)}
          </select>
        </div>
        <div className="relative m-4 mt-3 rounded-lg bg-jolu-side px-3.5 py-3">
          <pre className="overflow-x-auto whitespace-pre font-mono text-note leading-5 text-jolu-text">{code}</pre>
          <button type="button" aria-label="复制示例" onClick={() => copy(code)} className="absolute right-2 top-2 flex size-7 items-center justify-center rounded-md hover:bg-jolu-stage">
            <Copy aria-hidden className="size-3.5 text-jolu-muted" />
          </button>
        </div>
      </section>
    </div>
  )
}

const SHELL_LABELS: Record<Shell, string> = { powershell: "PowerShell", shell: "Shell", curl: "curl", python: "Python", node: "Node" }

function AddressRow({ label, value, note, mono, action }: { label: string; value: string; note?: string; mono?: boolean; action: React.ReactNode }) {
  return (
    <div className="flex min-h-11 items-center gap-3 border-t border-jolu-table-divider px-4 py-2 first-of-type:border-t-0">
      <span className="w-16 shrink-0 text-meta text-jolu-muted">{label}</span>
      <span className={cn("min-w-0 truncate text-meta text-jolu-text", mono && "font-mono")}>{value}</span>
      {note && <span className="min-w-0 truncate text-note text-jolu-muted">{note}</span>}
      <span className="flex-1" />
      {action}
    </div>
  )
}

function RowButton({ icon: Icon, label, disabled, onClick }: { icon: React.ElementType; label: string; disabled?: boolean; onClick: () => void }) {
  return (
    <button type="button" disabled={disabled} onClick={onClick}
      className="flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-jolu-border px-2.5 text-meta text-jolu-text outline-none hover:bg-jolu-side disabled:opacity-50">
      <Icon aria-hidden className="size-3 text-jolu-muted" />
      {label}
    </button>
  )
}

/** 接入示例：Key 一律写占位（局域网换成你的客户端 Key），不把任何真实 Key 放进页面。 */
export function example(protocol: Protocol, shell: Shell, base: string, model: string, shared: boolean): string {
  const key = shared ? "<你的客户端 Key>" : "jolu-local"
  const path = protocol === "openai" ? "/v1/chat/completions" : protocol === "anthropic" ? "/v1/messages" : `/v1beta/models/${model}:generateContent`
  const body = protocol === "gemini"
    ? `{"contents":[{"parts":[{"text":"你好"}]}]}`
    : protocol === "anthropic"
      ? `{"model":"${model}","max_tokens":256,"messages":[{"role":"user","content":"你好"}]}`
      : `{"model":"${model}","messages":[{"role":"user","content":"你好"}]}`
  const header = protocol === "anthropic" ? "x-api-key" : protocol === "gemini" ? "x-goog-api-key" : "Authorization"
  const value = protocol === "openai" ? `Bearer ${key}` : key
  switch (shell) {
    case "powershell":
      if (protocol === "anthropic") return `$env:ANTHROPIC_BASE_URL = "${base}"\n$env:ANTHROPIC_AUTH_TOKEN = "${key}"\nclaude --model ${model}`
      if (protocol === "openai") return `$env:OPENAI_BASE_URL = "${base}/v1"\n$env:OPENAI_API_KEY = "${key}"\ncodex -m ${model}`
      return `$env:GOOGLE_GEMINI_BASE_URL = "${base}"\n$env:GEMINI_API_KEY = "${key}"\ngemini -m ${model}`
    case "shell":
      if (protocol === "anthropic") return `export ANTHROPIC_BASE_URL="${base}"\nexport ANTHROPIC_AUTH_TOKEN="${key}"\nclaude --model ${model}`
      if (protocol === "openai") return `export OPENAI_BASE_URL="${base}/v1"\nexport OPENAI_API_KEY="${key}"\ncodex -m ${model}`
      return `export GOOGLE_GEMINI_BASE_URL="${base}"\nexport GEMINI_API_KEY="${key}"\ngemini -m ${model}`
    case "curl":
      return `curl ${base}${path} \\\n  -H "content-type: application/json" \\\n  -H "${header}: ${value}" \\\n  -d '${body}'`
    case "python":
      return `import requests\n\nr = requests.post(\n    "${base}${path}",\n    headers={"${header}": "${value}"},\n    json=${body},\n)\nprint(r.json())`
    case "node":
      return `const r = await fetch("${base}${path}", {\n  method: "POST",\n  headers: { "content-type": "application/json", "${header}": "${value}" },\n  body: JSON.stringify(${body}),\n})\nconsole.log(await r.json())`
  }
}
