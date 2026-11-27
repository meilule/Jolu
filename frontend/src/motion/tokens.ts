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
  /** 页签滑块、导航指示器：小而精确，不允许可见回弹 */
  snappy: { type: "spring", stiffness: 520, damping: 38 },
  /**
   * 液态开关落位弹簧（Liquid toggle，Bencho，MIT，stiffness: 170 / damping: 21.5 / mass: 0.9，
   * 阻尼比 zeta ≈ 0.87）：到位有极其柔和的液体回弹余韵，没有机械硬停；
   * 跟随/微胀弹簧见 `liquidFollow`。
   */
  liquidSettle: { type: "spring", stiffness: 170, damping: 21.5, mass: 0.9 },
  /** 液态滑块的速度采样平滑与悬停微胀（stiffness: 380 / damping: 34 / mass: 0.6，不产生二次回弹） */
  liquidFollow: { stiffness: 380, damping: 34, mass: 0.6 },
  /** 按钮与图标按钮触感：按下轻弹、松手利落回弹，与液态开关同一套弹性语汇 */
  tactile: { type: "spring", stiffness: 520, damping: 28, mass: 0.7 },
  /** 选项块与可点卡片触感：质量感稍重，按压柔和回弹不浮夸 */
  tactileSoft: { type: "spring", stiffness: 420, damping: 30, mass: 0.85 },
  /** 卡片展开：有轻微回弹，像有重量的东西被拉开 */
  soft: { type: "spring", stiffness: 260, damping: 30 },
  /**
   * 灵动岛形变。对应旧胶囊样式的 `cubic-bezier(.32,1.28,.48,1)`——那条曲线
   * 有超出 1 的回弹段，弹簧是最接近的表达。
   */
  island: { type: "spring", stiffness: 380, damping: 26 },
} as const

/**
 * 各类可交互控件的悬停/按下物理预设。
 * 统一从这里取参数，组件不各自手写 `scale` 和 `stiffness`。
 */
export const TACTILE = {
  /** 标准按钮 / 分页丸 / 批量动作 */
  button: {
    hover: { scale: 1.015 },
    tap: { scale: 0.96 },
    transition: SPRING.tactile,
  },
  /** 纯图标圆形/圆角按钮：几何接近液态水滴，悬停微胀与按压弹性略更明显 */
  icon: {
    hover: { scale: 1.045 },
    tap: { scale: 0.91 },
    transition: SPRING.tactile,
  },
  /** 选项块 / 可点击卡片 / 策略芯片：大面积按压只做轻收，避免字形抖动 */
  tile: {
    hover: { scale: 1.01 },
    tap: { scale: 0.975 },
    transition: SPRING.tactileSoft,
  },
  /** 下拉触发器 / 折叠头 */
  control: {
    hover: { scale: 1.008 },
    tap: { scale: 0.985 },
    transition: SPRING.tactile,
  },
  /** 列表行 / 表格行 / 横条行：不放大（防止宽行溢出卡片裁切），只做按压轻落反馈 */
  row: {
    hover: undefined,
    tap: { scale: 0.988 },
    transition: SPRING.tactileSoft,
  },
  /** 本机路由花瓣主开关：120px 核心控件，悬停微扩 + 按压深沉回弹 */
  petal: {
    hover: { scale: 1.03 },
    tap: { scale: 0.94 },
    transition: SPRING.soft,
  },
} as const

export type TactileVariant = keyof typeof TACTILE

/** 列表依次入场的间隔（秒）。超过 30 项时不做依次入场，直接整体淡入。 */
export const STAGGER = 0.03

/** 依次入场的项数上限。超过就整块淡入——30 项之后人眼看不出错开，只是变慢。 */
export const STAGGER_LIMIT = 30
