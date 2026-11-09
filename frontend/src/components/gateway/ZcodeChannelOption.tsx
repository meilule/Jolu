import { useEffect, useState } from "react"
import { invoke, type AppSettings } from "@/lib/api"
import { settingsApi } from "@/lib/settingsApi"

/**
 * ZCode 通道顺序（待确认：Jolu.pen 无画板，沿用「路由分组」折叠区的行样式）。
 *
 * 后端语义（`gateway/adapter/zcode.rs`）：关 = 先走 Coding Plan Key，余额 / 网络类失败再落免费
 * Start Plan；开 = 先试 Start Plan，失败再走 Coding Plan。每个连接还会记住上次成功的通道
 * 10 分钟，所以切换后已有记忆的连接最迟 10 分钟按新顺序探测。
 */
export function ZcodeChannelOption({ busy, run }: {
  busy: boolean
  run: (work: () => Promise<unknown>, ok?: string) => Promise<boolean>
}) {
  const [on, setOn] = useState<boolean | null>(null)
  const [version, setVersion] = useState<{ installed: string | null; expected: string; newer: boolean } | null>(null)
  useEffect(() => {
    invoke<AppSettings>("get_settings").then((settings) => setOn(Boolean(settings.zcode_free_first))).catch(() => setOn(null))
    invoke<{ installed: string | null; expected: string; newer: boolean }>("zcode_version_check").then(setVersion).catch(() => setVersion(null))
  }, [])
  if (on === null) return null
  return (
    <>
    {version?.newer && (
      <div className="rounded-lg bg-jolu-danger-soft px-3 py-2 text-[9px] text-jolu-danger">
        本机 ZCode {version.installed} 比 Jolu 适配的 {version.expected} 新：免费 Start Plan 可能被服务商拒绝（405 / 3012），等待 Jolu 更新适配。
      </div>
    )}
    <label className="flex items-start gap-2.5 rounded-lg border border-jolu-border px-3 py-2.5">
      <input type="checkbox" checked={on} disabled={busy} className="mt-0.5 size-3.5 accent-jolu-accent"
        onChange={(event) => {
          const next = event.target.checked
          setOn(next)
          void run(() => settingsApi.zcodeFreeFirstSet(next), next ? "ZCode 改为先试免费 Start Plan" : "ZCode 改为先走 Coding Plan").then((ok) => { if (!ok) setOn(!next) })
        }} />
      <span className="min-w-0 flex-1">
        <span className="block text-[10.5px] font-medium text-jolu-text">ZCode 先用免费 Start Plan</span>
        <span className="block text-[8.5px] text-jolu-muted">
          开启后 ZCode 来源先试免费额度，用完或失败再走 Coding Plan；关闭则先走 Coding Plan。需要本机已登录 ZCode。
        </span>
      </span>
    </label>
    </>
  )
}
