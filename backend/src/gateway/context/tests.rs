//! 上下文解析测试。magpie 部分移植自 `internal/gateway/prompt_test.go`；样本见 `testdata/README.md`。

use super::calls::files_of_command;
use super::estimate::tokens_of;
use super::parts::Builder;
use super::text::SKILLS_LIST;
use super::*;

const MAGPIE_CLAUDE: &str = include_str!("testdata/magpie-claude-code.anthropic.json");
const MAGPIE_CODEX: &str = include_str!("testdata/magpie-codex.responses.json");

/// (文件名, 协议, 内容)：本机 2026-11-17 抓的真实请求，正文等长填充。
const SAMPLES: &[(&str, Protocol, &str)] = &[
    ("claude-code-2.1.326-1", Protocol::Anthropic, include_str!("testdata/claude-code-2.1.326-1.anthropic.json")),
    ("claude-code-2.1.326-2", Protocol::Anthropic, include_str!("testdata/claude-code-2.1.326-2.anthropic.json")),
    ("claude-code-2.1.326-3", Protocol::Anthropic, include_str!("testdata/claude-code-2.1.326-3.anthropic.json")),
    ("magpie-claude-code", Protocol::Anthropic, MAGPIE_CLAUDE),
    ("codex-0.169.0-1", Protocol::Responses, include_str!("testdata/codex-0.169.0-1.responses.json")),
    ("codex-0.169.0-2", Protocol::Responses, include_str!("testdata/codex-0.169.0-2.responses.json")),
    ("codex-0.169.0-3", Protocol::Responses, include_str!("testdata/codex-0.169.0-3.responses.json")),
    ("magpie-codex", Protocol::Responses, MAGPIE_CODEX),
    ("opencode-1.19.5-1", Protocol::Chat, include_str!("testdata/opencode-1.19.5-1.chat.json")),
    ("opencode-1.19.5-2", Protocol::Chat, include_str!("testdata/opencode-1.19.5-2.chat.json")),
    ("opencode-1.19.5-3", Protocol::Chat, include_str!("testdata/opencode-1.19.5-3.chat.json")),
    ("gemini-cli-0.68.0-1", Protocol::Gemini, include_str!("testdata/gemini-cli-0.68.0-1.gemini.json")),
    ("gemini-cli-0.68.0-2", Protocol::Gemini, include_str!("testdata/gemini-cli-0.68.0-2.gemini.json")),
    ("gemini-cli-0.68.0-3", Protocol::Gemini, include_str!("testdata/gemini-cli-0.68.0-3.gemini.json")),
];

fn sample(name: &str) -> Prompt {
    let (_, protocol, body) = SAMPLES.iter().find(|s| s.0 == name).expect("样本");
    parse(*protocol, body.as_bytes()).unwrap_or_else(|| panic!("{name}: 解析不出"))
}

fn kinds(p: &Prompt) -> Vec<Kind> {
    p.parts.iter().map(|x| x.kind).collect()
}

fn item<'a>(p: &'a Prompt, kind: Kind, name: &str) -> Option<&'a PromptItem> {
    p.part(kind)?.items.iter().find(|it| it.name == name)
}

fn tokens(p: &Prompt, kind: Kind) -> i64 {
    p.part(kind).map_or(0, |x| x.tokens)
}

// ---- 所有样本 ----

