import { useEffect, useState } from "react"
import { invoke, type AppSettings } from "@/lib/api"
import { settingsApi } from "@/lib/settingsApi"
import { Banner, Checkbox } from "@/ui"

/**
 * ZCode 通道顺序（全局默认值；原挂在「管理模型来源」弹窗，弹窗拆除后迁入模型池底部）。
 *
 * 后端语义（`gateway/adapter/zcode.rs`）：关 = 先走 Coding Plan Key，余额 / 网络类失败再落免费
 * Start Plan；开 = 先试 Start Plan，失败再走 Coding Plan。每个连接还会记住上次成功的通道 10 分钟。
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
    <section className="flex flex-col gap-3 rounded-jn-card border border-jn-line bg-jn-surface p-4 shadow-jn-surface">
      {version?.newer && (
        <Banner tone="danger">
          本机 ZCode {version.installed} 比 Jolu 适配的 {version.expected} 新：免费 Start Plan 可能被服务商拒绝（405 / 3012），等待 Jolu 更新适配。
        </Banner>
      )}
      <div className="flex items-start gap-3">
        <div className="pt-0.5">
          <Checkbox
            checked={on}
            disabled={busy}
            label="ZCode 先用免费 Start Plan"
            onChange={(next) => {
              setOn(next)
              void run(() => settingsApi.zcodeFreeFirstSet(next), next ? "ZCode 改为先试免费 Start Plan" : "ZCode 改为先走 Coding Plan").then((ok) => {
                if (!ok) setOn(!next)
              })
            }}
          />
        </div>
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            const next = !on
            setOn(next)
            void run(() => settingsApi.zcodeFreeFirstSet(next), next ? "ZCode 改为先试免费 Start Plan" : "ZCode 改为先走 Coding Plan").then((ok) => {
              if (!ok) setOn(!next)
            })
          }}
          className="flex min-w-0 flex-1 flex-col gap-0.5 text-left outline-none disabled:opacity-45"
        >
          <span className="text-[13px] font-semibold text-jn-text">ZCode 先用免费 Start Plan</span>
          <span className="text-xs leading-relaxed text-jn-text-3">
            开启后 ZCode 来源先试免费额度，用完或失败再走 Coding Plan；关闭则先走 Coding Plan。需要本机已登录 ZCode。
          </span>
        </button>
      </div>
    </section>
  )
}
