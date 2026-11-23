"""P0-3 按请求 ID 的多平台逐条对账（只读）。

对每个平台，把「源端账本」与 Jolu `usage.db` 按稳定 ID 取交集，逐字段比对，
打印与 docs/verification/2026-11-23-p0-3-multi-platform-reconciliation.md 同格式的表格。

用法：
    python -I scripts/acceptance/reconcile-usage-by-id.py
    python -I scripts/acceptance/reconcile-usage-by-id.py --platform claude
    python -I scripts/acceptance/reconcile-usage-by-id.py --json > output/acceptance/reconcile.json

约束：
- 全程只读。源端与 Jolu 库都用 `mode=ro` 打开，不触发扫描、不写任何文件。
- 不读取消息正文，只读用量字段与 ID。Qoder 的 `content` / `answer` 一律不碰。
- 字段比对复刻 `db::insert_records_into` 的合并语义（见报告 §0），不是原始行比对。

为什么必须复刻合并语义：源端同一条消息可能出现多行（流式更新），Jolu 保存的是
「首次插入 + 后续 max + 只有值变化才更新 ts」。直接拿源端最后一行比会得到大量假差异。
"""
import argparse
import collections
import datetime
import glob
import json
import os
import sqlite3
import sys

RO = "mode=ro"


def open_ro(path):
    if not os.path.isfile(path):
        return None
    return sqlite3.connect(f"file:{path}?{RO}", uri=True)


def jolu_db_path():
    return os.path.join(os.environ["APPDATA"], "dev.ningz.jolu", "usage.db")


def ms_text(value):
    return datetime.datetime.fromtimestamp(value / 1000).strftime("%Y-%m-%d %H:%M:%S")


def non_negative(value):
    return value if isinstance(value, int) and value >= 0 else None


def merge_into(state, incoming, token_keys):
    """复刻 db::insert_records_into：token 取 max，ts 只在值变化时取 max。"""
    if state is None:
        return dict(incoming)
    changed = False
    for key in token_keys:
        old, new = state.get(key), incoming.get(key)
        merged = max(old, new) if (old is not None and new is not None) else (old if new is None else new)
        if merged != old:
            state[key] = merged
            changed = True
    if changed:
        state["ts"] = max(state["ts"], incoming["ts"])
    return state


# requests 表的列：值列是 `<x>_tokens`，但已知标记列去掉了 `_tokens`
# （`input_known` / `cache_read_known` …），必须显式配对，不能靠拼字符串。
TOKEN_COLUMNS = [
    ("input_tokens", "input", "input_known"),
    ("output_tokens", "output", "output_known"),
    ("cache_read_tokens", "read", "cache_read_known"),
    ("cache_write_tokens", "write", "cache_write_known"),
    ("total_tokens", "total", "total_known"),
]


def jolu_rows(conn, platform):
    """取 Jolu 侧的用量行，短名与源端解析结果对齐（input / output / read / write / total）。"""
    selects = ", ".join(
        [f"{value} AS {alias}" for value, alias, _ in TOKEN_COLUMNS]
        + [f"{known} AS {alias}_known" for _, alias, known in TOKEN_COLUMNS]
    )
    names = ["dedup_key", "ts"] + [alias for _, alias, _ in TOKEN_COLUMNS]         + [f"{alias}_known" for _, alias, _ in TOKEN_COLUMNS]
    rows = conn.execute(f"SELECT dedup_key, ts, {selects} FROM requests WHERE platform = ?", (platform,))
    return {row[0]: dict(zip(names, row)) for row in rows}


TOKEN_FIELDS = ["input", "output", "read", "write"]