/// 每个样本：解析得出、各类之和等于总量、unknown 占比低于 20%、每类不超过 24 项；打印八类 token 与占比。
#[test]
fn samples_add_up_and_unknown_stays_small() {
    for (name, protocol, body) in SAMPLES {
        let p = parse(*protocol, body.as_bytes()).unwrap_or_else(|| panic!("{name}: 解析不出"));
        let sum: i64 = p.parts.iter().map(|x| x.tokens).sum();
        assert_eq!(sum, p.tokens, "{name}: 各类之和");
        assert_eq!(p.estimated, p.tokens, "{name}: 未校准时总量就是估算");
        assert!(p.tokens > 500, "{name}: 总量 {}", p.tokens);
        let unknown = tokens(&p, Kind::Unknown);
        assert!(unknown * 5 < p.tokens, "{name}: unknown {unknown} / {}", p.tokens);
        for part in &p.parts {
            assert!(part.items.len() <= MAX_ITEMS, "{name}: {:?} {} 项", part.kind, part.items.len());
            let items: i64 = part.items.iter().map(|it| it.tokens).sum();
            assert_eq!(items, part.tokens, "{name}: {:?} 各项之和", part.kind);
        }
        let order: Vec<usize> = p.parts.iter().map(|x| Kind::ALL.iter().position(|k| *k == x.kind).unwrap()).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{name}: 顺序 {:?}", kinds(&p));
        let line: Vec<String> = Kind::ALL
            .iter()
            .map(|k| format!("{} {} ({:.1}%)", k.as_str(), tokens(&p, *k), tokens(&p, *k) as f64 * 100.0 / p.tokens as f64))
            .collect();
        println!("{name:<24} {:>6} tokens, {} turns | {}", p.tokens, p.turns, line.join(" | "));
    }
}

/// Claude Code 2.1.326：两个 CLAUDE.md 分别算用户与项目的，其余提醒是 reminders；技能列表单独一类；
/// Read 读的按路径算文件，Bash `cat` 读的也算文件（shell）；计费头与自身提示词是 system。
#[test]
fn claude_code_sample() {
    let p = sample("claude-code-2.1.326-3");
    assert_eq!(kinds(&p), vec![Kind::System, Kind::Tools, Kind::Skills, Kind::Memory, Kind::Files, Kind::Chat]);
    assert_eq!(item(&p, Kind::Memory, r"C:\Users\dev\.claude\CLAUDE.md").map(|it| it.tag.as_str()), Some("user"));
    assert_eq!(item(&p, Kind::Memory, r"D:\work\demo-shop\CLAUDE.md").map(|it| it.tag.as_str()), Some("project"));
    assert!(item(&p, Kind::Memory, "reminders").is_some());
    assert!(item(&p, Kind::Skills, "skills").is_some_and(|it| it.tokens > 500));
    assert_eq!(item(&p, Kind::Files, r"D:\work\demo-shop\README.md").map(|it| it.tag.as_str()), Some("read"));
    assert_eq!(item(&p, Kind::Files, "src/cart.ts").map(|it| it.tag.as_str()), Some("shell"));
    assert!(item(&p, Kind::System, "billing").is_some());
    assert!(item(&p, Kind::System, "prompt").is_some_and(|it| it.tokens > 3000));
    assert!(item(&p, Kind::Tools, "Read").is_some() && item(&p, Kind::Tools, "Bash").is_some());
    assert_eq!((p.turns, p.counted, p.held), (1, false, false));
    // 第 1 个请求只有提问：没有文件、没有结果
    let first = sample("claude-code-2.1.326-1");
    assert!(first.part(Kind::Files).is_none() && first.part(Kind::Results).is_none());
}

/// Codex 0.169：AGENTS.md 按 `--- project-doc ---` 拆成用户的与项目的；`<skills_instructions>` 是技能；
/// exec_command 的 `cat` 读的是文件；环境与权限说明是 system。
#[test]
fn codex_sample() {
    let p = sample("codex-0.169.0-3");
    assert_eq!(item(&p, Kind::Memory, r"$CODEX_HOME\AGENTS.md").map(|it| it.tag.as_str()), Some("user"));
    assert_eq!(item(&p, Kind::Memory, r"D:\work\demo-shop\AGENTS.md").map(|it| it.tag.as_str()), Some("project"));
    assert!(item(&p, Kind::Skills, "skills").is_some());
    assert!(item(&p, Kind::Memory, "skills").is_none());
    for f in ["README.md", "src/cart.ts"] {
        assert_eq!(item(&p, Kind::Files, f).map(|it| it.tag.as_str()), Some("shell"), "{f}");
    }
    for s in ["prompt", "permissions instructions", "environment_context"] {
        assert!(item(&p, Kind::System, s).is_some(), "{s}: {:?}", p.part(Kind::System));
    }
    assert!(item(&p, Kind::Tools, "exec_command").is_some());
    assert_eq!(p.turns, 1);
}

