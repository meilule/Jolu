"""V10 上下文观察：抓真实请求样本用的本地模型上游（只监听回环，不调用外网、不读取用户密钥）。

与 model-upstream.py 的区别：它会让工具真的走一轮「调用 → 结果」，好让请求体里出现工具调用与工具结果。
- 请求里还没有工具结果：从请求自带的工具定义里挑读文件的工具读 README（没有就用执行命令的工具），
  按它的参数结构填好参数，回复一次工具调用；CLI 执行后会带着结果再来请求。
- 已有 1 个工具结果：再用执行命令的工具 cat src/cart.ts（没有就用读文件工具），让样本里同时有两种读法。
- 已有 2 个及以上：回复一句文字，结束这一轮。
支持 Anthropic Messages、OpenAI Chat Completions / Responses 与 Gemini 原生（含流式）。

每个 POST 的完整请求体写入 output/acceptance/context-capture/raw/<序号>-<协议>.json（output/ 不进 Git）；
请求体里是各工具的真实系统提示词，要进测试样本必须先经 context-sample-sanitize.py 做等长填充。

用法：python -I scripts/acceptance/context-capture-upstream.py [端口，默认 18767] [要读的文件名，默认 README.md]
"""
import json
import os
import sys
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18767
READ_FILE = sys.argv[2] if len(sys.argv) > 2 else "README.md"
RAW = os.path.join(os.path.dirname(__file__), "..", "..", "output", "acceptance", "context-capture", "raw")
TEXT = "已读完，README 说明了这个演示项目的用途。"
READ_TOOLS = ["Read", "read", "read_file", "ReadFile", "view", "View", "readFile", "open_file"]
SHELL_TOOLS = ["shell", "shell_command", "exec_command", "Bash", "bash", "run_shell_command", "local_shell"]
PATH_KEYS = ["file_path", "absolute_path", "filePath", "path", "target_file", "file", "filename"]
SEQ = [0]


def save(proto, body):
    os.makedirs(RAW, exist_ok=True)
    SEQ[0] += 1
    with open(os.path.join(RAW, f"{SEQ[0]:03d}-{proto}.json"), "w", encoding="utf-8") as f:
        json.dump(body, f, ensure_ascii=False, indent=1)


def tool_defs(proto, body):
    """(名字, 参数 schema) 列表"""
    out = []
    if proto == "gemini":
        for t in body.get("tools") or []:
            for d in t.get("functionDeclarations") or []:
                out.append((d.get("name"), d.get("parametersJsonSchema") or d.get("parameters") or {}))
        return out
    for t in body.get("tools") or []:
        if not isinstance(t, dict):
            continue
        if proto == "anthropic":
            out.append((t.get("name"), t.get("input_schema") or {}))
        elif t.get("type") == "function" and isinstance(t.get("function"), dict):
            out.append((t["function"].get("name"), t["function"].get("parameters") or {}))
        else:
            out.append((t.get("name"), t.get("parameters") or {}))
            for sub in t.get("tools") or []:
                if isinstance(sub, dict):
                    out.append((sub.get("name"), sub.get("parameters") or {}))
    return [(n, s) for n, s in out if n]


def find_read(defs, target):
    for name in READ_TOOLS:
        if name in defs:
            props = (defs[name] or {}).get("properties") or {}
            key = next((k for k in PATH_KEYS if k in props), next(iter(props), "file_path"))
            return name, {key: target}
    return None


def find_shell(defs, rel):
    for name in SHELL_TOOLS:
        if name in defs:
            props = (defs[name] or {}).get("properties") or {}
            key = "command" if "command" in props else ("cmd" if "cmd" in props else next(iter(props), "command"))
            kind = (props.get(key) or {}).get("type")
            args = {key: ["cat", rel] if kind == "array" else f"cat {rel}"}
            if "description" in props:
                args["description"] = f"Show {rel}"
            return name, args
    return None


def pick_call(proto, body, cwd, done):
    """第 1 轮用读文件工具读 README（没有就用 shell），第 2 轮用 shell 读 src/cart.ts（没有就用读文件工具），之后回文字"""
    defs = dict(tool_defs(proto, body))
    if done == 0:
        return find_read(defs, os.path.join(cwd, READ_FILE) if cwd else READ_FILE) or find_shell(defs, READ_FILE)
    if done == 1:
        rel = "src/cart.ts"
        return find_shell(defs, rel) or find_read(defs, os.path.join(cwd, "src", "cart.ts") if cwd else rel)
    return None


