"""把 context-capture.mjs 抓到的真实请求体做成可以进仓库的测试样本。

请求体里是各工具的真实系统提示词、工具说明，以及本机路径。这里只保留解析器要认的东西：
- JSON 结构与所有键名原样保留；
- 不含空白的短字符串（工具名、type、role、id、模型名）原样保留；
- 命令与路径类字段（command / cmd / file_path / path …）原样保留，解析器靠它们认出「读了哪个文件」；
- 其余文字按字符类等长填充：ASCII 字母换成 lorem 字母（保留大小写），其他文字换成 UTF-8 字节数相同的字符，
  数字、标点、空白与换行不动——token 估算与「空行结束技能列表」之类的规则因此不受影响；
- 文字里解析器依赖的标记原样保留：形如 <tag> 的标签、「Contents of … (…):」整行、「# AGENTS.md instructions for …」整行、
  技能列表开头那句、「# Environment」、各工具注入文本的固定开头等；
- 内容是 JSON 的字符串（工具调用参数）先解开递归处理再写回；
- 填充前先把本机路径换成虚构路径，最后检查结果里不再出现本机用户名。

用法：python -I scripts/acceptance/context-sample-sanitize.py
输入 output/acceptance/context-capture/raw/<工具>/，输出 backend/src/gateway/context/testdata/。
"""
import json
import os
import re
import subprocess
import sys
import unicodedata

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), "..", ".."))
RAW = os.path.join(ROOT, "output", "acceptance", "context-capture", "raw")
OUT = os.path.join(ROOT, "backend", "src", "gateway", "context", "testdata")

# 每个工具挑哪些请求体：第 1 次（只有提问）、读完一个文件后、读完两个文件后
PICK = {
    "claude": ("claude-code-2.1.326", "anthropic", [0, 1, 2]),
    "codex": ("codex-0.169.0", "responses", [0, 1, 2]),
    "opencode": ("opencode-1.19.5", "chat", [0, 1, 3]),  # 第 0 个是起标题的小请求
    "gemini": ("gemini-cli-0.68.0", "gemini", [0, 1, 2]),
}

KEEP_KEYS = {"command", "cmd", "file_path", "path", "filePath", "absolute_path", "target_file", "notebook_path",
             "file", "filename", "uri", "name", "type", "role", "id", "call_id", "tool_use_id", "model", "tool_call_id"}
# 文字里要原样保留的整行
KEEP_LINES = [
    re.compile(r"^Contents of \S.*\):[ \t]*$"),
    re.compile(r"^Instructions from: \S.*$"),
    re.compile(r"^# \S+\.md instructions for \S.*$"),
    re.compile(r"^# Environment\b.*$"),
    re.compile(r"^--- project-doc ---$"),
    re.compile(r"^--- (?:End of )?Context from: .* ---$"),
    re.compile(r"^## My request.*$"),
]
# 文字里要原样保留的短语（userWords 的 injected 开头、技能列表开头等）
KEEP_PHRASES = [
    "The following skills are available for use with the Skill tool:",
    "x-anthropic-billing-header",
    "Available agent types for the Agent tool:",
    "Available agent types and the tools they have access to:",
    "New agent types are now available for the Agent tool:",
    "The TodoWrite tool hasn't been used recently.",
    "The task tools haven't been used recently.",
    "The following deferred tools are",
    "# MCP Server Instructions",
    "The following MCP servers",
    "## Skills",
]
TAG = re.compile(r"</?[A-Za-z_][\w -]{0,40}>")
LOREM = "loremipsumdolorsitametconsecteturadipiscingelitseddoeiusmodtemporincididuntutlaboreetdoloremagnaaliqua"


def protected_spans(text):
    spans = [m.span() for m in TAG.finditer(text)]
    for rx in KEEP_LINES:
        spans += [m.span() for m in re.finditer(rx.pattern, text, re.M)]
    for p in KEEP_PHRASES:
        start = 0
        while (i := text.find(p, start)) >= 0:
            spans.append((i, i + len(p)))
            start = i + len(p)
    return sorted(spans)


def same_bytes(ch):
    n = len(ch.encode("utf-8"))
    return {2: "é", 3: "文", 4: "𝑥"}.get(n, ch)


