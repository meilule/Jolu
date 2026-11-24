/**
 * DataValue —— 设计稿 R1-C「分区 · 4 · 状态与数据 / 数据值 DataValue（七种状态）」。
 *
 * 产品定位 §5.2 在界面上的唯一落点：**所有数字都经过这个组件**，页面不直接写文本。
 *
 * ## 七种状态为什么必须分开
 *
 * 这是整个产品最容易被做坏的一处：一个 `0` 可能表示「确实没有消耗」，也可能
 * 表示「接口没返回」。一个 `—` 可能表示「没有数据源」，也可能表示「采集出错了」。
 * 混在一起时用户无法判断该不该相信这个数，也无法判断该不该重试。
 *
 * | 状态 | 显示 | 语义 |
 * |---|---|---|
 * | value | 数字 + 单位 | 确定的读数 |
 * | zero | `0` + 单位，正常字色 | 采集成功且真的为零 |
 * | estimated | `≈` 前缀 + 「估算」 | 由公式或部分数据推算 |
 * | stale | 数字变浅 + 「已过期 · 多久前」 | 有旧值但超过刷新周期 |
 * | unknown | `—` + 「未知」 | 没有来源能回答 |
 * | failed | 红色 `—` + 「采集失败」 | 采集出错 |
 * | unsupported | 「不支持」 | 该平台没有这项能力 |
 *
 * ## 类型层面就不允许混用
 *
 * 用可辨识联合：`state="unknown"` 时不能传 `value`，`state="value"` 时必须传。
 * 这样「有值却标成未知」这种错在编译期就不可能发生，不必靠 review 兜。
 *
 * ## 为什么 stale 要带「多久前」
 *
 * 只写「已过期」用户不知道是 2 分钟还是 2 天前——两者的可信度完全不同。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"
import { AnimatedNumber } from "@/motion"

/** 数字格式化。默认按千分位，调用方可传 compact（1.2K）这类格式。 */
export type FormatNumber = (value: number) => string

const defaultFormat: FormatNumber = (value) => value.toLocaleString("en-US")

interface BaseProps {
  /** 单位。跟在数字后面，小一号、次级色。Token / 次 / 美元 等。 */
  unit?: string
  /** 数字格式。不传则千分位。 */
  format?: FormatNumber
  /** 覆盖默认的说明文字（如把「未知」写成「未连接」）。 */
  note?: ReactNode
  className?: string
}

/**
 * 七个成员各自只有一个 `state` 字面量。
 *
 * 不写成 `state: "value" | "zero"` 这种联合：TypeScript 对「成员内判别字段是联合」
 * 的排除式收窄并不可靠（早返回之后仍可能拿不到 `value`），拆成单字面量最稳。
 */
export type DataValueProps =
  | (BaseProps & { state: "value"; value: number })
  | (BaseProps & { state: "zero"; value: number })
  | (BaseProps & { state: "estimated"; value: number })
  | (BaseProps & { state: "stale"; value: number; /** 距上次成功多久，如「3 分钟前」 */ age: string })
  | (BaseProps & { state: "unknown" })
  | (BaseProps & { state: "failed" })
  | (BaseProps & { state: "unsupported" })

/** 主数字的字重与字号。指标卡与表格共用一档，不按场景各调。 */
const NUMBER = "tnum font-mono text-[15px] font-semibold"

export function DataValue(props: DataValueProps) {
  // ── 未知 / 失败 / 不支持：没有数字，只有占位符与一句说明 ──
  if (props.state !== "value" && props.state !== "zero" && props.state !== "estimated" && props.state !== "stale") {
    const { note, className } = props
    const failed = props.state === "failed"
    const text =
      props.state === "unknown" ? "—" : props.state === "failed" ? "—" : "不支持"
    const label =
      note ?? (props.state === "unknown" ? "未知" : props.state === "failed" ? "采集失败" : undefined)
    return (
      <span className={cn("inline-flex items-baseline gap-1.5", className)}>
        <span className={cn(NUMBER, failed ? "text-jn-danger" : "text-jn-text-3")}>{text}</span>
        {label && <span className="text-[11px] text-jn-text-3">{label}</span>}
      </span>
    )
  }

  // ── 有数字的四种 ──
  // 解构放在早返回之后：先按 state 收窄类型，再取字段，
  // 否则 TS 的联合收窄会被前面的解构打断。
  const { unit, format = defaultFormat, note, className } = props
  const stale = props.state === "stale"
  const estimated = props.state === "estimated"

  return (
    <span className={cn("inline-flex items-baseline gap-1.5", className)}>
      {estimated && <span className="text-jn-text-3 text-[15px]">≈</span>}
      <AnimatedNumber value={props.value} format={format} className={cn(NUMBER, stale ? "text-jn-text-3" : "text-jn-text")} />
      {unit && <span className="text-[11px] text-jn-text-3">{unit}</span>}
      {estimated && <span className="text-[11px] text-jn-text-3">{note ?? "估算"}</span>}
      {stale && (
        <span className="text-[11px] text-jn-text-3">
          {note ?? `已过期 · ${props.age}`}
        </span>
      )}
    </span>
  )
}