def compare(source, jolu, source_label):
    inter = set(source) & set(jolu)
    diffs = collections.Counter()
    samples = []
    for key in inter:
        left, right = source[key], jolu[key]
        if left["ts"] != right["ts"]:
            diffs["ts"] += 1
            if len(samples) < 3:
                samples.append((key, left["ts"], right["ts"], right["ts"] - left["ts"]))
        for field in TOKEN_FIELDS + ["total"]:
            value = left.get(field)
            jv = right.get(field)
            known = bool(right.get(f"{field}_known", 1))
            if value is None:
                if known:
                    diffs[field] += 1
            elif not known or jv != value:
                diffs[field] += 1
    return {
        "source": source_label,
        "source_ids": len(source),
        "jolu_rows": len(jolu),
        "intersection": len(inter),
        "only_jolu": len(set(jolu) - set(source)),
        "only_source": len(set(source) - set(jolu)),
        "field_diffs": {k: v for k, v in diffs.items() if v},
        "ts_samples": samples,
    }


# ---------------------------------------------------------------- Claude Code

def read_claude():
    """~/.claude/projects/**/*.jsonl，按 message.id 复刻合并语义。"""
    root = os.path.join(os.path.expanduser("~"), ".claude", "projects")
    if not os.path.isdir(root):
        return None
    source = {}
    for path in glob.glob(os.path.join(root, "**", "*.jsonl"), recursive=True):
        try:
            handle = open(path, encoding="utf-8")
        except OSError:
            continue
        with handle:
            for line in handle:
                line = line.strip()
                if not line:
                    continue
                try:
                    value = json.loads(line)
                except ValueError:
                    continue
                if value.get("type") != "assistant":
                    continue
                message = value.get("message") or {}
                usage = message.get("usage")
                if not isinstance(usage, dict):
                    continue
                ident = message.get("id")
                stamp = value.get("timestamp")
                if not isinstance(ident, str) or not isinstance(stamp, str):
                    continue
                try:
                    ts = int(datetime.datetime.fromisoformat(stamp.replace("Z", "+00:00")).timestamp() * 1000)
                except ValueError:
                    continue
                incoming = {
                    "ts": ts,
                    "input": non_negative(usage.get("input_tokens")),
                    "output": non_negative(usage.get("output_tokens")),
                    "read": non_negative(usage.get("cache_read_input_tokens")),
                    "write": non_negative(usage.get("cache_creation_input_tokens")),
                }
                source[ident] = merge_into(source.get(ident), incoming, TOKEN_FIELDS)
    for state in source.values():
        parts = [state.get(k) for k in TOKEN_FIELDS]
        state["total"] = sum(parts) if all(p is not None for p in parts) else None
    return source


def claude_jolu(conn):
    return jolu_rows(conn, "claude")


# ------------------------------------------------------------------- OpenCode

