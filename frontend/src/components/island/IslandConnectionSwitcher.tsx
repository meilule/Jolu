import { SelectField, type SelectOption } from "@/components/ui/SelectField"
import { useState } from "react"
import { Repeat2, Zap } from "lucide-react"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import type { Connection } from "@/types"

const FOLLOW_ACTIVE_VALUE = "__follow_active__"

const GROUPS = [
  { platform: "claude", kind: "auth", label: "Claude · Auth" },
  { platform: "claude", kind: "api", label: "Claude · API Key" },
  { platform: "codex", kind: "auth", label: "Codex · Auth" },
  { platform: "codex", kind: "api", label: "Codex · API Key" },
] as const

export function IslandConnectionSwitcher({
  connections = [],
  selectedId,
  followActive = false,
  loading = false,
  onSelect,
  onSelectFollowActive,
  onOpenChange,
}: {
  connections?: Connection[]
  selectedId?: string
  followActive?: boolean
  loading?: boolean
  onSelect?: (id: string) => Promise<unknown>
  onSelectFollowActive?: () => Promise<unknown>
  onOpenChange?: (open: boolean) => void
}) {
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const disabled = loading || pending || !onSelect
  const currentValue = followActive && onSelectFollowActive ? FOLLOW_ACTIVE_VALUE : (selectedId ?? "")

  return (
    <div className="flex shrink-0 flex-col items-end gap-1" onPointerDown={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()}>
      <SelectField
        label="切换连接"
        value={currentValue}
        disabled={disabled}
        onOpenChange={onOpenChange}
        className="h-7 bg-surface-3 text-[11px] font-medium"
        // 灵动岛的窗口很窄，触发器只有 142px；而最长选项「⚡ 智能跟随活跃 Agent」或「Claude · API Key（未配置）」
        // 在 11px 字号下约 186px，取 228px 留出余量。
        menuWidth={228}
        triggerLabel={
          <span className="inline-flex items-center gap-1">
            {followActive && onSelectFollowActive ? <Zap aria-hidden className="size-3 text-emerald-400" /> : <Repeat2 aria-hidden className="size-3" />}
            {pending ? "切换中…" : loading ? "读取连接…" : followActive && onSelectFollowActive ? "智能跟随中" : "切换连接"}
          </span>
        }
        options={[
          ...(onSelectFollowActive
            ? [
                {
                  value: FOLLOW_ACTIVE_VALUE,
                  label: "智能跟随活跃 Agent / 账号",
                  description: "自动跟随运行中的工具与本机切换的账号",
                  icon: <Zap aria-hidden className="size-3.5 text-emerald-400" />,
                },
              ]
            : !selectedId
              ? [{ value: "", label: "选择连接", disabled: true }]
              : []),
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
          if (!id || id === currentValue || disabled) return
          setPending(true)
          setError(null)
          try {
            if (id === FOLLOW_ACTIVE_VALUE && onSelectFollowActive) {
              await onSelectFollowActive()
            } else {
              await onSelect?.(id)
            }
          }
          catch { setError("切换失败，请重试") }
          finally { setPending(false) }
        }}
      />
      {error && <span role="alert" className="text-[10px] text-danger">{error}</span>}
    </div>
  )
}
