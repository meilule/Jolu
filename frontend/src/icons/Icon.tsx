/**
 * 界面图标组件。语义名由 `ui.ts` 的注册表决定，这里只管尺寸、线宽与无障碍。
 *
 * 单独成文件而不是塞进 `index.ts`：barrel 要保持纯导出（`.ts`），
 * 含 JSX 的必须是 `.tsx`——混在一起会让 `index.ts` 无法被非 React 代码引用，
 * 也会让「导入一个常量却拉进整棵 React 树」变成可能。
 */
import { UI_ICONS, type IconName } from "./ui"

/** 四档尺寸，与设计稿 R1-C 的 12 / 14 / 16 / 20 一一对应。 */
const SIZE = { xs: 12, sm: 14, md: 16, lg: 20 } as const

/** 统一线宽。lucide 默认 2，设计稿是 1.75——细一点更接近硬件感的描边。 */
const STROKE = 1.75

export function Icon({
  name,
  size = "md",
  className,
  label,
}: {
  name: IconName
  size?: keyof typeof SIZE
  /** 只放布局类（外边距、对齐）。颜色与尺寸分别由父元素和 size 决定。 */
  className?: string
  /** 纯图标元素的无障碍名。旁边有可见文字时不要传。 */
  label?: string
}) {
  const Component = UI_ICONS[name]
  return (
    <Component
      size={SIZE[size]}
      strokeWidth={STROKE}
      className={className}
      aria-hidden={label ? undefined : true}
      aria-label={label}
      role={label ? "img" : undefined}
    />
  )
}