/// OpenCode 1.19.5（Chat）：`Instructions from:` 的文件是 memory（`.config` 下的是用户的），
/// `<available_skills>` 是技能；read 读的与 bash `cat` 读的都是文件。
#[test]
fn opencode_sample() {
    let p = sample("opencode-1.19.5-3");
    assert_eq!(item(&p, Kind::Memory, r"C:\Users\dev\.config\opencode\AGENTS.md").map(|it| it.tag.as_str()), Some("user"));
    assert_eq!(item(&p, Kind::Memory, r"D:\work\jolu\AGENTS.md").map(|it| it.tag.as_str()), Some("project"));
    assert!(item(&p, Kind::Skills, "skills").is_some());
    assert_eq!(item(&p, Kind::Files, r"D:\work\demo-shop\README.md").map(|it| it.tag.as_str()), Some("read"));
    assert_eq!(item(&p, Kind::Files, "src/cart.ts").map(|it| it.tag.as_str()), Some("shell"));
    assert!(item(&p, Kind::Tools, "bash").is_some());
    assert_eq!(p.turns, 1);
    // 起标题的小请求：只有系统提示词和对话
    let title = sample("opencode-1.19.5-1");
    assert_eq!(kinds(&title), vec![Kind::System, Kind::Chat]);
}

/// Gemini CLI 0.68：`--- Context from: … ---` 的 GEMINI.md 是 memory，`<available_skills>` 是技能，
/// `<session_context>` 是 harness 上下文不算用户说话；read_file 与 run_shell_command 的 `cat` 读的都是文件。
#[test]
fn gemini_sample() {
    let p = sample("gemini-cli-0.68.0-3");
    assert_eq!(item(&p, Kind::Memory, "C:/Users/dev/.gemini/GEMINI.md").map(|it| it.tag.as_str()), Some("user"));
    assert_eq!(item(&p, Kind::Memory, "D:/work/demo-shop/GEMINI.md").map(|it| it.tag.as_str()), Some("project"));
    assert!(item(&p, Kind::Skills, "skills").is_some());
    assert!(item(&p, Kind::System, "session_context").is_some(), "{:?}", p.part(Kind::System));
    assert_eq!(item(&p, Kind::Files, r"D:\work\demo-shop\README.md").map(|it| it.tag.as_str()), Some("read"));
    assert_eq!(item(&p, Kind::Files, "src/cart.ts").map(|it| it.tag.as_str()), Some("shell"));
    assert_eq!(p.turns, 1);
}

// ---- 移植自 magpie ----

/// magpie 的 Claude Code 2.1 样本：用户与项目的 CLAUDE.md、一个 MCP 服务器的工具记一项、Read 读的文件、ls 的结果、harness 的 # Environment。
#[test]
fn magpie_claude_code() {
    let p = sample("magpie-claude-code");
    assert_eq!(kinds(&p), vec![Kind::System, Kind::Tools, Kind::Memory, Kind::Files, Kind::Results, Kind::Chat]);
    assert_eq!(item(&p, Kind::Memory, "/home/dev/.claude/CLAUDE.md").map(|it| it.tag.as_str()), Some("user"));
    assert_eq!(item(&p, Kind::Memory, "/home/dev/proj/CLAUDE.md").map(|it| it.tag.as_str()), Some("project"));
    assert!(item(&p, Kind::Memory, "reminders").is_some());
    let linear = item(&p, Kind::Tools, "linear").expect("linear");
    assert_eq!((linear.tag.as_str(), linear.n), ("mcp", 3));
    assert!(item(&p, Kind::Tools, "Bash").is_some());
    for f in ["/home/dev/proj/main.go", "/home/dev/proj/reconcile.go"] {
        assert_eq!(item(&p, Kind::Files, f).map(|it| it.tag.as_str()), Some("read"), "{f}");
    }
    assert_eq!(item(&p, Kind::Results, "Bash").map(|it| it.tag.as_str()), Some("result"));
    assert!(item(&p, Kind::System, "environment").is_some());
    assert_eq!((p.turns, p.counted, p.held), (1, false, false));
    assert!(p.tokens >= 10000);
}

