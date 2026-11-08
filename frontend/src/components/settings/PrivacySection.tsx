import { useEffect, useState } from "react"
import { ShieldCheck } from "lucide-react"
import { isTauri } from "@/lib/api"
import { settingsApi } from "@/lib/settingsApi"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import { MASKING_PLACEHOLDER_SAMPLE, PRIVACY_MASKING_RULES, maskingDisplay } from "./privacyMasking"
import { SettingsGroup, SettingsRow, Switch } from "./settingsUi"

/**
 * 隐私脱敏（P5）：网关中转的请求在离开本机前按固定规则换成占位符，回复自动还原。
 * 待确认：Jolu.pen 模块 G（画板 86–103）没有覆盖此分区，按既有 SettingsGroup 布局挂在「网络与代理」页；
 * 「本地模型识别层与改写对照页暂未实现」如实标注，不提供假入口。
 */
export function PrivacySection() {
  const cfg = useSettings()
  const toast = useToast()
  // 乐观值：点下去立即显示目标态；保存失败回滚到后端保存值，成功等 settings-changed 广播对齐后清除
  const [applied, setApplied] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  const saved = cfg.settings.privacy_masking === true
  const on = maskingDisplay(saved, applied)
  const disabled = !cfg.ready || busy || !isTauri

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
          <span className="text-[13px] font-medium text-jolu-text">脱敏范围</span>
          <span className="text-[10.5px] text-jolu-muted">固定规则 · 占位符形如 <code className="rounded bg-jolu-side px-1 py-px font-mono text-[10px]">{MASKING_PLACEHOLDER_SAMPLE}</code></span>
        </div>
        <ul className="grid grid-cols-2 gap-x-6 gap-y-2 pt-0.5">
          {PRIVACY_MASKING_RULES.map((rule) => (
            <li key={rule.name} className="flex min-w-0 flex-col">
              <span className="text-xs text-jolu-text">{rule.name}</span>
              <span className="truncate text-[10.5px] text-jolu-muted">{rule.hint}</span>
            </li>
          ))}
        </ul>
      </div>
      <p className="border-t border-jolu-border py-3 text-[10.5px] leading-[1.6] text-jolu-muted">
        本地模型识别层与改写对照页暂未实现。
      </p>
    </SettingsGroup>
  )
}
