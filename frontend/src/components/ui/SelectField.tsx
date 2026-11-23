import * as Menu from "@radix-ui/react-dropdown-menu"
import { Check, ChevronDown, Search } from "lucide-react"
import { useCallback, useEffect, useId, useLayoutEffect, useState, type ReactNode } from "react"
import { cn } from "@/lib/utils"

export interface SelectOption {
  value: string
  label: string
  description?: string
  icon?: ReactNode
  disabled?: boolean
}

/**
 * [DROPDOWN-STANDARD] 全项目单选下拉的唯一实现。
 *
 * 新代码必须遵循 DROPDOWN-SPEC.md（样式与动效规范），
 * 全量清单与迁移状态见 DROPDOWN-INVENTORY.md。
 * 新增下拉一律用它，不要再用 Menu.RadioGroup / 原生 <select> 自建。
 *
 * 原型统一选择器：菜单、选中、禁用与焦点均由应用主题绘制。
 *
 * `glide` 打开时换成「共享滑块」的高亮方式（React Bits GlideSelect 的动效）：
 * 菜单里只有一个绝对定位的滑块元素，靠 translateY 在行间平移，而不是每行
 * 各自切换底色。一次只有一个物体在动，读起来是「一个滑块在移动」。
 * 其余全部沿用 Radix：Portal 定位与碰撞翻转、键盘导航、typeahead、焦点管理。
 */
/**
 * 下拉宽度挡位（统一口径）。
 *
 * 之前每处手写 `w-[142px]` / `w-[120px]` / `w-full`，同一个界面里宽窄不一。
 * 现在收敛成四个语义值，调用方只选挡位，不再写像素：
 *
 *   xs   120px  行内的短值（分钟数、百分比）
 *   sm   142px  设置页右侧的操作控件（默认）
 *   md   170px  需要读完整选项名的行内选择
 *   lg   280px  长文案（带副标题的策略 / 模型名）
 *   full 100%   表单里独占一列（配合父容器宽度）
 *
 * `className` 仍可传，但只应放布局意图（对齐、外边距）；
 * 宽度请用 `width` 挡位，不要再写 w-[...]。
 */
export type SelectWidth = "xs" | "sm" | "md" | "lg" | "full"

/**
 * 触发器外观变体。
 *
 * `default` 标准下拉：32px 高、8px 圆角、带边框。
 * `filter`  筛选条：36px 高、9px 圆角、无边框（由外层 label 画框），
 *           左侧留给外部图标、右侧箭头 12px。用于工具栏里的筛选控件，
 *           与搜索框并列时保持同一条视觉基线。
 * `form`    表单下拉：38px 高、10px 圆角、全宽，与同表单里的 input 同高
 *           （画板 80 / 82 的「供应商」「作用范围」「安装范围」）。
 * `toolbar` 工具栏筛选条：34px 高、8px 圆角、带边框、可带前置图标。
 *           用于页面顶部的一排筛选（用量统计、Skills 管理）。
 * `picker`  带图标的资源选择器：38px 高、176px 宽、font-medium、带边框。
 *           用于「选一个供应商/资源」这类带品牌图标的选择（画板 76）。
 *
 * 三者共用同一套菜单、滑块、键盘与无障碍行为，只是触发器外壳不同。
 */
export type SelectVariant = "default" | "filter" | "form" | "toolbar" | "picker"