/// magpie 的 Codex 样本：additional_tools 里自己的函数逐个记、其他命名空间整体记；exec 的 JavaScript 用 cat 读文件。
#[test]
fn magpie_codex() {
    let p = sample("magpie-codex");
    assert_eq!(item(&p, Kind::Memory, "/home/dev/proj/AGENTS.md").map(|it| it.tag.as_str()), Some("project"));
    assert!(item(&p, Kind::Skills, "skills").is_some());
    assert!(item(&p, Kind::Memory, "skills").is_none());
    let collab = item(&p, Kind::Tools, "collaboration").expect("collaboration");
    assert!(collab.tag == "namespace" && collab.n >= 2, "{collab:?}");
    assert!(item(&p, Kind::Tools, "exec").is_some());
    for f in ["main.go", "reconcile.go"] {
        assert_eq!(item(&p, Kind::Files, f).map(|it| it.tag.as_str()), Some("shell"), "{f}");
    }
    assert!(item(&p, Kind::System, "environment_context").is_some());
    assert!(item(&p, Kind::System, "prompt").is_some());
    assert_eq!(p.turns, 1);
}

#[test]
fn split_skills_ends_at_blank_line() {
    let mut b = Builder::new();
    let rest = b.split_skills(&format!("Notes.\n\n{SKILLS_LIST}\n\n- a: one\n  more of a\n- b: two\n\nToday's date is 2026-10-24."));
    assert_eq!(rest, "Notes.\n\nToday's date is 2026-10-24.");
    let p = b.finish();
    assert_eq!(item(&p, Kind::Skills, "skills").map(|it| it.tokens), Some(tokens_of(&format!("{SKILLS_LIST}\n\n- a: one\n  more of a\n- b: two"))));
    let mut b = Builder::new();
    assert_eq!(b.split_skills("no list here"), "no list here");
}

#[test]
fn codex_mcp_namespace() {
    let body = format!(
        r#"{{"model":"gpt-5.5","input":[{{"type":"message","role":"user","content":[{{"type":"input_text","text":"hi"}}]}}],"tools":[
        {{"type":"namespace","name":"mcp__linear__","tools":[{{"type":"function","name":"list_issues","description":"{}"}},{{"type":"function","name":"get_issue","description":"one issue"}}]}},
        {{"type":"namespace","name":"clock","tools":[{{"type":"function","name":"sleep","description":"sleeps"}}]}}]}}"#,
        "issues ".repeat(200)
    );
    let p = parse(Protocol::Responses, body.as_bytes()).unwrap();
    let linear = item(&p, Kind::Tools, "linear").unwrap();
    assert_eq!((linear.tag.as_str(), linear.n), ("mcp", 2));
    assert_eq!(item(&p, Kind::Tools, "clock").map(|it| it.tag.as_str()), Some("namespace"));
}

#[test]
fn chat_completions() {
    let body = r#"{"model":"m","messages":[
     {"role":"system","content":"You are OpenCode.\nInstructions from: /p/AGENTS.md\nbe brief, always"},
     {"role":"user","content":"look at a.go"},
     {"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"read","arguments":"{\"filePath\":\"/p/a.go\"}"}},
       {"id":"c2","type":"function","function":{"name":"bash","arguments":"{\"command\":\"go test ./...\"}"}}]},
     {"role":"tool","tool_call_id":"c1","content":"package a\nfunc A() {}"},
     {"role":"tool","tool_call_id":"c2","content":"ok  a 0.1s"},
     {"role":"user","content":[{"type":"text","text":"and this"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}],
     "tools":[{"type":"function","function":{"name":"read","parameters":{}}},{"type":"function","function":{"name":"mcp__gh__issue","parameters":{}}}]}"#;
    let p = parse(Protocol::Chat, body.as_bytes()).unwrap();
    assert_eq!(p.turns, 2);
    assert_eq!(item(&p, Kind::Memory, "/p/AGENTS.md").map(|it| it.tag.as_str()), Some("project"));
    assert!(item(&p, Kind::Files, "/p/a.go").is_some());
    assert_eq!(item(&p, Kind::Files, "image").map(|it| it.tokens), Some(IMAGE_TOKENS));
    assert!(item(&p, Kind::Results, "bash").is_some());
    assert_eq!(item(&p, Kind::Tools, "gh").map(|it| it.tag.as_str()), Some("mcp"));
}