def results_in(proto, body):
    """请求里已有几个工具结果"""
    if proto == "anthropic":
        return sum(1 for m in body.get("messages") or [] if isinstance(m.get("content"), list)
                   for b in m["content"] if isinstance(b, dict) and b.get("type") == "tool_result")
    if proto == "chat":
        return sum(1 for m in body.get("messages") or [] if m.get("role") == "tool")
    if proto == "responses":
        items = body.get("input") if isinstance(body.get("input"), list) else []
        return sum(1 for i in items if isinstance(i, dict) and str(i.get("type", "")).endswith("_output"))
    if proto == "gemini":
        return sum(1 for c in body.get("contents") or [] for p in c.get("parts") or [] if "functionResponse" in p)
    return 0


def guess_cwd(_body):
    """读文件工具多半要绝对路径：由启动方用 JOLU_CAPTURE_CWD 告知演示项目目录，没给就用相对路径"""
    return os.environ.get("JOLU_CAPTURE_CWD", "")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def _json(self, status, data):
        raw = json.dumps(data, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def _sse(self, events, done=False):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        for name, data in events:
            chunk = (f"event: {name}\n" if name else "") + f"data: {json.dumps(data, ensure_ascii=False)}\n\n"
            self.wfile.write(chunk.encode())
            self.wfile.flush()
        if done:
            self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()
        self.close_connection = True

    def do_GET(self):
        if "models" in self.path:
            if self.path.startswith("/v1beta"):
                return self._json(200, {"models": [{"name": "models/test-model", "displayName": "test-model", "supportedGenerationMethods": ["generateContent"]}]})
            return self._json(200, {"object": "list", "data": [{"id": "test-model", "object": "model", "owned_by": "capture"}]})
        return self._json(404, {"error": "not found"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        try:
            body = json.loads(self.rfile.read(length) or b"{}")
        except ValueError:
            body = {}
        path = self.path.split("?")[0]
        if path.endswith("/count_tokens"):
            return self._json(200, {"input_tokens": 10})
        if path.endswith(":countTokens"):
            return self._json(200, {"totalTokens": 10})
        proto = ("anthropic" if path.endswith("/messages") else "chat" if path.endswith("/chat/completions")
                 else "responses" if path.endswith("/responses") else "gemini" if "generatecontent" in path.lower() else None)
        if proto is None:
            return self._json(404, {"error": f"unsupported path {path}"})
        save(proto, body)
        stream = bool(body.get("stream")) or "streamGenerateContent" in path
        call = pick_call(proto, body, guess_cwd(body), results_in(proto, body))
        model = body.get("model", "test-model")
        return getattr(self, f"reply_{proto}")(model, stream, call)

    def reply_anthropic(self, model, stream, call):
        mid = f"msg_{uuid.uuid4().hex[:12]}"
        usage = {"input_tokens": 12, "output_tokens": 3, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}
        if call:
            block = {"type": "tool_use", "id": f"toolu_{uuid.uuid4().hex[:12]}", "name": call[0], "input": call[1]}
            stop = "tool_use"
        else:
            block = {"type": "text", "text": TEXT}
            stop = "end_turn"
        if not stream:
            return self._json(200, {"id": mid, "type": "message", "role": "assistant", "model": model, "content": [block], "stop_reason": stop, "stop_sequence": None, "usage": usage})
        start = {**block, "input": {}} if call else {"type": "text", "text": ""}
        delta = {"type": "input_json_delta", "partial_json": json.dumps(call[1], ensure_ascii=False)} if call else {"type": "text_delta", "text": TEXT}
        self._sse([
            ("message_start", {"type": "message_start", "message": {"id": mid, "type": "message", "role": "assistant", "model": model, "content": [], "stop_reason": None, "stop_sequence": None, "usage": {**usage, "output_tokens": 1}}}),
            ("content_block_start", {"type": "content_block_start", "index": 0, "content_block": start}),
            ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": delta}),
            ("content_block_stop", {"type": "content_block_stop", "index": 0}),
            ("message_delta", {"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": None}, "usage": {"output_tokens": 3}}),
            ("message_stop", {"type": "message_stop"}),
        ])

    def reply_chat(self, model, stream, call):
        cid = f"chatcmpl-{uuid.uuid4().hex[:12]}"
        usage = {"prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15}
        if call:
            tc = {"index": 0, "id": f"call_{uuid.uuid4().hex[:12]}", "type": "function", "function": {"name": call[0], "arguments": json.dumps(call[1], ensure_ascii=False)}}
            message = {"role": "assistant", "content": None, "tool_calls": [{k: v for k, v in tc.items() if k != "index"}]}
            finish = "tool_calls"
        else:
            message = {"role": "assistant", "content": TEXT}
            finish = "stop"
        if not stream:
            return self._json(200, {"id": cid, "object": "chat.completion", "created": int(time.time()), "model": model, "choices": [{"index": 0, "message": message, "finish_reason": finish}], "usage": usage})
        base = {"id": cid, "object": "chat.completion.chunk", "created": int(time.time()), "model": model}
        delta = {"role": "assistant", "tool_calls": [tc]} if call else {"role": "assistant", "content": TEXT}
        self._sse([
            (None, {**base, "choices": [{"index": 0, "delta": delta, "finish_reason": None}]}),
            (None, {**base, "choices": [{"index": 0, "delta": {}, "finish_reason": finish}]}),
            (None, {**base, "choices": [], "usage": usage}),
        ], done=True)

    def reply_responses(self, model, stream, call):
        rid = f"resp_{uuid.uuid4().hex[:12]}"
        if call:
            item = {"id": f"fc_{uuid.uuid4().hex[:12]}", "type": "function_call", "status": "completed", "call_id": f"call_{uuid.uuid4().hex[:12]}", "name": call[0], "arguments": json.dumps(call[1], ensure_ascii=False)}
        else:
            item = {"id": f"msg_{uuid.uuid4().hex[:12]}", "type": "message", "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": TEXT, "annotations": []}]}
        usage = {"input_tokens": 12, "input_tokens_details": {"cached_tokens": 0}, "output_tokens": 3, "output_tokens_details": {"reasoning_tokens": 0}, "total_tokens": 15}
        done = {"id": rid, "object": "response", "created_at": int(time.time()), "status": "completed", "model": model, "output": [item], "usage": usage}
        if not stream:
            return self._json(200, done)
        events = [
            ("response.created", {"type": "response.created", "response": {**done, "status": "in_progress", "output": [], "usage": None}}),
            ("response.output_item.added", {"type": "response.output_item.added", "output_index": 0, "item": {**item, "status": "in_progress", **({"arguments": ""} if call else {"content": []})}}),
        ]
        if call:
            events += [
                ("response.function_call_arguments.delta", {"type": "response.function_call_arguments.delta", "item_id": item["id"], "output_index": 0, "delta": item["arguments"]}),
                ("response.function_call_arguments.done", {"type": "response.function_call_arguments.done", "item_id": item["id"], "output_index": 0, "arguments": item["arguments"]}),
            ]
        else:
            events += [
                ("response.output_text.delta", {"type": "response.output_text.delta", "item_id": item["id"], "output_index": 0, "content_index": 0, "delta": TEXT}),
                ("response.output_text.done", {"type": "response.output_text.done", "item_id": item["id"], "output_index": 0, "content_index": 0, "text": TEXT}),
            ]
        events += [
            ("response.output_item.done", {"type": "response.output_item.done", "output_index": 0, "item": item}),
            ("response.completed", {"type": "response.completed", "response": done}),
        ]
        self._sse(events)

    def reply_gemini(self, model, stream, call):
        part = {"functionCall": {"name": call[0], "args": call[1]}} if call else {"text": TEXT}
        data = {"candidates": [{"content": {"role": "model", "parts": [part]}, "finishReason": "STOP", "index": 0}],
                "usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 3, "totalTokenCount": 15}, "modelVersion": "test-model"}
        if not stream:
            return self._json(200, data)
        self._sse([(None, data)])


if __name__ == "__main__":
    print(f"context capture upstream on 127.0.0.1:{PORT}, raw bodies -> {os.path.abspath(RAW)}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
