#!/usr/bin/env python3
"""从 docs/ui/jolu-new.pen 生成 frontend/src/theme/tokens.css。

## 为什么用脚本生成而不是手抄

`jolu-new.pen` 是设计稿的唯一事实来源，`theme/tokens.css` 只是它在代码里的投影。
手抄会有两个必然结果：数值抄错一位看不出，或者设计稿更新后代码忘了跟。
脚本跑一遍就能对齐，也让「tokens.css 少了 pen 里某个 jn-* 变量」这种问题
不可能发生——少一个就生成不出来。

## 为什么读 JSON 而不走 Pencil MCP

`docs/ui/Jolu.pen`（旧稿）是加密的，只能通过 MCP 读；`docs/ui/jolu-new.pen`
是本仓库生成的明文 JSON（见 `build-jolu-new-pen.py` 的说明，schema version 2.25），
直接解析更可靠、结果可重复，也不需要 MCP 在线。

## 用法（仓库根目录）

    python -I scripts/dev/extract-pen-tokens.py           # 生成
    python -I scripts/dev/extract-pen-tokens.py --check   # 只校验，不写文件

`--check` 用于提交前检查：生成结果与磁盘上的文件不一致时以非零码退出。

## 变量分类

| pen 里的 type / 命名 | 去向 | Tailwind 暴露 |
|---|---|---|
| color（`jn-*`） | `:root` / `.dark` 两组 `--jn-*` | `--color-jn-*` → `bg-jn-*` / `text-jn-*` |
| number 且名字以 `jn-r-` 开头 | 同上 | `--radius-jn-*` → `rounded-jn-*` |
| number 且名字以 `jn-gap-` 开头 | 同上 | `--spacing-jn-*` → `gap-jn-*` / `p-jn-*` |
| string 且名字以 `jn-font` / `jn-mono` 开头 | 同上 | `--font-jn` / `--font-jn-mono` → `font-jn` / `font-jn-mono` |

另外从画板的 `shadow` effect 里提取**表面投影**（卡片 / 列表行 / 输入框共用的那一档），
输出源变量 `--jn-shadow-surface` 与 Tailwind 映射 `--shadow-jn-surface`（→ `shadow-jn-surface`）。设计稿里有 24 种阴影定义，其中
223 次引用都是同一个 `0 1px 3px $jn-shadow`，其余是灵动岛的装饰性光晕（内高光、
额度告警光圈），属于具体组件的局部效果，不在这里做成全局令牌。

其余类型或命名会被报错而不是静默跳过——设计稿加了新类别的变量时，
这里要显式决定它怎么落到代码里，不能默认丢掉。
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PEN = ROOT / "docs" / "ui" / "jolu-new.pen"
OUT = ROOT / "frontend" / "src" / "theme" / "tokens.css"

# 生成文件的开头。放在脚本里而不是手写在 css 里，保证重新生成后说明不会丢。
HEADER = """/* 由 scripts/dev/extract-pen-tokens.py 从 docs/ui/jolu-new.pen 生成，请勿手改。
 *
 * 改颜色的正确做法：改设计稿 → 重跑脚本。手改这里会在下次生成时被覆盖，
 * 而且设计稿与代码会立刻不一致。
 *
 * 变量命名：设计稿里是 jn-*（Jolu New）。Tailwind 侧通过 @theme inline 暴露成
 * --color-jn-* / --radius-jn-* / --spacing-jn-* / --font-jn*，页面只用类名，
 * 不直接写 var(--jn-*)，更不写十六进制色值。
 *
 * 浅 / 深两套：:root 是浅色，.dark 覆盖同名变量。深色值逐项取自设计稿，
 * 不用滤镜或亮度推导——设计稿给的深色不是浅色的机械变暗。
 */
