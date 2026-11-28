import { useEffect, useState } from "react"
import { gatewayApi, type GatewayPortOutcomeDto } from "@/lib/gatewayApi"
import { GATEWAY_PORT_MAX, GATEWAY_PORT_MIN, parseGatewayPort, sanitizePortInput } from "@/lib/gatewayPort"
import { Banner, Button, Dialog, FormField, Input } from "@/ui"

export interface GatewayPortDialogProps {
  open: boolean
  currentPort: number | null
  wired: number
  /** 由启动失败触发时展示具体原因（如端口被占用或被 Windows 保留） */
  reason?: string | null
  /** 预填端口（例如原地编辑失败后保留输入） */
  initialPort?: string | null
  /** 打开时自动向后端获取一个可绑定的推荐端口并填入 */
  autoSuggest?: boolean
  /** 自定义确认按钮文案（例如自动重试场景下显示「保存并继续」） */
  confirmLabel?: string
  onOpenChange: (open: boolean) => void
  onSaved: (outcome: GatewayPortOutcomeDto) => void | Promise<void>
}

/**
 * 修改模型路由监听端口弹窗（无独立画板，待确认）：
 * - 共用于「模型路由」更多菜单 / 状态条、概览大卡（已接入工具或原地编辑失败时）与「Agent」页端口报错闭环；
 * - 内置「推荐可用端口」实测可绑定端口；
 * - 方案 A：当 wired > 0 时明确告知将先停止中转并还原已接入工具。
 */
export function GatewayPortDialog({
  open,
  currentPort,
  wired,
  reason,
  initialPort,
  autoSuggest = false,
  confirmLabel,
  onOpenChange,
  onSaved,
}: GatewayPortDialogProps) {
  const [draft, setDraft] = useState("")
  const [suggested, setSuggested] = useState<number | null>(null)
  const [suggesting, setSuggesting] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    setError(null)
    setSuggested(null)
    const fallback = initialPort ?? (currentPort ? String(currentPort) : "")
    setDraft(sanitizePortInput(fallback))
    if (!autoSuggest) return
    let alive = true
    setSuggesting(true)
    void gatewayApi
      .suggestPort()
      .then((port) => {
        if (!alive) return
        setSuggested(port)
        setDraft(String(port))
      })
      .catch(() => {
        /* 推荐失败时保留原输入，由用户手填 */
      })
      .finally(() => {
        if (alive) setSuggesting(false)
      })
    return () => {
      alive = false
    }
  }, [open, currentPort, initialPort, autoSuggest])

  const pickRecommended = async () => {
    if (suggesting || saving) return
    setSuggesting(true)
    setError(null)
    try {
      const port = await gatewayApi.suggestPort()
      setSuggested(port)
      setDraft(String(port))
    } catch (reasonText) {
      setError(String(reasonText))
    } finally {
      setSuggesting(false)
    }
  }

  const parsed = parseGatewayPort(draft)
  const unchanged = parsed.ok && currentPort !== null && parsed.port === currentPort
  const primaryLabel =
    confirmLabel ?? (wired > 0 ? "还原并修改端口" : "保存端口")

  const submit = async () => {
    if (saving || suggesting) return
    if (!parsed.ok) {
      setError(parsed.error)
      return
    }
    if (unchanged) {
      onOpenChange(false)
      return
    }
    setSaving(true)
    setError(null)
    try {
      const outcome = await gatewayApi.setPort(parsed.port, wired > 0)
      onOpenChange(false)
      await onSaved(outcome)
    } catch (reasonText) {
      setError(String(reasonText))
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => { if (!saving) onOpenChange(next) }}
      preventClose={saving}
      width="sm"
      title="修改监听端口"
      description={`本机路由当前地址为 127.0.0.1:${currentPort ?? "—"}；范围 ${GATEWAY_PORT_MIN}–${GATEWAY_PORT_MAX}。`}
      actions={
        <>
          <Button kind="ghost" size="md" disabled={saving} onClick={() => onOpenChange(false)}>取消</Button>
          <Button
            kind={wired > 0 ? "danger" : "primary"}
            size="md"
            icon="check"
            loading={saving}
            disabled={saving || suggesting || !parsed.ok || unchanged}
            onClick={() => void submit()}
          >
            {primaryLabel}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        {reason && <Banner tone="danger">{reason}</Banner>}
        {wired > 0 && (
          <Banner tone="warning">
            当前有 {wired} 个工具已接入旧端口；修改端口将先停止中转并还原这些工具的配置，修改完成后请按需重新接入。
          </Banner>
        )}
        <FormField
          label="监听端口"
          required
          error={error ?? (!parsed.ok && draft !== "" ? parsed.error : undefined)}
          hint={
            suggested !== null && draft === String(suggested)
              ? `已检测端口 ${suggested} 当前可用，可直接保存`
              : "保存前会先试探端口是否可用；在 Windows 上可点右侧按钮避开系统保留端口段"
          }
        >
          <div className="flex items-center gap-2">
            <Input
              size="lg"
              className="font-mono tnum"
              inputMode="numeric"
              value={draft}
              disabled={saving}
              placeholder="例如 18731"
              aria-label="模型路由监听端口"
              onChange={(event) => {
                setError(null)
                setDraft(sanitizePortInput(event.target.value))
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault()
                  void submit()
                }
              }}
            />
            <Button
              kind="ghost"
              size="md"
              icon="spark"
              loading={suggesting}
              disabled={saving || suggesting}
              onClick={() => void pickRecommended()}
            >
              推荐可用端口
            </Button>
          </div>
        </FormField>
      </div>
    </Dialog>
  )
}
