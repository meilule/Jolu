/**
 * 静态快照入口。
 *
 * 顺序很重要：先装假 IPC，再动态导入业务模块——`lib/api.ts` 的 `isTauri` 是模块级常量，
 * 必须在导入时就能看到 `__TAURI_INTERNALS__`，否则整棵树会走浏览器示例分支。
 */
import { emitSnapshotEvent, installFakeTauri, unknownCommands } from "./fakeTauri"
import { routeCommand } from "./snapshotCommands"

installFakeTauri((cmd, args) => {
  const value = routeCommand(cmd, args)
  if (value === undefined) {
    unknownCommands.add(cmd)
    // 自检脚本与快照页脚据此显示漏登记的命令
    ;(window as unknown as Record<string, unknown>).__SNAPSHOT_UNKNOWN__ = [...unknownCommands]
    // 未登记的命令：写命令当成功（null），读命令给空数组，调用方不会因 undefined 崩
    return cmd.startsWith("set_") || cmd.includes("_set") ? null : []
  }
  // 设置类写命令返回完整设置快照：广播给 useSettings 的共享 store，开关才不会回弹
  if (value && typeof value === "object" && "revision" in value && "island_platform" in value) {
    queueMicrotask(() => emitSnapshotEvent("settings-changed", value))
  }
  return value
})

void import("./mount").then(({ mountSnapshot }) => mountSnapshot())
