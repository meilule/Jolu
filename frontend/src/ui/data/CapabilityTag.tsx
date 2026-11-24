/**
 * CapabilityTag —— 设计稿 R1-C「分区 · 4 · 状态与数据 / 能力标签 CapabilityTag」。
 *
 * 规格取自画板：高 24、圆角 8、文字 11.5 / 600、左内边距 8 / 右 9。
 *
 * ## 六个等级（产品定位 §3、§7 + 能力矩阵五态）
 *
 * | level | 文案 | 含义 |
 * |---|---|---|
 * | verified | 真实账号已验证 | 用真实账号跑通过 |
 * | partial | 部分验证 | 部分档位 / 版本 / 流程真实跑通，其余缺样本 |
 * | synthetic | 合成测试通过 | 只有 fixture / 假上游 |
 * | pending | 待验证 | 代码在，没有验收记录（能力矩阵的「未验证」） |
 * | unknown | 无记录 | 没找到任何验证记录，不推断（能力矩阵的「无记录」） |
 * | unsupported | 不支持 | 该平台没有这个能力 |
 *
 * `partial` / `unknown` 是 Agent 详情「能力验证」按真实能力矩阵展示时补的：
 * 矩阵有五种状态（见 `lib/capabilityVerification.ts`），四个标签盖不住，
 * 少一档就只能把「部分验证」说成「待验证」或把「无记录」说成「不支持」。
 *
 * ## 为什么「合成测试通过」不能写成「通过」
 *
 * 合成测试证明的是协议翻译对得上，不证明真实账号能跑。把它写成「通过」，
 * 用户会以为这项功能已经在真机上验证过——这正是能力矩阵要区分五档的原因。
 * 所以文案由组件固定，调用方只选等级，不能自填文字。
 */
import { cn } from "@/lib/utils"

const LEVEL = {
  verified: { label: "真实账号已验证", className: "bg-jn-primary-soft text-jn-primary-ink" },
  partial: { label: "部分验证", className: "bg-jn-warning-soft text-jn-warning" },
  synthetic: { label: "合成测试通过", className: "bg-jn-info-soft text-jn-info" },
  pending: { label: "待验证", className: "bg-jn-warning-soft text-jn-warning" },
  unknown: { label: "无记录", className: "bg-jn-well text-jn-text-2" },
  unsupported: { label: "不支持", className: "bg-jn-well text-jn-text-3" },
} as const

export type CapabilityLevel = keyof typeof LEVEL

export function CapabilityTag({
  level,
  className,
}: {
  level: CapabilityLevel
  className?: string
}) {
  const spec = LEVEL[level]
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center rounded-[8px] pr-[9px] pl-2 text-[11.5px] font-semibold whitespace-nowrap",
        spec.className,
        className,
      )}
    >
      {spec.label}
    </span>
  )
}
