# ZCode 3.18 凭据封装（`enc:v2:os:`）支持与验收

日期：2026-11-13
范围：`backend/src/zcode_quota.rs`、`backend/Cargo.toml`（新增 `Win32_Security_Credentials` feature）

## 1. 问题

ZCode 升级到 3.18 后，本机 `~/.zcode/v2/credentials.json` 里的 10 条登录凭据全部变成
`enc:v2:os:…` 封装，Jolu 只认 `enc:v1:`，因此：

- 账号管理里 ZCode 显示「无法解密当前用户的 ZCode 登录」；
- 额度读不出来（Start Plan、个人订阅余额都依赖同一份凭据）；
- 网关转发 ZCode 订阅（`coding_plan_key_at`）、Start Plan 着装（`coding_plan_jwt_at`）
  全部拿不到 Key / JWT。

## 2. 官方实现（对照依据）

来源：zai-org/ZCode `packages/shared/src/credential-security/`

| 项 | 值 |
|---|---|
| 前缀 | 系统封装 `enc:v2:os:`，另有 file 封装（Jolu 不认，遇到即失败） |
| 密钥 | **随机 32 字节**，不是从机器信息派生 |
| 密钥存放 | 系统凭据管理器，service = `ZCode Credential Encryption` |
| keyring account | `sha256(path.resolve(credentials.json))` 的 hex |
| 条目名 | `<account>.<service>`（macOS keychain / Windows Credential Manager 后的实际 target） |
| blob 编码 | UTF-16LE 的 43 字符 base64url（Node keyring 把字符串按 UTF-16 交出） |
| 算法 | AES-256-GCM |
| IV / TAG | 12 / 16 字节 |
| 密文体 | `<24位hex指纹>:<iv>.<tag>.<ciphertext>`，三段 base64url 无填充 |
| 指纹 | `sha256(key)` 的前 12 字节 hex，用于换机器/换目录时快速判错 |

旧版 `enc:v1:` 保持不变：密钥 = `sha256(credential_secret(home))`，secret 由机器与路径派生。

## 3. 实现

`backend/src/zcode_quota.rs`：

- `decrypt_credential` 增加 `enc:v2:os:` 分支：先判形状（24 位指纹 + 冒号），再读密钥；
- `os_credential_key()`（`cfg(windows)`）：`CredReadW(target, CRED_TYPE_GENERIC, 0, &mut raw)`
  → 拷出 `CredentialBlob` → `CredFree`；target = `<sha256hex(credentials.json 绝对路径)>.<service>`；
- `decode_key_blob()`：UTF-16LE 优先（偶数字节且奇位全 0）、退 UTF-8；必须 43 字符 base64url
  且解出正好 32 字节；
- `decrypt_v2()`：指纹比对后走同一个 `decrypt_gcm`；
- 非 Windows 返回 `Err(())`（v2 封装只在 Windows 上用系统凭据）；file 封装与未知 `enc:` 一律失败。

`backend/Cargo.toml`：`windows-sys` features 增加 `Win32_Security_Credentials`。

只读纪律未变：只调用 `CredReadW`（读），不写 `~/.zcode`、不写凭据库；Key 原文不进日志、
错误文案、诊断与统计导出。

## 4. 测试

单元测试（`cargo test --lib zcode_quota`，29 passed）：

| 测试 | 覆盖 |
|---|---|
| `decrypts_official_envelope_and_rejects_tampering` | v1 正常解密、错口令、残缺三段、未知封装、明文直通 |
| `decrypts_v2_with_keyring_key_and_checks_fingerprint` | 合成 32 字节密钥的 v2 正常解密；换密钥（指纹不符）失败；密文被改（tag 校验）失败；`enc:v2:os:` 三种残缺形状在取密钥前被挡 |
| `key_blob_decodes_utf16_and_utf8_and_rejects_other_lengths` | UTF-16LE / UTF-8 两种 blob 编码都能解；长度不对（41 / 空 / 31 字节密钥）一律失败 |

全量：`cargo test --lib` → **1380 passed / 0 failed / 46 ignored**。

## 5. 真机只读验收（本机 ZCode 3.18.2）

两个 `#[ignore]` 的只读 smoke，仅在显式核对时执行，不打印任何明文：

```
cargo test --lib local_v2_credentials_decrypt_readonly_smoke -- --ignored --nocapture
  entries=10 encrypted=10 v2=10 failed=0
  coding_plan_key=ok family=zai
  coding_plan_jwt=ok
  team_selection=none

cargo test --lib local_quota_readonly_smoke -- --ignored --nocapture
  state=ok windows=0 plan=Some("ZCODE WEEKEND BUILD") email=Some("l***@o***.com")
```

结论：

- 10 条凭据全部为 v2，全部解密成功；
- 网关转发入口 `coding_plan_key_at` 拿到 zai 家族的 Coding Plan Key；
- Start Plan 的会话 JWT 可读；
- 额度端点返回 `state=ok`，套餐名正常，邮箱按掩码 `l***@o***.com` 输出；
- `team_selection=none` 符合预期：本机是个人套餐，不是团队坐席。

`windows=0` 表示该套餐没有窗口式额度（周套餐），与 `plan` 同时存在，界面按「有套餐、无窗口」表达。

## 6. 未做 / 待确认

- **`ZCODE_APP_VERSION` 仍为 `3.17.2`**（本机 3.18.2）。额度端点在该版本号下已返回正常结果，
  但升版本要同时与 `gateway/zcode_start_prompt.rs` 的系统提示文本、`zcode_start.rs` 的 UA
  逐字节对齐，属独立变更，未在本轮改动。
- 非 Windows 平台没有这套系统凭据库，v2 登录读不出来；目前 Jolu 只发 Windows 包。
- file 封装（`enc:v2:file:`）未实现：本机没有样本，也不清楚其密钥来源，遇到即按失败处理。
