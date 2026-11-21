//! 行级 YAML 编辑：把一个「顶层条目数组」的 YAML 文件按条目切开、就地改、再拼回去，
//! **保留用户写的一切**（注释、空行、缩进风格、条目顺序、未触碰的键）。
//!
//! ## 为什么不用 YAML 库
//!
//! 这类文件（dsh 的 `cordis.patch.yml` 是典型）是**用户手编**的：带注释、带自定义缩进、
//! 条目顺序有意义。用 `serde_yaml` 之类解析再序列化会丢注释、重排键、统一缩进——用户打开
//! 一看自己的配置被"格式化"了，就是数据损失。
//!
//! ## 适用范围
//!
//! 文件形态必须是：
//!
//! ```yaml
//! # 注释若干（head）
//! - id: first          ← 条目起头
//!   key: value
//!   nested:
//!     - ...
//! - id: second
//!   ...
//! ```
//!
//! 即「顶层是序列，每项以 `- ` 或 `  - ` 开头」。不是这种形态的文件（纯对象、纯标量）
//! 会被 [`parse`] 判为「无条目」，调用方据此跳过、不硬写。
//!
//! 出处：magpie `internal/agent/dsh.go` 的 `dshParse` / `dshJoin` / `dshIDLine` /
//! `dshBlockRange`（commit `e9dd8e8`）。2026-11-20 从 `wiring_dsh.rs` 抽出，
//! 为后续 qwen / droid / workbuddy 这类「数组插项」的接线复用。

/// 一个顶层条目：`id`（取不到时为空串）+ 原始行块。
///
/// 行块逐字保留（包括首行与尾随空行），这样 [`join`] 能原样拼回去。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Item {
    pub id: String,
    pub lines: Vec<String>,
}

