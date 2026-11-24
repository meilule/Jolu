import { CHECKIN_EDITIONS } from "@/lib/dailyCheckin"
import { connectionProviderId, type ProviderId } from "@/lib/providerCatalog"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { Banner, Button, Dialog } from "@/ui"

export function CheckinProviderDialog({ onClose, onPick }: { onClose: () => void; onPick: (provider: ProviderId) => void }) {
  return (
    <Dialog
      open
      onOpenChange={(next) => { if (!next) onClose() }}
      width="lg"
      title="添加账号"
      description="选择供应商与版本，继续连接账号。"
      actions={<Button kind="ghost" size="md" onClick={onClose}>取消</Button>}
    >
      <div className="grid grid-cols-1 gap-3 min-[520px]:grid-cols-2">
        {CHECKIN_EDITIONS.map((entry) => {
          const provider = connectionProviderId({ platformId: entry.platform, edition: entry.edition })
          return (
            <button
              key={entry.id}
              type="button"
              aria-label={`${entry.name} ${entry.region}`}
              disabled={!entry.manual || !provider}
              onClick={() => provider && onPick(provider)}
              className="jn-interactive flex min-h-[100px] min-w-0 flex-col gap-2 rounded-[16px] border border-jn-line bg-jn-well p-4 text-left outline-none hover:border-jn-primary hover:bg-jn-primary-wash disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:border-jn-line disabled:hover:bg-jn-well"
            >
              <span className="flex items-center gap-2.5 text-sm font-semibold text-jn-text">
                <PlatformLogo platform={entry.platform} size={22} />
                <span>{entry.name} {entry.region}</span>
              </span>
              <span className="text-xs leading-relaxed text-jn-text-3">{entry.description}</span>
            </button>
          )
        })}
      </div>
    </Dialog>
  )
}

export function CheckinRemoveAccountDialog({ account, busy, error, onClose, onConfirm }: {
  account: string
  busy: boolean
  error: string | null
  onClose: () => void
  onConfirm: () => void
}) {
  return (
    <Dialog
      open
      onOpenChange={(next) => { if (!next && !busy) onClose() }}
      preventClose={busy}
      width="sm"
      title="删除账号？"
      description={<span className="break-all font-mono">{account}</span>}
      actions={
        <>
          <Button kind="ghost" size="md" disabled={busy} onClick={onClose}>取消</Button>
          <Button kind="danger" size="md" icon="delete" loading={busy} disabled={busy} onClick={onConfirm}>
            {busy ? "正在删除…" : "删除账号"}
          </Button>
        </>
      }
    >
      <Banner tone="warning">
        将从 Jolu 移除此账号连接及保存的凭据，签到记录保留。远端账号与原工具登录不受影响。
      </Banner>
      {error && (
        <Banner tone="danger">
          <span role="alert" className="break-words">{error}</span>
        </Banner>
      )}
    </Dialog>
  )
}
