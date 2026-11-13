import assert from 'node:assert/strict'
import test from 'node:test'
import { parseAppNotice, noticeToast, formatNoticeAmount } from './appNotices.ts'
import {pushToast} from './toastQueue.ts'

const balance = { id:1,kind:'balance',connection_id:'synthetic-api',connection_name:'开发连接',platform:'claude',remaining:8.5,currency:'USD',threshold:10 }
test('only valid finite same-currency balance and known task states are accepted',()=>{
  assert.deepEqual(parseAppNotice(balance),balance)
  for(const patch of [{remaining:NaN},{remaining:Infinity},{currency:'usd'},{currency:'USD<script>'},{threshold:0},{remaining:10},{connection_id:''},{id:0},{id:1.5}]) assert.equal(parseAppNotice({...balance,...patch}),null)
  for(const state of ['resumed','completed','failed']) assert.ok(parseAppNotice({id:2,kind:'task',task_id:7,state,platform:'Claude Code',task_name:'测试任务',reason:state==='failed'?'查询失败':null}))
  assert.equal(parseAppNotice({id:2,kind:'task',task_id:-1,state:'completed',platform:'Claude Code',task_name:'任务'}),null)
})
test('notice amount preserves currency, zero and negative balances',()=>{
  assert.equal(formatNoticeAmount(8.5,'USD'),'$8.50')
  assert.equal(formatNoticeAmount(0,'CNY'),'¥0.00')
  assert.equal(formatNoticeAmount(-1.25,'USD'),'-$1.25')
  assert.equal(formatNoticeAmount(1234,'EUR'),'1,234.00')
})
test('balance and task cards have truthful actions and tones',()=>{
  const calls=[];const open={account:id=>calls.push(['account',id]),task:id=>calls.push(['task',id])}
  const card=noticeToast(balance,open)
  assert.equal(card.tone,'warn');assert.equal(card.balance.remaining,8.5);assert.equal(card.action.label,'查看账号')
  card.action.onClick()
  for(const [state,tone] of [['resumed','info'],['completed','success'],['failed','danger']]){
    const card=noticeToast({id:2,kind:'task',task_id:7,state,platform:'Claude Code',task_name:'测试任务',reason:state==='failed'?'查询失败':null},open)
    assert.equal(card.tone,tone);card.action.onClick()
  }
  assert.deepEqual(calls,[['account','synthetic-api'],['task',7],['task',7],['task',7]])
})
test('identical names and amounts from different accounts or tasks never merge their actions',()=>{
  const open={account:()=>{},task:()=>{}}
  let list=pushToast([],noticeToast(balance,open),1,0)
  list=pushToast(list,noticeToast({...balance,id:2,connection_id:'another-api'},open),2,0)
  assert.equal(list.length,2)
  const task={id:3,kind:'task',task_id:7,state:'completed',platform:'Claude Code',task_name:'same name'}
  list=pushToast([],noticeToast(task,open),3,0)
  list=pushToast(list,noticeToast({...task,id:4,task_id:8},open),4,0)
  assert.equal(list.length,2)
})

test("额度主动提醒（P2-2）：只认三种种类，点击打开账号", () => {
  const notice = parseAppNotice({ id: 7, kind: "quota", connection_id: "c1", connection_name: "工作账号", platform: "claude", alert: "low", detail: "5 小时剩余 12%" })
  assert.ok(notice)
  let opened = null
  const toast = noticeToast(notice, { account: (id) => { opened = id }, task: () => {} })
  assert.equal(toast.title, "额度快用完了")
  assert.equal(toast.tone, "warn")
  toast.action.onClick()
  assert.equal(opened, "c1")
  assert.equal(parseAppNotice({ id: 8, kind: "quota", connection_id: "c1", connection_name: "x", platform: "claude", alert: "other", detail: "d" }), null)
})