/// 从条目首行取 id：`- id: xxx` 或缩进两格的 `  - id: xxx`。
///
/// 支持引号包裹与行尾注释；`- insert:` 这类没有 id 的条目返回 `None`
/// （调用方把它记成空 id——它仍是一条目，只是没法按名字找）。
pub(crate) fn id_of(line: &str) -> Option<String> {
    let rest = line.strip_prefix("- ").or_else(|| line.strip_prefix("  - "))?;
    let rest = rest.trim_start().strip_prefix("id:")?;
    let value = rest.trim().trim_start_matches(['\'', '"']);
    let value = value.split(['"', '#']).next()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// 切成（首个条目前的行, 条目数组）。
///
/// - 首个条目之前的一切（注释、`---` 文档标记、空行）进 `head`，原样保留；
/// - 条目起头 = `- ` / `-` / `  - `（不管带不带 id）；
/// - 空文件或纯注释返回空 `items`，调用方据此**不动这个文件**。
pub(crate) fn parse(text: &str) -> (Vec<String>, Vec<Item>) {
    let mut head: Vec<String> = Vec::new();
    let mut items: Vec<Item> = Vec::new();
    for line in text.lines() {
        let starts_item = line.starts_with("- ") || line == "-" || line.starts_with("  - ");
        if starts_item {
            items.push(Item { id: id_of(line).unwrap_or_default(), lines: vec![line.to_string()] });
        } else if items.is_empty() {
            head.push(line.to_string());
        } else if let Some(last) = items.last_mut() {
            last.lines.push(line.to_string());
        }
    }
    (head, items)
}

/// 拼回去。保证文件以换行结尾（与常见编辑器一致）。
pub(crate) fn join(head: &[String], items: &[Item]) -> String {
    let mut out = head.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    for item in items {
        out.push_str(&item.lines.join("\n"));
        out.push('\n');
    }
    out
}

/// 按 id 找条目下标（同一 id 多次出现时取第一个）。
pub(crate) fn find(items: &[Item], id: &str) -> Option<usize> {
    items.iter().position(|item| item.id == id)
}

/// 在条目内取某缩进层级的键块范围（起止行下标，含）。
///
/// `indent` 是键行的空格数（例如条目内顶层键是 2、`config` 下是 4）。
/// 子块结束于**下一个缩进 ≤ 本层的非空行**；空行与注释行不结束块（归入当前块尾巴）。
///
/// 注意：只按缩进判断，不解析引号内的 `:`——对手编配置够用，复杂 YAML 不支持。
pub(crate) fn block_range(lines: &[String], indent: usize, key: &str) -> Option<(usize, usize)> {
    let prefix = " ".repeat(indent) + key;
    let start = lines.iter().position(|line| line.starts_with(&prefix) && line.as_bytes().get(indent + key.len()) == Some(&b':'))?;
    let mut end = start;
    for (at, line) in lines.iter().enumerate().skip(start + 1) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let depth = line.len() - line.trim_start().len();
        if depth <= indent {
            break;
        }
        end = at;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        [
            "# 用户自己的注释",
            "# 第二行",
            "- id: first",
            "  name: 甲",
            "  config:",
            "    a: 1",
            "    nested:",
            "      deep: true",
            "- id: second",
            "  name: 乙",
            "- insert:",
            "    - id: inner",
            "      x: 1",
        ]
        .join("\n")
    }

    #[test]
    fn parse_keeps_head_and_unidentified_entries() {
        let (head, items) = parse(&sample());
        assert_eq!(head, vec!["# 用户自己的注释", "# 第二行"]);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, "first");
        assert_eq!(items[1].id, "second");
        // `- insert:` 没有 id，但仍是条目（magpie dshParse 同款）
        assert_eq!(items[2].id, "", "无 id 的条目要保留为空 id");
        assert!(items[2].lines.iter().any(|l| l.contains("inner")), "子条目不能丢");
    }

    #[test]
    fn join_round_trips_byte_for_byte() {
        let text = sample();
        let (head, items) = parse(&text);
        assert_eq!(join(&head, &items), text + "\n", "拼接必须逐字还原（只补末尾换行）");
    }

    #[test]
    fn empty_or_object_file_has_no_items() {
        assert!(parse("").1.is_empty());
        assert!(parse("# 只有注释\n").1.is_empty());
        // 顶层是对象、不是序列 → 不是我们能改的文件
        assert!(parse("key: value\nother: 1\n").1.is_empty());
    }

    #[test]
    fn id_of_handles_quotes_and_comments() {
        assert_eq!(id_of("- id: plain"), Some("plain".into()));
        assert_eq!(id_of("- id: \"quoted\""), Some("quoted".into()));
        assert_eq!(id_of("  - id: indented"), Some("indented".into()));
        assert_eq!(id_of("- id: with-comment # 说明"), Some("with-comment".into()));
        assert_eq!(id_of("- insert:"), None);
        assert_eq!(id_of("  name: not-an-id"), None);
    }

    #[test]
    fn block_range_respects_indent_and_skips_comments() {
        let (_, items) = parse(&sample());
        let lines = &items[0].lines;
        // 条目内缩进 4 的 config 块：包含两行键，且跳过内部的空行/注释
        let (start, end) = block_range(lines, 2, "config").unwrap();
        assert_eq!(lines[start], "  config:");
        assert!(lines[end].contains("deep"), "块要延续到最深的子键：{:?}", &lines[start..=end]);
        // 条目内不存在的键
        assert!(block_range(lines, 2, "missing").is_none());
        // 缩进 6 的 nested（在 config 内）
        let (s2, _) = block_range(lines, 4, "nested").unwrap();
        assert_eq!(lines[s2], "    nested:");
    }

    #[test]
    fn block_range_ignores_comment_and_blank_lines_inside() {
        let lines: Vec<String> = ["  config:", "    a: 1", "", "    # 注释", "    b: 2", "  name: x"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (start, end) = block_range(&lines, 2, "config").unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 4, "空行与注释不结束块，b: 2 仍在块内：{end}");
    }
}