def filler(text):
    keep = bytearray(len(text))
    for a, b in protected_spans(text):
        for i in range(a, b):
            keep[i] = 1
    out, k = [], 0
    for i, ch in enumerate(text):
        if keep[i] or not unicodedata.category(ch).startswith("L"):
            out.append(ch)
        elif ch.isascii():
            c = LOREM[k % len(LOREM)]
            k += 1
            out.append(c.upper() if ch.isupper() else c)
        else:
            out.append(same_bytes(ch))
    return "".join(out)


def is_prose(s):
    return len(s) > 40 or any(c.isspace() for c in s)


def clean(value, pairs, key=None):
    if isinstance(value, dict):
        return {strip_paths(k, pairs): clean(v, pairs, k) for k, v in value.items()}
    if isinstance(value, list):
        return [clean(v, pairs, key) for v in value]
    if not isinstance(value, str):
        return value
    t = value.strip()
    if t[:1] in "{[" and len(t) > 1:
        try:
            inner = json.loads(t)
        except ValueError:
            inner = None
        if isinstance(inner, (dict, list)):
            return json.dumps(clean(inner, pairs), ensure_ascii=False, separators=(",", ":"))
    # 先换路径再填充：填充会改掉路径里的字母，之后就认不出是本机路径了
    value = strip_paths(value, pairs)
    if key in KEEP_KEYS or not is_prose(value):
        return value
    return filler(value)


def local_paths():
    """(原文, 替换) 列表：长的在前，避免短的先换掉一半"""
    project = os.path.join(ROOT, "output", "acceptance", "context-capture", "project")
    homes = os.path.join(ROOT, "output", "acceptance", "context-capture", "home")
    profile = os.environ.get("USERPROFILE") or os.path.expanduser("~")
    pairs = []
    for tool in PICK:
        pairs.append((os.path.join(homes, tool), r"C:\Users\dev"))
    pairs += [(project, r"D:\work\demo-shop"), (ROOT, r"D:\work\jolu"), (profile, r"C:\Users\dev")]
    out = []
    for src, dst in pairs:
        for s in {src, src[0].lower() + src[1:], src[0].upper() + src[1:]}:
            out.append((s, dst))
            out.append((s.replace("\\", "/"), dst.replace("\\", "/")))
    return sorted(out, key=lambda p: -len(p[0]))


def strip_paths(text, pairs):
    for src, dst in pairs:
        text = text.replace(src, dst)
    return text


def git_identity():
    """本机 Git 身份（名字与邮箱 @ 前的部分），也不能出现在样本里"""
    out = []
    for key in ("user.name", "user.email"):
        try:
            v = subprocess.run(["git", "config", key], cwd=ROOT, capture_output=True, text=True, encoding="utf-8").stdout.strip()
        except OSError:
            v = ""
        out.append(v.split("@")[0].split("+")[-1] if key == "user.email" else v)
    return [v for v in out if len(v) >= 3]


def main():
    os.makedirs(OUT, exist_ok=True)
    pairs = local_paths()
    user = os.path.basename(os.environ.get("USERPROFILE") or os.path.expanduser("~"))
    identity = git_identity()
    written = []
    for tool, (label, proto, picks) in PICK.items():
        files = sorted(f for f in os.listdir(os.path.join(RAW, tool)) if f.endswith(".json"))
        for n, idx in enumerate(picks, 1):
            with open(os.path.join(RAW, tool, files[idx]), encoding="utf-8") as f:
                body = json.load(f)
            text = json.dumps(clean(body, pairs), ensure_ascii=False, separators=(",", ":"))
            leaks = [w for w in (user, *identity, "cc-usage") if w and w in text]
            if leaks:
                sys.exit(f"{tool} {files[idx]} 仍含本机信息：{leaks}")
            json.loads(text)
            name = f"{label}-{n}.{proto}.json"
            with open(os.path.join(OUT, name), "w", encoding="utf-8", newline="\n") as f:
                f.write(text)
            written.append((name, len(text.encode("utf-8"))))
    for name, size in written:
        print(f"{name}\t{size}")


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