#[test]
fn gemini_matches_response_by_name() {
    let body = r#"{"systemInstruction":{"parts":[{"text":"You are Gemini CLI."}]},
     "contents":[{"role":"user","parts":[{"text":"read go.mod"}]},
      {"role":"model","parts":[{"functionCall":{"name":"read_file","args":{"path":"go.mod"}}}]},
      {"role":"user","parts":[{"functionResponse":{"name":"read_file","response":{"output":"module x"}}}]}],
     "tools":[{"functionDeclarations":[{"name":"read_file","parameters":{}}]}]}"#;
    let p = parse(Protocol::Gemini, body.as_bytes()).unwrap();
    assert_eq!(p.turns, 1);
    assert!(item(&p, Kind::Files, "go.mod").is_some());
}

/// 模型的思考段计入对话（与 magpie 一致）。
#[test]
fn gemini_thought_counts_as_chat() {
    let body = r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]},
      {"role":"model","parts":[{"text":"let me think about this for a while","thought":true},{"text":"hello"}]}]}"#;
    let p = parse(Protocol::Gemini, body.as_bytes()).unwrap();
    assert_eq!(tokens(&p, Kind::Chat), tokens_of("hi") + tokens_of("let me think about this for a while") + tokens_of("hello"));
}

#[test]
fn responses_held_and_not_a_body() {
    let p = parse(Protocol::Responses, br#"{"previous_response_id":"resp_1","input":[{"role":"user","content":"go on"}]}"#).unwrap();
    assert!(p.held);
    assert_eq!(p.turns, 1);
    assert!(parse(Protocol::Anthropic, b"not json").is_none());
    assert!(parse(Protocol::Anthropic, b"[1,2]").is_none());
    assert!(parse(Protocol::Anthropic, br#"{"model":"x"}"#).is_none(), "没有 messages 不是 Messages 请求");
    assert!(parse(Protocol::Gemini, br#"{"messages":[]}"#).is_none());
}

#[test]
fn calibrated_adds_up_and_keeps_estimate() {
    let p = sample("magpie-claude-code");
    let before = p.clone();
    let c = p.calibrated(31337);
    let sum: i64 = c.parts.iter().map(|x| x.tokens).sum();
    assert!(c.counted && c.tokens == 31337 && sum == 31337, "{sum}");
    assert_eq!(c.estimated, before.tokens, "估算量保留，用来提示偏差");
    assert_eq!(p, before, "校准不改原来的");
    let again = c.calibrated(5);
    assert_eq!(again.tokens, 31337, "已校准的不再校准");
    assert_eq!(p.calibrated(0), p, "没有实测量时原样");
}

#[test]
fn files_of_command_cases() {
    let cases: &[(&str, &[&str])] = &[
        ("cat main.go reconcile.go", &["main.go", "reconcile.go"]),
        ("sed -n 1,80p internal/x.go", &["internal/x.go"]),
        ("head -n 40 README.md", &["README.md"]),
        (r#"bash -lc "nl -ba a.go""#, &["a.go"]),
        ("cd /p && cat go.mod", &["go.mod"]),
        ("cat a.go | grep x", &[]),
        ("go test ./...", &[]),
        ("ls", &[]),
    ];
    for (cmd, want) in cases {
        assert_eq!(files_of_command(cmd), want.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{cmd}");
    }
}

// ---- Jolu 新增 ----

/// 认不出的块、条目、角色记到 unknown，带上类型名，不推断。
#[test]
fn unrecognized_goes_to_unknown() {
    let a = parse(
        Protocol::Anthropic,
        br#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"hologram","data":"xxxxxxxxxxxxxxxx"}]},{"role":"narrator","content":"once upon a time"}]}"#,
    )
    .unwrap();
    assert_eq!(item(&a, Kind::Unknown, "hologram").map(|it| it.tag.as_str()), Some("type"));
    assert_eq!(item(&a, Kind::Unknown, "narrator").map(|it| it.tag.as_str()), Some("role"));
    let r = parse(Protocol::Responses, br#"{"input":[{"type":"teleport","x":"yyyyyyyy"},{"role":"oracle","content":"zzzz"}]}"#).unwrap();
    assert!(item(&r, Kind::Unknown, "teleport").is_some() && item(&r, Kind::Unknown, "oracle").is_some());
    let c = parse(Protocol::Chat, br#"{"messages":[{"role":"critic","content":"meh meh meh"}]}"#).unwrap();
    assert!(item(&c, Kind::Unknown, "critic").is_some());
    let g = parse(Protocol::Gemini, br#"{"contents":[{"role":"user","parts":[{"videoMetadata":{"start":"1s"}}]}]}"#).unwrap();
    assert!(item(&g, Kind::Unknown, "videoMetadata").is_some());
}

/// 超过 24 项：非对话类把最小的汇总成 more，对话类把最早的汇总成 earlier。
#[test]
fn overflow_is_summed() {
    let mut b = Builder::new();
    for i in 0..30 {
        b.add(Kind::Tools, &format!("t{i:02}"), "", 100 + i);
    }
    for _ in 0..30 {
        b.new_turn();
        b.chat(10);
    }
    let p = b.finish();
    let tools = p.part(Kind::Tools).unwrap();
    assert_eq!(tools.items.len(), MAX_ITEMS);
    let more = tools.items.last().unwrap();
    assert_eq!((more.name.as_str(), more.tag.as_str(), more.n), ("more", "more", 7));
    assert_eq!(tools.items[0].name, "t29", "最大的在前");
    let chat = p.part(Kind::Chat).unwrap();
    assert_eq!(chat.items.len(), MAX_ITEMS);
    assert_eq!((chat.items[0].name.as_str(), chat.items[0].tag.as_str(), chat.items[0].n), ("earlier", "turns", 7));
    assert_eq!(chat.items.last().unwrap().name, "30", "最近一轮在最后");
}

/// 一段带 `--- project-doc ---` 的 Codex AGENTS.md：前半是用户的，后半是项目的；没有分隔时整段算项目的（与 magpie 一致）。
#[test]
fn codex_agents_md_split() {
    let mut b = Builder::new();
    b.user_text("# AGENTS.md instructions for /p\n\n<INSTRUCTIONS>\nmine\n\n--- project-doc ---\n\nproject rules here\n</INSTRUCTIONS>");
    b.user_text("# AGENTS.md instructions for /q\n\n<INSTRUCTIONS>\nonly one\n</INSTRUCTIONS>");
    let p = b.finish();
    assert_eq!(item(&p, Kind::Memory, "$CODEX_HOME/AGENTS.md").map(|it| it.tag.as_str()), Some("user"));
    assert_eq!(item(&p, Kind::Memory, "/p/AGENTS.md").map(|it| it.tag.as_str()), Some("project"));
    assert_eq!(item(&p, Kind::Memory, "/q/AGENTS.md").map(|it| it.tag.as_str()), Some("project"));
}

/// 描述里带 auto-memory 的是 memory；Windows 路径的用户目录也认。
#[test]
fn memory_tags() {
    use super::text::memory_tag;
    assert_eq!(memory_tag("x", "user's auto-memory, persists across conversations"), "memory");
    assert_eq!(memory_tag("x", "user's private global instructions for all projects"), "user");
    assert_eq!(memory_tag("x", "project instructions, checked into the codebase"), "project");
    assert_eq!(memory_tag(r"C:\Users\dev\.claude\CLAUDE.md", ""), "user");
    assert_eq!(memory_tag("/home/dev/.gemini/GEMINI.md", ""), "user");
    assert_eq!(memory_tag(r"D:\work\AGENTS.md", ""), "project");
}

/// 各种奇怪的文字（多字节字符紧贴标记、不闭合的标签、只有标记）走一遍文字规则，不 panic。
#[test]
fn odd_texts_never_panic() {
    let odd = [
        "",
        "<",
        "<system-reminder",
        "<system-reminder>",
        "<system-reminder x=\"中\">文</system-reminder>",
        "Contents of 中文 (描述):\n内容</system-reminder>尾",
        "Contents of x ((((:",
        "Contents of x (a(b)c):",
        "Contents of  (x):",
        "# A.md instructions for 中\n--- project-doc ---\n",
        "# 中.md instructions for x",
        "--- Context from: 中 ---\n文\n--- End of Context from: 中 ---",
        "--- Context from:  ---",
        "<available_skills>中",
        "<available_skills></available_skills>",
        SKILLS_LIST,
        "## My request:",
        "## My request for Codex:中",
        "<中文>",
        "<a中>",
        "<environment_context>",
        "<session_context>--- Context from: a ---\n",
        "tools.中(",
        "cmd:\"\\",
    ];
    for t in odd {
        let mut b = Builder::new();
        b.user_text(t);
        b.system_text(t, "prompt");
        let _ = super::text::user_says(t);
        let _ = super::calls::call_of_text("exec", t);
        let _ = files_of_command(t);
        b.finish();
    }
}

/// 把每个样本里的节点逐个换成别的类型（null、数字、字符串、空数组、空对象、带怪 type 的数组）再解析：
/// 不 panic（测试构建里 panic 会让测试失败；发布版 panic = abort，所以必须靠实现本身不 panic）。
#[test]
fn mutated_bodies_never_panic() {
    use serde_json::{json, Value};
    fn count(v: &Value) -> usize {
        1 + match v {
            Value::Array(a) => a.iter().map(count).sum(),
            Value::Object(m) => m.values().map(count).sum(),
            _ => 0,
        }
    }
    fn replace(v: &mut Value, target: &mut usize, with: &Value) -> bool {
        if *target == 0 {
            *v = with.clone();
            return true;
        }
        *target -= 1;
        match v {
            Value::Array(a) => a.iter_mut().any(|x| replace(x, target, with)),
            Value::Object(m) => m.values_mut().any(|x| replace(x, target, with)),
            _ => false,
        }
    }
    let swaps = [json!(null), json!(7), json!("x"), json!([]), json!({}), json!([{"type": "?", "text": 1}]), json!({"type": 3})];
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    for (name, protocol, body) in SAMPLES {
        let base: Value = serde_json::from_str(body).unwrap();
        let total = count(&base);
        for round in 0..120 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let mut v = base.clone();
            let mut at = (seed >> 33) as usize % total;
            replace(&mut v, &mut at, &swaps[round % swaps.len()]);
            let bytes = serde_json::to_vec(&v).unwrap();
            let got = std::panic::catch_unwind(|| parse(*protocol, &bytes));
            assert!(got.is_ok(), "{name}: 第 {round} 次变异 panic");
        }
        // 截断：不是合法 JSON，返回 None
        assert!(parse(*protocol, &body.as_bytes()[..body.len() / 2]).is_none());
    }
}

/// panic 被兜住、返回 None（调试与测试构建）。
#[test]
fn guarded_turns_panic_into_none() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let got: Option<i32> = super::guarded(|| panic!("boom"));
    std::panic::set_hook(prev);
    assert!(got.is_none());
}

/// 约 10 万 token 的请求体（把 Claude Code 样本的对话复制到 40 万字节以上）：发布构建单次解析低于 5ms。
/// 调试构建只打印耗时（未优化的 serde_json 慢一个数量级），用 `cargo test --release` 跑这条看门槛。
#[test]
fn parse_large_body_fast() {
    use serde_json::Value;
    let mut v: Value = serde_json::from_str(SAMPLES[2].2).unwrap();
    let messages = v["messages"].as_array().unwrap().clone();
    let mut grown = Vec::new();
    while serde_json::to_vec(&grown).unwrap().len() < 480_000 {
        grown.extend(messages.iter().cloned());
    }
    v["messages"] = Value::Array(grown);
    let body = serde_json::to_vec(&v).unwrap();
    let p = parse(Protocol::Anthropic, &body).unwrap();
    assert!(p.tokens > 100_000, "{}", p.tokens);
    let runs = 20;
    let start = std::time::Instant::now();
    for _ in 0..runs {
        std::hint::black_box(parse(Protocol::Anthropic, std::hint::black_box(&body)));
    }
    let each = start.elapsed() / runs;
    println!("{} 字节、估算 {} tokens：单次 {:?}", body.len(), p.tokens, each);
    if !cfg!(debug_assertions) {
        assert!(each < std::time::Duration::from_millis(5), "{each:?}");
    }
}
