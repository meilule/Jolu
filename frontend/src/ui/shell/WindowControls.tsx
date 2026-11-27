/**
 * WindowControls —— 设计稿 R1-C「分区 · 1 · 外壳 / 页头」右端的三枚窗口按钮。
 *
 * 规格取自画板：每枚 32×32、圆角 10、彼此间距 2。
 *
 * ## 为什么单独成一个组件
 *
 * 旧外壳把窗口按钮放在独立标题栏（`components/panel/Chrome.tsx`），新外壳按
 * 画板把它们移进页头右端。逻辑（最小化 / 最大化切换 / 关闭并隐藏）与新外观
 * 分开放：这里只负责外观与调用，替换期间两套外壳都能用同一份行为。
 *
 * ## 关闭是「隐藏」不是「退出」
 *
 * 后端拦截关闭请求，只把主面板隐藏；灵动岛与后台采集继续运行，托盘再次打开时秒开。
 * 这一点由主窗口的 `closeRequested` 处理，这里只调 `win.close()`。
 */
import { useEffect, useState } from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { Icon, type IconName } from "@/icons"
import { TactileButton } from "@/motion"

const ACTIONS: { action: "minimize" | "maximize" | "close"; label: string; icon: IconName }[] = [
  { action: "minimize", label: "最小化", icon: "minimize" },
  { action: "maximize", label: "最大化", icon: "maximize" },
  { action: "close", label: "关闭主面板", icon: "close" },
]

export function WindowControls({ className }: { className?: string }) {
  const [maximized, setMaximized] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // 初始状态从窗口读一次：直接在浏览器里渲染（无 Tauri）时跳过。
  useEffect(() => {
    if (!isTauri) return
    void getCurrentWindow()
      .isMaximized()
      .then(setMaximized)
      .catch(() => {})
  }, [])

  const act = async (action: "minimize" | "maximize" | "close") => {
    if (!isTauri) return
    setError(null)
    try {
      const win = getCurrentWindow()
      if (action === "minimize") await win.minimize()
      else if (action === "maximize") {
        await win.toggleMaximize()
        setMaximized(await win.isMaximized())
      } else await win.close()
    } catch {
      setError("窗口操作失败，请重试")
    }
  }

  return (
    <div className={cn("flex shrink-0 items-center gap-0.5 pl-2", className)}>
      {error && (
        <span role="alert" className="mr-1 text-[11px] text-jn-danger">
          {error}
        </span>
      )}
      {ACTIONS.map(({ action, label, icon }) => (
        <TactileButton
          key={action}
          variant="icon"
          type="button"
          aria-label={action === "maximize" && maximized ? "还原" : label}
          title={isTauri ? (action === "maximize" && maximized ? "还原" : label) : `${label}（仅桌面端可用）`}
          disabled={!isTauri}
          onClick={() => void act(action)}
          className={cn(
            "jn-interactive flex size-8 items-center justify-center rounded-[10px] text-jn-text-2 disabled:pointer-events-none disabled:opacity-45",
            // 关闭键悬停变红：三枚按钮里只有它是「消失」，需要区分出来
            action === "close" ? "hover:bg-jn-danger-soft hover:text-jn-danger" : "hover:bg-jn-well",
          )}
        >
          {/* 最大化后图标换成「还原」语义：与 aria-label 保持一致 */}
          <Icon
            name={action === "maximize" && maximized ? "restore" : icon}
            size={action === "maximize" ? "sm" : "md"}
          />
        </TactileButton>
      ))}
    </div>
  )
}
