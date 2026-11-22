import { SelectField, type SelectOption } from "@/components/ui/SelectField"
import { useState } from "react"
import { Repeat2 } from "lucide-react"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import type { Connection } from "@/types"

const GROUPS = [
  { platform: "claude", kind: "auth", label: "Claude · Auth" },
  { platform: "claude", kind: "api", label: "Claude · API Key" },
  { platform: "codex", kind: "auth", label: "Codex · Auth" },
  { platform: "codex", kind: "api", label: "Codex · API Key" },
] as const

export function IslandConnectionSwitcher({
  connections = [], selectedId, loading = false, onSelect, onOpenChange,
}: {
  connections?: Connection[]
  selectedId?: string
  loading?: boolean
  onSelect?: (id: string) => Promise<unknown>
  onOpenChange?: (open: boolean) => void
}) {
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const disabled = loading || pending || !onSelect

  return (
    <div className="flex shrink-0 flex-col items-end gap-1" onPointerDown={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()}>
      <SelectField
        label="切换连接"
        value={selectedId ?? ""}
        disabled={disabled}
        onOpenChange={onOpenChange}
        className="h-7 bg-surface-3 text-[11px] font-medium"
        // 灵动岛的窗口很窄，触发器只有 142px；而最长选项「Claude · API Key（未配置）」
        // 在 11px 字号下约 182px（含 14px 图标与间距），200px 减去面板内边距只剩 176px，
        // 会压线折行。取 220px 留出余量。
        menuWidth={220}
        triggerLabel={<span className="inline-flex items-center gap-1"><Repeat2 aria-hidden className="size-3" />{pending ? "切换中…" : loading ? "读取连接…" : "切换连接"}</span>}
        options={[
          ...(!selectedId ? [{ value: "", label: "选择连接", disabled: true }] : []),
          ...GROUPS.flatMap((group): SelectOption[] => {
            const matches = connections.filter((connection) => connection.platformId === group.platform && connection.kind === group.kind)
            return matches.length ? matches.map((connection) => ({
              value: connection.id, label: `${group.label}${connection.status === "connected" ? "" : "（未连接）"}`,
              disabled: connection.status !== "connected",
              icon: <PlatformLogo platform={group.platform} size={14} />,
              description: matches.length > 1 ? connection.name : undefined,
            })) : [{ value: `missing:${group.label}`, label: `${group.label}（未配置）`, icon: <PlatformLogo platform={group.platform} size={14} />, disabled: true }]
          }),
        ]}
        onValueChange={async (id) => {
          if (!id || id === selectedId || disabled) return
          setPending(true)
          setError(null)
          try { await onSelect?.(id) }
          catch { setError("切换失败，请重试") }
          finally { setPending(false) }
        }}
      />
      {error && <span role="alert" className="text-[10px] text-danger">{error}</span>}
    </div>
  )
}
