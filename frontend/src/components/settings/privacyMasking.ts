/**
 * 隐私脱敏（P5）的纯逻辑：规则清单与开关的乐观显示值。
 * 引擎与规则都在后端 gateway/mask.rs，这里只承载界面如实展示的内容，不做规则扩展。
 */

/** 开关显示值：点了还没保存成功时显示目标态，保存失败回滚到后端保存值 */
export function maskingDisplay(saved: boolean, applied: boolean | null): boolean {
  return applied ?? saved
}

export interface MaskingRule {
  name: string
  hint: string
}

/** 脱敏范围（固定规则，只读展示；后端是唯一事实来源，界面不提供增删入口） */
export const PRIVACY_MASKING_RULES: readonly MaskingRule[] = [
  { name: "API Key", hint: "sk- 等已知前缀的密钥" },
  { name: "私钥 PEM 段", hint: "-----BEGIN … PRIVATE KEY-----" },
  { name: "Bearer 令牌", hint: "Bearer / Basic 后的令牌" },
  { name: "token= 凭证", hint: "token=、api_key= 等键值对" },
  { name: "邮箱", hint: "邮箱地址" },
  { name: "银行卡", hint: "通过 Luhn 校验的卡号" },
  { name: "身份证号", hint: "18 位身份证" },
]

/** 占位符示例，格式与后端一致：{{JOLU_<类型>_<8位hash>}}；同值同占位符，不破坏 prompt cache */
export const MASKING_PLACEHOLDER_SAMPLE = "{{JOLU_APIKEY_1a2b3c4d}}"