def read_opencode():
    """~/.local/share/opencode/opencode.db，复刻 opencode_usage::parse 的过滤与 fallback。"""
    path = os.path.join(os.path.expanduser("~"), ".local", "share", "opencode", "opencode.db")
    conn = open_ro(path)
    if conn is None:
        return None
    tables = {row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    # 生产代码 detect_layout 优先 session_message，但那只是结构探测；这里还要有数据才算数，
    # 否则一台只有旧 message 表的机器会被判成「源端 0 条」。
    table = None
    for candidate in ("session_message", "message"):
        if candidate in tables:
            if conn.execute(f"SELECT COUNT(*) FROM {candidate}").fetchone()[0] > 0:
                table = candidate
                break
    if table is None:
        conn.close()
        return None
    source = {}
    for ident, session, ts, data in conn.execute(f"SELECT id, session_id, time_created, data FROM {table}"):
        try:
            value = json.loads(data)
        except ValueError:
            continue
        role = value.get("role") or value.get("type")
        if role is not None and role != "assistant":
            continue
        tokens = value.get("tokens")
        if not isinstance(tokens, dict):
            continue

        def pick(*path_parts):
            cur = tokens
            for part in path_parts:
                if not isinstance(cur, dict):
                    return None
                cur = cur.get(part)
            return non_negative(cur)

        inp, visible, reasoning = pick("input"), pick("output"), pick("reasoning")
        read, write, total = pick("cache", "read"), pick("cache", "write"), pick("total")
        if all(x is None for x in (inp, visible, reasoning, read, write, total)):
            continue
        if total is None:
            parts = [inp, visible, reasoning, read, write]
            if all(p is not None for p in parts):
                total = sum(parts)
        source[f"{session}:{ident}"] = {
            "ts": ts,
            "input": inp,
            "output": (visible + reasoning) if (visible is not None and reasoning is not None) else None,
            "read": read,
            "write": write,
            "total": total,
        }
    conn.close()
    return source


# ---------------------------------------------------------------------- Qoder

def read_qoder():
    """~/.qoder/shared_client/cache/db/local.db 的 chat_message。

    只用用量字段与 ID，不读 content / answer / question / reasoning_content。
    """
    path = os.path.join(os.path.expanduser("~"), ".qoder", "shared_client", "cache", "db", "local.db")
    conn = open_ro(path)
    if conn is None:
        return None
    source = {}
    query = ("SELECT session_id, id, gmt_create, token_info FROM chat_message "
             "WHERE role = 'assistant' AND token_info IS NOT NULL "
             "AND token_info NOT IN ('', '{}')")
    for session, ident, ts, token_info in conn.execute(query):
        try:
            usage = json.loads(token_info)
        except ValueError:
            continue
        prompt = non_negative(usage.get("prompt_tokens"))
        cached = non_negative(usage.get("cached_tokens"))
        # Jolu 存**原始 prompt**（含缓存），缓存单列；新鲜输入由 includes_cache 的消费方扣。
        # 这里若先扣再比，会和生产代码对不上（缓存大的请求差值巨大）。
        inp = prompt
        source[f"{session}:{ident}"] = {
            "ts": ts,
            "input": inp,
            "output": non_negative(usage.get("completion_tokens")),
            "read": cached,
            "write": None,
            "total": (prompt + non_negative(usage.get("completion_tokens")))
            if (prompt is not None and non_negative(usage.get("completion_tokens")) is not None) else None,
        }
    conn.close()
    return source


def read_workbuddy():
    """`~/.workbuddy/projects/**/*.jsonl`（`native_parse::workbuddy` 的输入格式）。

    字段映射复刻 `native_parse::workbuddy`：
    - `id` 或 `messageId` → dedup_key 的一部分，`sessionId` → 会话
    - `providerData.usage.inputTokens` / `outputTokens` / `totalTokens`（优先），
      回退到 `providerData.rawUsage.prompt_tokens` / `completion_tokens` / `total_tokens`
    - 缓存读：`rawUsage.prompt_tokens_details.cached_tokens`（优先）→ `rawUsage.prompt_cache_hit_tokens`
      → `usage.inputTokensDetails[].cached_tokens`
    - 缓存写：`rawUsage.prompt_cache_write_tokens` 或 `cache_creation_input_tokens`
    - `rawUsage.prompt_tokens` 存在时 `input_semantics = includes_cache`（输入含缓存，新鲜输入要扣）

    没有客户端目录时返回 None（本机就是这种状态，见对账报告 §4）。
    """
    root = os.path.join(os.path.expanduser("~"), ".workbuddy", "projects")
    if not os.path.isdir(root):
        return None
    source = {}
    for path in glob.glob(os.path.join(root, "**", "*.jsonl"), recursive=True):
        try:
            handle = open(path, encoding="utf-8")
        except OSError:
            continue
        fallback = os.path.splitext(os.path.basename(path))[0]
        with handle:
            for line in handle:
                line = line.strip()
                if not line:
                    continue
                try:
                    value = json.loads(line)
                except ValueError:
                    continue
                pd = value.get("providerData")
                if not isinstance(pd, dict):
                    continue
                usage = pd.get("usage") if isinstance(pd.get("usage"), dict) else {}
                raw = pd.get("rawUsage") if isinstance(pd.get("rawUsage"), dict) else {}
                if not usage and not raw:
                    continue
                ident = value.get("id") or value.get("messageId")
                stamp = value.get("timestamp")
                if not isinstance(ident, str) or stamp is None:
                    continue
                try:
                    ts = int(datetime.datetime.fromisoformat(
                        str(stamp).replace("Z", "+00:00")).timestamp() * 1000) \
                        if isinstance(stamp, str) else int(stamp)
                except (ValueError, TypeError):
                    continue
                session = value.get("sessionId") or fallback

                def pick(*keys):
                    for container in (usage, raw):
                        for key in keys:
                            got = container.get(key)
                            if isinstance(got, int) and got >= 0:
                                return got
                    return None

                inp = pick("inputTokens", "prompt_tokens")
                out = pick("outputTokens", "completion_tokens")
                total = pick("totalTokens", "total_tokens")
                details = raw.get("prompt_tokens_details") if isinstance(raw.get("prompt_tokens_details"), dict) else {}
                cached = details.get("cached_tokens")
                if not isinstance(cached, int) or cached < 0:
                    cached = raw.get("prompt_cache_hit_tokens")
                if not isinstance(cached, int) or cached < 0:
                    cached = None
                    for item in usage.get("inputTokensDetails") or []:
                        got = item.get("cached_tokens") if isinstance(item, dict) else None
                        if isinstance(got, int) and got >= 0:
                            cached = got
                            break
                write = raw.get("prompt_cache_write_tokens")
                if not isinstance(write, int) or write < 0:
                    write = raw.get("cache_creation_input_tokens")
                if not isinstance(write, int) or write < 0:
                    write = None
                # `rawUsage.prompt_tokens` 存在 → 输入含缓存（`input_semantics = includes_cache`）。
                # 但**存的是原始 input**，缓存单列——扣减由消费方按语义做，这里不能提前扣，
                # 否则和生产代码的存值对不上。
                includes_cache = "prompt_tokens" in raw
                source[f"{session}:{ident}"] = {
                    "ts": ts,
                    "input": inp,
                    "includes_cache": includes_cache,
                    "output": out,
                    "read": cached,
                    "write": write,
                    "total": total,
                }
    return source


def workbuddy_jolu(conn):
    """WorkBuddy 只对源端采集行（`native:workbuddy:*`），不含网关转发行。

    网关行（`source='gateway-subscription'`）是 Jolu 自己转发时记的，源端日志里
    没有对应条目，混进来会变成一片「只在 Jolu」的噪音。
    """
    rows = jolu_rows(conn, "workbuddy")
    native = {row[0] for row in conn.execute(
        "SELECT dedup_key FROM requests WHERE platform='workbuddy' AND source LIKE 'native:workbuddy:%'"
    )}
    return {key: row for key, row in rows.items() if key in native}


# ---------------------------------------------------------------------- ZCode

def read_zcode():
    """~/.zcode/cli/db/db.sqlite 的 model_usage。

    **dedup_key 用的是 `model_usage.id`**（`native_sources.rs:779` 的 `dedup_key: id`），
    不是 `logical_request_id`——后者是一次逻辑请求，同一次可能有多个 attempt 行。
    因此这里也按 `id` 逐行对账，与 Jolu 一行一次调用保持一致。
    """
    path = os.path.join(os.path.expanduser("~"), ".zcode", "cli", "db", "db.sqlite")
    conn = open_ro(path)
    if conn is None:
        return None
    columns = {row[1] for row in conn.execute("PRAGMA table_info(model_usage)")}
    required = {"id", "started_at", "input_tokens", "output_tokens", "raw_usage_json"}
    if not required <= columns:
        conn.close()
        return None
    query = (
        "SELECT id, started_at, completed_at, input_tokens, output_tokens, "
        "cache_read_input_tokens, cache_creation_input_tokens, provider_total_tokens, "
        "computed_total_tokens, raw_usage_json FROM model_usage"
    )
    source = {}
    for row in conn.execute(query):
        ident, started_at, completed_at = row[0], row[1], row[2]
        if not isinstance(ident, str):
            continue
        ts = started_at if started_at is not None else completed_at
        if ts is None:
            continue
        raw = row[9]
        try:
            known = bool(raw) and isinstance(json.loads(raw), dict)
        except (TypeError, ValueError):
            known = False
        # 复刻 native_sources 的 number()：非负且在无 raw 证据时要求 > 0
        def pick(value):
            value = non_negative(value)
            return value if (known or (value is not None and value > 0)) else None

        inp, out = pick(row[3]), pick(row[4])
        read, write = pick(row[5]), pick(row[6])
        provider_total = non_negative(row[7])
        total = provider_total if provider_total is not None else (non_negative(row[8]) if known else None)
        source[ident] = {
            "ts": ts, "input": inp, "output": out, "read": read, "write": write, "total": total,
        }
    conn.close()
    return source


def zcode_jolu(conn):
    """ZCode 只对 `native:zcode:main`（源端日志采集），不含网关转发行。"""
    rows = jolu_rows(conn, "zcode")
    native = {row[0] for row in conn.execute(
        "SELECT dedup_key FROM requests WHERE platform='zcode' AND source='native:zcode:main'"
    )}
    return {key: row for key, row in rows.items() if key in native}


# ---------------------------------------------------------------------- main

READERS = {
    "claude": (read_claude, lambda conn: claude_jolu(conn), "~/.claude/projects/**/*.jsonl"),
    "opencode": (read_opencode, lambda conn: jolu_rows(conn, "opencode"), "opencode.db message"),
    "qoder": (read_qoder, lambda conn: jolu_rows(conn, "qoder"), "qoder local.db chat_message"),
    "workbuddy": (read_workbuddy, workbuddy_jolu, "~/.workbuddy/projects/**/*.jsonl"),
    "zcode": (read_zcode, zcode_jolu, ".zcode/cli/db/db.sqlite model_usage"),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--platform", choices=sorted(READERS), action="append", help="只对账指定平台，可重复")
    parser.add_argument("--json", action="store_true", help="输出 JSON 而不是表格")
    args = parser.parse_args()

    db_path = jolu_db_path()
    if not os.path.isfile(db_path):
        sys.exit(f"找不到 Jolu usage.db：{db_path}")
    conn = open_ro(db_path)

    platforms = args.platform or list(READERS)
    results = []
    for name in platforms:
        reader, jolu_reader, label = READERS[name]
        try:
            source = reader()
        except Exception as error:  # 源端结构变化不应中断其他平台
            results.append({"platform": name, "error": f"{type(error).__name__}: {error}"})
            continue
        if source is None:
            results.append({"platform": name, "error": "源端不存在或结构不受支持", "source": label})
            continue
        try:
            jolu = jolu_reader(conn)
        except Exception as error:
            results.append({"platform": name, "error": f"Jolu 侧查询失败：{error}"})
            continue
        report = compare(source, jolu, label)
        report["platform"] = name
        results.append(report)

    conn.close()

    if args.json:
        print(json.dumps(results, ensure_ascii=False, indent=2))
        return

    for report in results:
        print(f"=== {report['platform']} ===")
        if "error" in report:
            print(f"  跳过：{report['error']}")
            print()
            continue
        print(f"  源端：{report['source']}")
        print(f"  源端 ID {report['source_ids']}  Jolu {report['jolu_rows']}  "
              f"交集 {report['intersection']}  只在 Jolu {report['only_jolu']}  只在源端 {report['only_source']}")
        if report["field_diffs"]:
            print(f"  字段差异：{report['field_diffs']}")
            for sample in report["ts_samples"]:
                print(f"    ts 样例 {sample[0][:20]}… 源端 {ms_text(sample[1])} Jolu {ms_text(sample[2])} 差 {sample[3]}ms")
        else:
            print("  字段差异：零")
        print()


if __name__ == "__main__":
    main()