"""


def light_dark(spec: dict) -> tuple[object, object]:
    """拆出一组变量的浅色 / 深色取值。

    pen 的 color 变量是 `value: [{value, theme:{mode}}...]`；number / string
    是单值（`value: 14`），没有主题之分，深浅共用。
    """
    value = spec.get("value")
    if not isinstance(value, list):
        return value, value
    light = dark = None
    for entry in value:
        mode = (entry.get("theme") or {}).get("mode")
        if mode == "light":
            light = entry.get("value")
        elif mode == "dark":
            dark = entry.get("value")
    # 只有单主题的变量：缺失的一侧沿用另一侧，不编一个值
    return light if light is not None else dark, dark if dark is not None else light


def classify(name: str, kind: str) -> tuple[str, str]:
    """把一个 pen 变量归到 (CSS 变量名, Tailwind 命名空间)。

    Tailwind 命名空间决定能生成哪些类名：
    - color → `--color-*`，得到 bg-*/text-*/border-*
    - radius → `--radius-*`，得到 rounded-*
    - spacing → `--spacing-*`，得到 p-*/gap-*/m-*
    - font → `--font-*`，得到 font-*

    返回的 CSS 变量名保持设计稿原名（`--jn-primary`），这样设计稿与代码
    可以逐字对照；Tailwind 那一侧只是多一层 `--color-` 之类的前缀。
    """
    if kind == "color":
        return f"--{name}", f"--color-{name}"
    if kind == "number":
        if name.startswith("jn-r-"):
            # 设计稿 jn-r-card → Tailwind --radius-jn-card → 类名 rounded-jn-card。
            # 去掉 jn-r- 里的 r：Tailwind 的 --radius-* 前缀已经表达了「圆角」，
            # 保留会变成 rounded-jn-r-card 这种同义重复。
            return f"--{name}", f"--radius-jn-{name[len('jn-r-'):]}"
        if name.startswith("jn-gap-"):
            # 设计稿 jn-gap-card → --spacing-jn-card → gap-jn-card / p-jn-card
            return f"--{name}", f"--spacing-jn-{name[len('jn-gap-'):]}"
    if kind == "string":
        if name == "jn-font":
            return f"--{name}", "--font-jn"
        if name == "jn-mono":
            return f"--{name}", "--font-jn-mono"
    raise SystemExit(
        f"未识别的变量 {name!r}（type={kind}）：请在 extract-pen-tokens.py 的 "
        f"classify() 里显式决定它怎么落到 CSS，不要静默跳过"
    )


def css_value(kind: str, raw: object) -> str:
    """把 pen 的值转成 CSS 字面量。

    number 一律补 px：设计稿里的 14 / 16 / 24 都是像素，没有无单位语义。
    字符串只允许字体名，补一层引号以防名字里有空格（如 JetBrains Mono）。
    """
    if kind == "number":
        if not isinstance(raw, (int, float)):
            raise SystemExit(f"number 类型的值不是数字：{raw!r}")
        return f"{raw}px"
    if kind == "string":
        text = str(raw).strip()
        if not text:
            raise SystemExit("字符串变量的值为空")
        # 字体名可能带空格，统一加引号；单个词也加，避免与关键字冲突
        return f'"{text}", sans-serif' if "Mono" not in text else f'"{text}", monospace'
    return str(raw)


# 设计稿里表面投影的主规格：outbound、颜色引用 $jn-shadow、y=1、blur=3。
# 223 次引用都是它（卡片 / 列表行 / 输入框 / 页头），所以单独做成令牌。
SURFACE_SHADOW = {"shadowType": "outer", "color": "$jn-shadow", "offset": {"x": 0, "y": 1}, "blur": 3}


def surface_shadow_count(pen: dict) -> int:
    """数一遍表面投影在设计稿里出现多少次。

    设计稿改了阴影规格时这里会数到 0，脚本随之报错——比悄悄生成一条过期令牌好。
    """
    hits = 0

    def walk(node: object) -> None:
        nonlocal hits
        if isinstance(node, dict):
            if node.get("type") == "shadow":
                same = (
                    node.get("shadowType") == SURFACE_SHADOW["shadowType"]
                    and node.get("color") == SURFACE_SHADOW["color"]
                    and (node.get("offset") or {}) == SURFACE_SHADOW["offset"]
                    and node.get("blur") == SURFACE_SHADOW["blur"]
                )
                if same:
                    hits += 1
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for item in node:
                walk(item)

    for child in pen.get("children", []):
        walk(child)
    return hits


def main() -> None:
    pen = json.loads(PEN.read_text(encoding="utf-8"))
    variables: dict[str, dict] = pen.get("variables") or {}
    if not variables:
        raise SystemExit(f"{PEN} 里没有 variables，设计稿结构可能已变")

    colors: list[tuple[str, str, str, str]] = []   # (原名, css 名, 浅, 深)
    singles: list[tuple[str, str, str, str]] = []  # (原名, css 名, 值, tailwind 名)

    for name in sorted(variables):
        spec = variables[name]
        kind = spec.get("type")
        css_name, tw_name = classify(name, kind)
        light, dark = light_dark(spec)
        if kind == "color":
            colors.append((name, css_name, str(light), str(dark)))
        else:
            # 非颜色变量深浅共用一份，但 Tailwind 命名空间不同
            singles.append((name, css_name, css_value(kind, light), tw_name))

    lines: list[str] = [HEADER]

    # ── 浅色（默认） ────────────────────────────────────────────────
    lines.append(":root {")
    for _name, css_name, light, _dark in colors:
        lines.append(f"  {css_name}: {light};")
    for _name, css_name, value, _tw in singles:
        lines.append(f"  {css_name}: {value};")
    lines.append("}\n")

    # ── 深色覆盖 ───────────────────────────────────────────────────
    # 只覆盖颜色：半径 / 间距 / 字体不随主题变化，重复声明只会制造两份真相。
    lines.append("/* 深色主题：只覆盖颜色，几何与字体不随主题变化。 */")
    lines.append(".dark {")
    for _name, css_name, _light, dark in colors:
        lines.append(f"  {css_name}: {dark};")
    lines.append("}\n")

    # ── 表面投影 ───────────────────────────────────────────────────
    # 只有一档：卡片 / 列表行 / 输入框共用。颜色走 --jn-shadow，深浅主题各一份。
    surface_hits = surface_shadow_count(pen)
    if surface_hits == 0:
        raise SystemExit(
            "设计稿里找不到表面投影 0 1px 3px $jn-shadow：阴影规格可能已改，"
            "请核对 R1-00 设计语言后更新 SURFACE_SHADOW"
        )
    lines.append(f"/* 表面投影（设计稿里引用 {surface_hits} 次）：卡片、列表行、输入框共用。 */")
    lines.append(":root {")
    # 源变量名必须与 @theme 里的映射名不同：写成同名会生成
    # `--shadow-jn-surface: var(--shadow-jn-surface)` 这种自引用，阴影直接失效。
    lines.append("  --jn-shadow-surface: 0 1px 3px var(--jn-shadow);")
    lines.append("}\n")

    # ── 暴露给 Tailwind ────────────────────────────────────────────
    lines.append(
        "/* 以下把变量接进 Tailwind 4 的 @theme：页面用 bg-jn-primary / rounded-jn-card /\n"
        " * gap-jn-card 这类类名，不写 var()。inline 表示值在运行时解析，主题切换即时生效。 */"
    )
    lines.append("@theme inline {")
    for name, _css_name, _light, _dark in colors:
        lines.append(f"  --color-{name}: var(--{name});")
    for name, _css_name, _value, tw in singles:
        lines.append(f"  {tw}: var(--{name});")
    lines.append("  --shadow-jn-surface: var(--jn-shadow-surface);")
    lines.append("}")
    lines.append("")

    rendered = "\n".join(lines)

    if "--check" in sys.argv:
        current = OUT.read_text(encoding="utf-8") if OUT.is_file() else ""
        if current != rendered:
            print(
                "tokens.css 与设计稿不一致：请运行 python -I scripts/dev/extract-pen-tokens.py 重新生成",
                file=sys.stderr,
            )
            raise SystemExit(1)
        print(f"tokens.css 与设计稿一致（{len(colors)} 个颜色 / {len(singles)} 个其他变量）")
        return

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(rendered, encoding="utf-8")
    print(f"已生成 {OUT.relative_to(ROOT)}：{len(colors)} 个颜色 + {len(singles)} 个其他变量")


if __name__ == "__main__":
    main()