export function SelectField({ value, options, onValueChange, label, disabled,
  className, triggerLabel, onOpenChange, readOnlySingle = false, placeholder = "请选择", glide = true,
  width = "sm", menuWidth, variant = "default", triggerIcon, searchable = false,
}: {
  value: string
  options: SelectOption[]
  onValueChange: (value: string) => void
  label: string
  disabled?: boolean
  className?: string
  triggerLabel?: ReactNode
  onOpenChange?: (open: boolean) => void
  readOnlySingle?: boolean
  /** value 不落在任何选项上时触发器显示的文案 */
  placeholder?: string
  /**
   * 共享滑块高亮（滑动选择动效）。默认开启 —— 全项目统一样式；
   * 个别需要旧行为的（如超长列表、特殊排版）传 `glide={false}`。
   */
  glide?: boolean
  /** 宽度挡位，见 SelectWidth。默认 sm（142px，设置页标准） */
  width?: SelectWidth
  /**
   * 下拉面板宽度（px）。不给就与触发器等宽。
   *
   * 什么时候需要：**触发器在窄容器里、选项文字又长**。灵动岛的「切换连接」按钮
   * 只有 142px（受窗口宽度限制），而选项是「Claude · API Key（未配置）」这种长文案，
   * 等宽面板会把每项折成三行（现场截图）。这时给面板一个更宽的值，
   * 既不动触发器的占位，也不让文字折行。
   */
  menuWidth?: number
  /** 触发器外观，见 SelectVariant。默认 default */
  variant?: SelectVariant
  /**
   * 触发器左侧图标（如筛选条的 SlidersHorizontal）。
   * 只在 `filter` 变体下有位置——那里触发器去壳、左侧留白由外层 label 放图标，
   * 这个参数用于「图标在触发器内部」的筛选条（Skills / 用量页的 MenuFilter）。
   */
  triggerIcon?: ReactNode
  /**
   * 菜单顶部显示搜索框（画板 76 的供应商菜单）。
   * 选项多（≥10）时建议开启；搜索只过滤显示，不改变 value。
   */
  searchable?: boolean
}) {
  /* React 会把 useId() 的 ":r6:" 渲染成 "_r_6_" 写进 DOM。
     查询时要对齐这个格式，否则 CSS.escape(":r6:") 永远匹配不到。 */
  const uid = useId().replace(/[:]/g, "_")
  const [query, setQuery] = useState("")

  /**
   * 菜单宽度跟随触发器。
   *
   * 不用 Radix 的 --radix-dropdown-menu-trigger-width：实测在
   * 「触发器外面还包着一层 span + className 带 w-full」的结构里，
   * 那个变量量到的是错的元素（543px 的触发器读成 120px）。
   *
   * 也不能用 document.querySelector(".jolu-select-menu") —— 同页有 8 个
   * SelectField，会写到别人刚打开的那个菜单上（实测所有菜单都被写成同一个值）。
   * 给每个实例一个 useId，菜单带，各自找各自的那个。
   */
  const selected = options.find((option) => option.value === value)
  if (readOnlySingle && value && selected && options.filter((option) => option.value && !option.disabled).length === 1) {
    return <span aria-label={label} title={selected.label} className={cn(
      "inline-flex h-8 min-w-0 items-center text-xs text-text-primary",
      width === "xs" && "w-[120px]",
      width === "sm" && "w-[142px]",
      width === "md" && "w-[170px]",
      width === "lg" && "w-[280px]",
      width === "full" && "w-full",
      className,
    )}><span className="truncate">{selected.label}</span></span>
  }
  return (
    <Menu.Root modal={false} onOpenChange={(open) => { if (!open) setQuery(""); onOpenChange?.(open) }}>
      <Menu.Trigger disabled={disabled}
        aria-label={label}
        type="button"
        className={cn("jolu-control jolu-select-trigger inline-flex min-w-0 shrink-0 items-center justify-between gap-2 text-left text-text-primary outline-none transition-colors disabled:cursor-not-allowed disabled:opacity-45",
            // 标准触发器：32px 高、8px 圆角、带边框；焦点环也只在有边框时才有意义
            variant === "default" && "h-8 rounded-[8px] border bg-surface px-2.5 text-xs hover:bg-hover focus-visible:border-accent-blue focus-visible:ring-2 focus-visible:ring-accent-blue-soft data-[state=open]:border-border-strong",
            // 筛选条：去壳，外观与焦点环都由外层 label 画（focus-within）。
            // 这里不能留 border / ring —— 触发器本身无边框，画出来就是一圈多余的框。
            // 外观（去边框 / 透明底 / 无焦点环 / 无内边距）全部由 index.css 的
            // `html:not(.island-window) .jolu-select-trigger--filter` 规则接管，
            // 因为全局规则权重更高，Tailwind 工具类压不住。
            // 筛选条：去壳后撑满外层 label（宽度由外层定，不用 width 挡位），
            // 高度与 label 一致，内边距 0（外层 label 已给 pl/pr）
            variant === "filter" && "jolu-select-trigger--filter h-9 w-full rounded-[9px] text-meta",
            // 表单下拉：与同表单里的 input 同高（38px）、10px 圆角、全宽
            variant === "form" && "h-[38px] w-full rounded-[10px] border bg-jolu-bg px-3 text-xs hover:bg-jolu-stage focus-visible:border-jolu-accent data-[state=open]:border-jolu-accent",
            // 工具栏筛选条：34px 高、带边框、前置图标 + 文字 + 箭头（用量统计 / Skills 顶栏）
            variant === "toolbar" && "h-[34px] rounded-lg border bg-jolu-bg px-3 text-xs hover:bg-jolu-stage focus-visible:border-jolu-accent data-[state=open]:bg-jolu-stage",
            // 资源选择器：38px 高、176px 宽、font-medium（画板 76 的供应商下拉，原样沿用）
            variant === "picker" && "h-[38px] w-[176px] rounded-lg border bg-jolu-bg px-3 text-xs font-medium hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent data-[state=open]:bg-jolu-stage",
            // 宽度挡位：完整类名写在这里，Tailwind 才能扫到（查表在运行时才求值）。
            // `picker` 自带 176px 固定宽度、`filter` 跟随外层容器，两者都跳过挡位。
            variant !== "picker" && variant !== "filter" && width === "xs" && "w-[120px]",
            variant !== "picker" && variant !== "filter" && width === "sm" && "w-[142px]",
            variant !== "picker" && variant !== "filter" && width === "md" && "w-[170px]",
            variant !== "picker" && variant !== "filter" && width === "lg" && "w-[280px]",
            variant !== "picker" && variant !== "filter" && width === "full" && "w-full",
            className)}
      >
        {/* 图标尺寸完全交给调用方（ProviderMark 是 18px，筛选图标是 14–16px）。
            这里不加 size-* 包裹，否则会把 18px 压成 16px、还多一圈居中留白。 */}
        {triggerIcon ? <span aria-hidden className="flex shrink-0 items-center">{triggerIcon}</span> : null}
        <span className="min-w-0 flex-1 truncate text-left">{triggerLabel ?? selected?.label ?? placeholder}</span>
        {/* 箭头尺寸按变体区分：picker 沿用原版 16px，其余 12px（设置页更紧凑） */}
        {/* 箭头尺寸按变体区分：picker 沿用原版 16px，其余 12px。
            用 size-[16px] 而不是 size-4 —— lucide 的 size-4 会被 svg 自身撑到 18px。 */}
        <ChevronDown aria-hidden className={variant === "picker"
          ? "motion-select-chevron size-[16px] shrink-0 text-jolu-muted"
          : "motion-select-chevron size-3 shrink-0 text-text-secondary"} />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content loop align="start" sideOffset={6} collisionPadding={12}
          className={cn(
            "jolu-select-menu jolu-menu motion-popover z-[140] overflow-y-auto overscroll-contain text-text-primary",
            // 宽度由 open 时实测触发器写入（见下方 onOpenAutoFocus）。
            // 不用 --radix-dropdown-menu-trigger-width：实测该变量在
            // 「触发器被 span 包着 + className 带 w-full」时量到的是错的元素
            // （543px 的触发器读成 120px），菜单宽度因此永远对不上框。
            "max-w-[calc(100vw-24px)]",
            glide && "jolu-select--glide",
          )}
          style={{
            maxHeight: "min(248px, var(--radix-dropdown-menu-content-available-height))",
            // 下拉与框等宽：Radix 把触发器宽度挂在 --radix-dropdown-menu-trigger-width。
            // filter 变体的触发器是 w-full（撑满外层 label），所以这里拿到的
            // 就是外层视觉框的宽度 —— 日期、状态两个筛选条都因此对齐。
            width: menuWidth ? `${menuWidth}px` : "var(--radix-dropdown-menu-trigger-width)",
          }}
        >
          {glide ? (
            <GlideGroup uid={uid} value={value} options={options} onValueChange={onValueChange} searchable={searchable} query={query} onQueryChange={setQuery} />
          ) : (
            <Menu.RadioGroup value={value} onValueChange={onValueChange} className="p-1">
              {options.map((option) => (
                <Menu.RadioItem key={option.value} value={option.value} disabled={option.disabled}
                  textValue={option.label}
                  className="relative flex min-h-7 cursor-default select-none items-center gap-2 rounded-[6px] py-1 pl-3 pr-7 text-xs outline-none data-[highlighted]:bg-surface-3 data-[state=checked]:bg-surface-3 data-[disabled]:opacity-40"
                >
                  {option.icon && <span aria-hidden className="grid size-4 shrink-0 place-items-center">{option.icon}</span>}
                  <span className="min-w-0">
                    <span className="block break-words">{option.label}</span>
                    {option.description && <span className="mt-px block max-w-[290px] whitespace-normal text-[12px] text-text-secondary" title={option.description}>{option.description}</span>}
                  </span>
                  <Menu.ItemIndicator className="absolute right-2"><Check aria-hidden className="size-3" /></Menu.ItemIndicator>
                </Menu.RadioItem>
              ))}
            </Menu.RadioGroup>
          )}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

/**
 * 滑块分组：Radix 的 RadioGroup 负责语义与键盘，这里只负责把「当前高亮行」
 * 翻译成一个 translateY。行高由 CSS 变量统一（--jolu-row-h），
 * 支持标签带 description 的双行项（用 getBoundingClientRect 实测，不依赖固定行高）。
 */
function GlideGroup({ uid, value, options, onValueChange, searchable, query, onQueryChange }: {
  /** 本实例的唯一标识：同页可能同时有多个 SelectField（模型路由页有日期 + 状态两个），
      用 document.querySelector(".jolu-select-glide") 会操作到别人的滑块，位置与可见性全乱 */
  uid: string
  value: string
  options: SelectOption[]
  onValueChange: (value: string) => void
  searchable?: boolean
  query?: string
  onQueryChange?: (next: string) => void
}) {
  /* 搜索只过滤显示：过滤后若当前选中项被隐藏，滑块就没有落点，此时隐藏滑块 */
  const q = (query ?? "").trim().toLowerCase()
  const visible = q
    ? options.filter((o) => o.label.toLowerCase().includes(q) || o.value.toLowerCase().includes(q))
    : options
  const place = useCallback((target: HTMLElement | null, jump: boolean) => {
    // 用自己的 uid 精确找本组件的菜单容器。
    // 同页多个 SelectField 时，用 document.querySelector(".jolu-select-glide")
    // 会拿到别人的滑块 —— 模型路由页（日期 + 状态两个筛选）就是这样坏的。
    const wrap = document.querySelector<HTMLElement>(`[data-glide-owner="${uid}"]`)
    const pill = wrap?.querySelector<HTMLElement>(".jolu-select-glide__pill") ?? null
    if (!wrap || !pill) return
    /**
     * 位置与可见性写成 wrap 上的 CSS 变量，而不是 pill 的内联 style：
     * pill 是 React 渲染的节点，每次 GlideGroup 重渲染（切选中值、Radix 更新
     * aria-checked）都会把它手写的内联 style 清回 CSS 的 opacity:0。
     * wrap 的 style 由我们自己管，React 不碰，所以变量能存活。
     */
    if (!target) {
      wrap.style.setProperty("--gs-pill-y", "0px")
      wrap.style.setProperty("--gs-pill-h", "0px")
      wrap.style.setProperty("--gs-pill-o", "0")
      return
    }
    const w = wrap.getBoundingClientRect()
    const t = target.getBoundingClientRect()
    if (jump) wrap.dataset.jump = ""
    else delete wrap.dataset.jump
    wrap.style.setProperty("--gs-pill-y", `${t.top - w.top}px`)
    wrap.style.setProperty("--gs-pill-h", `${t.height}px`)
    // 水平位置与宽度按目标行的实际矩形量 —— 容器有 p-1 内边距，
    // 只写 left:0/right:0 会让滑块比绿色选中底两边各窄 4px。
    wrap.style.setProperty("--gs-pill-x", `${t.left - w.left}px`)
    wrap.style.setProperty("--gs-pill-w", `${t.width}px`)
    wrap.style.setProperty("--gs-pill-o", "1")
  }, [])

  /**
   * 挂载后把滑块放到选中行。
   * Radix 的菜单内容是异步挂载的（开启动画期间还在渲染），而且
   * data-state="checked" 也比挂载晚一帧，所以这里带重试：
   * 最多等 20 帧（≈330ms），直到找到行并量出非零高度为止。
   */
  useLayoutEffect(() => {
    const wrap = document.querySelector<HTMLElement>(`[data-glide-owner="${uid}"]`)
    if (!wrap) return
    let raf = 0
    let tries = 0
    const tryPlace = () => {
      const row =
        wrap.querySelector<HTMLElement>('[data-state="checked"]') ??
        wrap.querySelector<HTMLElement>("[data-glide-value]")
      const h = row?.getBoundingClientRect().height ?? 0
      if (row && h > 0) {
        place(row, true)
        wrap.style.setProperty("--gs-pill-o", "0")
        return
      }
      if (tries++ < 20) raf = requestAnimationFrame(tryPlace)
    }
    raf = requestAnimationFrame(tryPlace)
    return () => cancelAnimationFrame(raf)
  }, [place])

  /**
   * 滑块跟随「当前高亮的行」。
   *
   * 鼠标：自己监听行的 pointerover（会冒泡，绑在 wrap 上就能收到）。
   * 不能用 pointerenter —— 它不冒泡，绑在容器上收不到子元素的事件。
   * 键盘：Radix 改 data-highlighted 属性，属性变化不触发事件，
   * 所以另用 MutationObserver 盯住它。两条通路都让滑块动。
   *
   * 指针离开菜单时 Radix 会清掉 data-highlighted，但滑块**不回弹**：
   * 停在最后停留的那一行（参考实现里的 rememberPosition）。
   * 只有重新打开菜单时才回到选中行。
   */
  useEffect(() => {
    const wrap = document.querySelector<HTMLElement>(".jolu-select-glide")
    if (!wrap) return
    let raf = 0
    let lastKeyAt = 0
    /** 最后停留的行。指针离开后滑块停在这里，不回弹到选中行 */
    let lastRow: HTMLElement | null = null

    const apply = (row: HTMLElement | null, jump: boolean) => {
      cancelAnimationFrame(raf)
      raf = requestAnimationFrame(() => {
        place(row, jump)
        // 滑块停在「选中行」上时隐藏：那里已经有原来的绿色选中底，
        // 叠一块灰的会变脏。指针移到别的行时才露出滑块。
        const onChecked = row !== null && row.getAttribute("data-state") === "checked"
        wrap.style.setProperty("--gs-pill-o", onChecked ? "0" : "1")
      })
    }

    /* 主通路：自己监听行的 pointerover —— 不依赖 Radix 的 data-highlighted，
       因为它何时写这个属性取决于内部焦点状态（菜单刚开、焦点在外时可能不写）。
       用 pointerover 而不是 pointerenter：后者不冒泡，绑在 wrap 上收不到行的事件。 */
    const onOver = (e: Event) => {
      const row = (e.target as HTMLElement).closest<HTMLElement>("[data-glide-value]")
      if (!row || row.hasAttribute("data-disabled")) return
      if (row === lastRow) return
      lastRow = row
      apply(row, false)   // 悬停：滑动过去
    }
    wrap.addEventListener("pointerover", onOver)

    /* 指针离开整个列表：停在最后停留的那一行，不回弹（参考实现的 rememberPosition） */
    const onLeave = (e: Event) => {
      const related = (e as PointerEvent).relatedTarget as Node | null
      if (related && wrap.contains(related)) return
      if (lastRow) apply(lastRow, false)
    }
    wrap.addEventListener("pointerleave", onLeave)

    /* 键盘导航：Radix 会写 data-highlighted，用 MutationObserver 跟上（瞬移，连按不排队） */
    const observer = new MutationObserver(() => {
      if (performance.now() - lastKeyAt > 300) return
      const highlighted = wrap.querySelector<HTMLElement>('[data-highlighted]:not([data-disabled])')
      if (highlighted) { lastRow = highlighted; apply(highlighted, true) }
    })
    observer.observe(wrap, { subtree: true, attributes: true, attributeFilter: ["data-highlighted"] })

    const onKey = () => { lastKeyAt = performance.now() }
    wrap.addEventListener("keydown", onKey)

    return () => {
      cancelAnimationFrame(raf)
      observer.disconnect()
      wrap.removeEventListener("pointerover", onOver)
      wrap.removeEventListener("pointerleave", onLeave)
      wrap.removeEventListener("keydown", onKey)
    }
  }, [place])

  return (
    <div className="jolu-select-glide p-1" data-glide-owner={uid}>
      {searchable ? (
        <label className="mb-1 flex h-8 shrink-0 items-center gap-2 rounded-[6px] bg-surface-3 px-2.5">
          <Search aria-hidden className="size-3.5 shrink-0 text-text-secondary" />
          <input
            value={query ?? ""}
            onChange={(e) => onQueryChange?.(e.target.value)}
            // 输入时不触发 Radix 的 typeahead / 方向键导航
            onKeyDown={(e) => { if (e.key !== "Escape") e.stopPropagation() }}
            placeholder="搜索"
            aria-label="搜索选项"
            className="min-w-0 flex-1 bg-transparent text-xs text-text-primary outline-none placeholder:text-text-secondary"
          />
        </label>
      ) : null}
      <Menu.RadioGroup
        value={value}
        onValueChange={onValueChange}
        className="jolu-select-glide__rows"
      >
        {visible.map((option) => (
          <Menu.RadioItem key={option.value} value={option.value} disabled={option.disabled}
            textValue={option.label}
            data-glide-value={option.value}
            className="jolu-select-glide__row"
          >
            {option.icon && <span aria-hidden className="grid size-4 shrink-0 place-items-center">{option.icon}</span>}
            <span className="min-w-0">
              <span className="block break-words">{option.label}</span>
              {option.description && <span className="mt-px block max-w-[290px] whitespace-normal text-[12px] text-text-secondary" title={option.description}>{option.description}</span>}
            </span>
            <Menu.ItemIndicator className="absolute right-2"><Check aria-hidden className="size-3" /></Menu.ItemIndicator>
          </Menu.RadioItem>
        ))}
        {visible.length === 0 ? <div className="px-3 py-2 text-xs text-text-secondary">没有匹配项</div> : null}
      </Menu.RadioGroup>
      {/* 滑块放在行的后面：绝对定位，靠 z-index 沉到文字下面 */}
      <span className="jolu-select-glide__pill" aria-hidden />
    </div>
  )
}
