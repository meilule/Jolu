/**
 * 设置快照的修订号把关（P1「事件序号」的设置部分）。
 *
 * 后端每次保存成功把修订号加一，`get_settings` 与 `settings-changed` 都带上它；发送时现读快照，
 * 但多个线程发出的推送仍可能乱序到达。规则：修订号**不比当前小**才接受——相等也接受（同一份快照
 * 重复到达无害）；没有修订号的（旧后端、保存命令的返回值）不在这里判，交给调用方原有的把关。
 */
export function acceptsRevision(applied: number, incoming: number | undefined): boolean {
  return incoming === undefined || incoming >= applied
}

/** 接受后新的「已应用修订号」：没有修订号的快照不改变它。 */
export function nextApplied(applied: number, incoming: number | undefined): number {
  return incoming === undefined ? applied : Math.max(applied, incoming)
}
