/**
 * 界面图标的唯一注册表。
 *
 * ## 为什么要有注册表
 *
 * 现状是 80 个文件各自 `import { … } from "lucide-react"`，141 个图标里
 * 同一个意思有两种画法（「刷新」用了 `RefreshCw` 和 `RotateCw`）。换一个图标要
 * 翻遍全仓库，而改一个只改到一半，界面就不一致。
 *
 * 这里把**语义**固定下来：页面说 `name="refresh"`，具体用哪个 lucide 图标是
 * 这张表的事。换图标只改一行，全站跟着变。
 *
 * ## 加图标的两步
 *
 * 1. 在 `UI_ICONS` 里加一行语义名 → lucide 组件（按语义命名，不按形状命名：
 *    `add` 而不是 `plus`，`delete` 而不是 `trash`）。
 * 2. 如果确实是一个新的语义，同时确认没有已有语义能表达它——重复的语义名
 *    正是这个文件要消灭的东西。
 *
 * ## 尺寸与线宽
 *
 * 只有四档（12 / 14 / 16 / 20）与一个线宽（1.75），与设计稿 R1-C 一致。
 * 不允许页面传任意像素值：`size={13}` 这种写法会让同行图标高矮不齐。
 */
import {
  Activity,
  ArrowLeft,
  ArrowRight,
  ArrowUpRight,
  Blocks,
  BookOpen,
  Bot,
  Calendar,
  CalendarCheck2,
  ChartColumn,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  ChevronsUp,
  CircleAlert,
  CircleCheck,
  CircleFadingArrowUp,
  CircleX,
  Clock,
  Coins,
  Cpu,
  Bell,
  Copy,
  Database,
  Download,
  Ellipsis,
  Eye,
  EyeOff,
  FileCode,
  FileDown,
  FileText,
  Filter,
  Flame,
  Folder,
  FolderCog,
  FolderInput,
  FolderOpen,
  Gauge,
  Gift,
  GitBranch,
  GitCompare,
  Github,
  Globe,
  Hourglass,
  Image,
  KeyRound,
  Layers,
  Link2,
  List,
  ListChecks,
  MessageSquareText,
  Minus,
  Palette,
  PanelLeft,
  PictureInPicture2,
  Route,
  Info,
  LoaderCircle,
  Moon,
  Pause,
  Pencil,
  Play,
  Plug,
  Plus,
  Power,
  RefreshCw,
  RotateCcw,
  Scan,
  Search,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Square,
  SquareCheck,
  SquareTerminal,
  Sun,
  Terminal,
  Trash2,
  TriangleAlert,
  Undo2,
  Upload,
  UsersRound,
  Wallet,
  X,
} from "lucide-react"
import type { LucideIcon } from "lucide-react"

/**
 * 语义名 → 图标组件。
 *
 * 命名约定：动作类用动词（`add` / `delete` / `copy`），状态类用名词
 * （`success` / `warning` / `danger` / `info`），方向类用相对位置
 * （`next` / `prev` / `expand` / `collapse`）。
 */
export const UI_ICONS = {
  // ── 关闭、窗口与方向 ──
  close: X,
  minimize: Minus,
  maximize: Square,
  restore: Copy,
  sidebarToggle: PanelLeft,
  next: ChevronRight,
  prev: ChevronLeft,
  expand: ChevronDown,
  collapse: ChevronUp,
  arrowLeft: ArrowLeft,
  arrowRight: ArrowRight,

  // ── 增删改 ──
  add: Plus,
  delete: Trash2,
  edit: Pencil,
  copy: Copy,
  download: Download,
  upload: Upload,
  import: FolderInput,
  more: Ellipsis,

  // ── 查找、刷新与重试 ──
  // 这三个是不同语义，不能合并：
  //   search  = 查找
  //   refresh = 主动拉取最新（列表/额度的手动刷新）
  //   retry   = 上一次失败后重来（错误态里的按钮）
  //   undo    = 撤销刚才的编辑，回到上一版
  search: Search,
  filter: Filter,
  refresh: RefreshCw,
  retry: RotateCcw,
  undo: Undo2,
  scan: Scan,
  loading: LoaderCircle,

  // ── 状态 ──
  success: CircleCheck,
  warning: TriangleAlert,
  danger: CircleAlert,
  fail: CircleX,
  info: Info,
  check: Check,
  /** 未选中的勾选框底板 */
  unchecked: Square,
  /** 已选中的勾选框（带勾） */
  checked: SquareCheck,

  // ── 对象 ──
  file: FileCode,
  fileText: FileText,
  compress: FileDown,
  image: Image,
  gitBranch: GitBranch,
  diff: GitCompare,
  chevronsUp: ChevronsUp,
  folder: Folder,
  folderConfig: FolderCog,
  database: Database,
  bell: Bell,
  github: Github,
  globe: Globe,
  update: CircleFadingArrowUp,
  connect: Plug,
  openFolder: FolderOpen,
  shield: ShieldCheck,
  activity: Activity,
  bot: Bot,
  calendar: Calendar,
  clock: Clock,
  hourglass: Hourglass,
  list: List,
  listChecks: ListChecks,
  flame: Flame,
  layers: Layers,
  spark: Sparkles,
  gift: Gift,
  palette: Palette,
  sliders: SlidersHorizontal,
  terminal: Terminal,
  key: KeyRound,
  cpu: Cpu,
  coins: Coins,
  wallet: Wallet,

  // ── 显示 ──
  eye: Eye,
  eyeOff: EyeOff,
  external: ArrowUpRight,

  // ── 播放控制 ──
  play: Play,
  pause: Pause,
  stop: Square,
  power: Power,
  gauge: Gauge,

  // ── 主题 ──
  light: Sun,
  dark: Moon,

  // ── 侧栏导航（10 项，与 mainNavigation 的页面一一对应）──
  navIsland: PictureInPicture2,
  navProviders: Link2,
  navCheckin: CalendarCheck2,
  navUsage: ChartColumn,
  navSessions: MessageSquareText,
  navAutomation: Bot,
  navSkills: BookOpen,
  navAgents: SquareTerminal,
  navGateway: Route,
  navSettings: SlidersHorizontal,

  // ── 页面内功能图标 ──
  blocks: Blocks,
  users: UsersRound,
} as const satisfies Record<string, LucideIcon>

export type IconName = keyof typeof UI_ICONS
