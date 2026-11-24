/**
 * 图标的唯一出口。
 *
 * 页面与组件一律 `import { Icon } from "@/icons"`，不直接引 `lucide-react`：
 *
 * ```tsx
 * <Icon name="refresh" />                       // 16px，跟随当前文字颜色
 * <Icon name="close" size="sm" label="关闭" />   // 14px，带无障碍名
 * ```
 *
 * 本文件只做转出，不写 JSX——含 JSX 的组件在 `Icon.tsx`，语义注册表在 `ui.ts`。
 * 拆开的实际原因：`index.ts` 要能被任何地方安全引用，而含 JSX 的文件必须是
 * `.tsx`。纯逻辑（比如校验注册表完整性的测试）直接引 `@/icons/ui`，
 * 不必经过 barrel，也就不会连带加载组件。
 */
export { Icon } from "./Icon"
export { UI_ICONS } from "./ui"
export type { IconName } from "./ui"
