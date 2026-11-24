/**
 * 动效令牌（JS 层）。CSS 侧同值定义在 `theme/motion.css`。
 *
 * ## 为什么要两份
 *
 * 即时反馈（悬停变色）用 CSS transition 最省；进出场、布局变化、弹簧必须走
 * Motion（CSS 的 transition 被打断时速度归零，弹簧做不到「保留当前速度」）。
 * 两套运行时并存，但**数值必须同源**。
 *
 * `tokens.test.mjs` 读本文件与 `theme/motion.css` 比对，改一处忘了另一处会失败。
 * 这是唯一的目的——不是为了覆盖率。
 *
 * ## 为什么不从 CSS 读
 *
 * 构建期读 CSS 需要额外的解析步骤，且值在运行时才可读；这里直接是常量，
 * 组件 import 就能用。代价是两份，用测试兜住。
 */

/** 时长（秒）。与 `theme/motion.css` 的 `--motion-*` 一一对应。 */
export const DURATION = {
  /** 悬停、按下、颜色切换 —— 唯一走 CSS transition 的一档 */
  feedback: 0.12,
  /** 菜单、下拉、Tooltip 进场 */
  pop: 0.16,
  /** 同上退场。比进场短：退场时用户已经决定离开，拖长只会显得迟钝 */
  popOut: 0.12,
  /** 弹窗 */
  dialog: 0.18,
  /** 抽屉、面板滑入 */
  slide: 0.22,
} as const satisfies Record<string, number>

/** 缓动曲线。与 `theme/motion.css` 的 `--motion-ease-*` 同值。 */
export const EASE = {
  /** 标准进出：快起慢收 */
  standard: [0.2, 0.8, 0.2, 1],
  /** 退场：慢起快收，让消失更快结束 */
  exit: [0.4, 0, 1, 1],
} as const

/**
 * 弹簧参数。
 *
 * 弹簧没有 CSS 对应值——这正是它们必须走 Motion 的原因。三个档位按
 * 「物体质量」分：越轻的东西（开关、滑块）刚度越高、阻尼越大；越重的东西
 * （卡片展开）刚度越低、回弹越明显。
 */
export const SPRING = {
  /** 页签滑块、开关：小而精确，不允许可见回弹 */
  snappy: { type: "spring", stiffness: 520, damping: 38 },
  /** 卡片展开：有轻微回弹，像有重量的东西被拉开 */
  soft: { type: "spring", stiffness: 260, damping: 30 },
  /**
   * 灵动岛形变。对应旧胶囊样式的 `cubic-bezier(.32,1.28,.48,1)`——那条曲线
   * 有超出 1 的回弹段，弹簧是最接近的表达。
   */
  island: { type: "spring", stiffness: 380, damping: 26 },
} as const

/** 列表依次入场的间隔（秒）。超过 30 项时不做依次入场，直接整体淡入。 */
export const STAGGER = 0.03

/** 依次入场的项数上限。超过就整块淡入——30 项之后人眼看不出错开，只是变慢。 */
export const STAGGER_LIMIT = 30
