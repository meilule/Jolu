import { useEffect, useState } from "react"
import { ShieldCheck } from "lucide-react"
import { isTauri } from "@/lib/api"
import { settingsApi } from "@/lib/settingsApi"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import {
  MASK_ACTIONS,
  MASKING_PLACEHOLDER_SAMPLE,
  PRIVACY_MASKING_KINDS,
  actionOf,
  maskingDisplay,
  parseAllow,
  withAction,
  type MaskAction,
} from "./privacyMasking"
import { PrivacyRecordsDialog } from "./PrivacyRecordsDialog"
import { ActionButton, Segmented, SettingsGroup, SettingsRow, Switch } from "./settingsUi"

/**
 * 隐私脱敏（P5 / 3.7）：网关中转的请求在离开本机前按固定规则换成占位符，回复自动还原。
 * 待确认：Jolu.pen 模块 G（画板 86–103）没有覆盖此分区，按既有 SettingsGroup 布局挂在「网络与代理」页；
 * 3.7 的每类处理方式、从不脱敏清单与脱敏记录同样无画板，按现有控件做最小版。
 * 「本地模型识别层暂未实现」如实标注，不提供假入口。
 */
export function PrivacySection() {
  const cfg = useSettings()
  const toast = useToast()
  // 乐观值：点下去立即显示目标态；保存失败回滚到后端保存值，成功等 settings-changed 广播对齐后清除
  const [applied, setApplied] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  const [records, setRecords] = useState(false)
  const savedAllow = (cfg.settings.privacy_allow ?? []).join("\n")
  const [allowDraft, setAllowDraft] = useState<string | null>(null)
  const saved = cfg.settings.privacy_masking === true
  const on = maskingDisplay(saved, applied)
  const disabled = !cfg.ready || busy || !isTauri
  const actions = cfg.settings.privacy_actions ?? {}

  useEffect(() => {
    // 广播到达且与乐观值一致后清掉，显示完全交给后端真值
    if (applied !== null && applied === saved) setApplied(null)
  }, [saved, applied])

  const toggle = (next: boolean) => {
    if (busy) return
    setBusy(true)
    setApplied(next)
    settingsApi.privacyMaskingSet(next)
      .then(() => {
        toast.success(
          next ? "已启用隐私脱敏" : "已关闭隐私脱敏",
          next ? "网关请求离开本机前按固定规则替换隐私值" : "网关请求按原文发出",
        )
      })
      .catch((reason) => {
        setApplied(null)
        toast.danger("隐私脱敏设置未保存", String(reason))
      })
      .finally(() => setBusy(false))
  }

  const savePolicy = (nextActions: Record<string, string>, allow: string[], done?: () => void) => {
    if (busy) return
    setBusy(true)
    settingsApi.privacyPolicySet(nextActions, allow)
      .then(() => done?.())
      .catch((reason) => toast.danger("隐私脱敏设置未保存", String(reason)))
      .finally(() => setBusy(false))
  }

  const setAction = (kind: string, action: MaskAction) =>
    savePolicy(withAction(actions, kind, action), cfg.settings.privacy_allow ?? [])

  const saveAllow = () => {
    const allow = parseAllow(allowDraft ?? savedAllow)
    savePolicy(actions, allow, () => {
      setAllowDraft(null)
      toast.success("从不脱敏清单已保存", allow.length ? `共 ${allow.length} 项` : "清单已清空")
    })
  }

  return (
    <SettingsGroup
      icon={ShieldCheck}
      title="隐私脱敏"
      desc="网关中转的请求与回复按固定规则脱敏；替换表只在内存里，不落盘。"
    >
      <SettingsRow
        className="min-h-[76px]"
        title="启用隐私脱敏"
        desc="请求离开本机前，把密钥等隐私值换成稳定占位符，回复到达后自动还原；同一原值始终得到同一占位符，不影响 prompt cache。脱敏引擎不可用时请求会被拒绝（503），而不是原样发出。"
        control={<Switch label="启用隐私脱敏" on={on} disabled={disabled} onChange={toggle} />}
      />
      <div className="flex flex-col gap-2 border-t border-jolu-border py-3.5">
        <div className="flex items-baseline gap-2.5">
          <span className="text-[13px] font-medium text-jolu-text">各类处理方式</span>
          <span className="text-[11.5px] text-jolu-muted">遮住：换成占位符如 <code className="rounded bg-jolu-side px-1 py-px font-mono text-[11px]">{MASKING_PLACEHOLDER_SAMPLE}</code> · 只记录：原样发出 · 拦下：整条请求不发</span>
        </div>
        <ul className="flex flex-col gap-1.5 pt-0.5">
          {PRIVACY_MASKING_KINDS.map((item) => (
            <li key={item.kind} className="flex items-center gap-3">
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="text-xs text-jolu-text">{item.name}</span>
                <span className="truncate text-[11.5px] text-jolu-muted">{item.hint}</span>
              </span>
              <Segmented
                label={`${item.name}的处理方式`}
                items={MASK_ACTIONS.map((a) => ({ value: a.value, label: a.label }))}
                value={actionOf(actions, item.kind)}
                disabled={disabled}
                onChange={(action) => setAction(item.kind, action)}
              />
            </li>
          ))}
        </ul>
      </div>
      <div className="flex flex-col gap-2 border-t border-jolu-border py-3.5">
        <span className="text-[13px] font-medium text-jolu-text">从不脱敏</span>
        <span className="text-[11.5px] text-jolu-muted">这些值总是原样发出（整段匹配、不分大小写），比如团队邮箱、测试密钥；每行一个，也可以用逗号隔开。交接摘要的脱敏同样放过它们。</span>
        <textarea
          aria-label="从不脱敏的值"
          value={allowDraft ?? savedAllow}
          disabled={disabled}
          onChange={(event) => setAllowDraft(event.target.value)}
          rows={3}
          className="w-full resize-y rounded-lg border border-jolu-border bg-jolu-bg px-3 py-2 font-mono text-xs text-jolu-text outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-60"
        />
        <div className="flex justify-end gap-2">
          {allowDraft !== null && <ActionButton disabled={busy} onClick={() => setAllowDraft(null)}>还原</ActionButton>}
          <ActionButton tone="accent" disabled={disabled || allowDraft === null} onClick={saveAllow}>保存清单</ActionButton>
        </div>
      </div>
      <SettingsRow
        title="脱敏记录"
        desc="最近的请求各遮了什么，发出前后对照；只在内存、不含原值，重启后清空。"
        control={<ActionButton disabled={!isTauri} onClick={() => setRecords(true)}>查看记录</ActionButton>}
      />
      <p className="border-t border-jolu-border py-3 text-[11.5px] leading-[1.6] text-jolu-muted">
        本地模型识别层暂未实现。
      </p>
      {records && <PrivacyRecordsDialog onClose={() => setRecords(false)} />}
    </SettingsGroup>
  )
}
