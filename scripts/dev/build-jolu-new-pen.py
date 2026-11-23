#!/usr/bin/env python3
"""生成 docs/ui/jolu-new.pen：把当前前端主要界面的真实截图排成对照画板。

为什么直接写 JSON 而不走 Pencil MCP：
MCP 只操作「当前打开的文档」，而当前打开的必然是 Jolu.pen（设计稿原稿）。
用 save(uri) 另存实测会把原稿也写脏（2026-11-22 已发生一次，从 git 恢复）。
.pen 是公开的 JSON schema（pen.dev 文档 version 2.22），直接生成更安全、可重复。

用法（仓库根目录）：
    py scripts/dev/build-jolu-new-pen.py

前置：先跑 node scripts/dev/check-snapshot.mjs 生成 output/snapshot-shots/ 截图。
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SHOTS = ROOT / "output" / "snapshot-shots"
PEN = ROOT / "docs" / "ui" / "jolu-new.pen"
ASSETS = ROOT / "docs" / "ui" / "jolu-new-assets"

# (画板标题, 截图文件名, 组件路径)
LIGHT_MAIN = [
    ("灵动岛 · 设置", "island", "views/IslandView.tsx"),
    ("灵动岛窗口 · 经典", "island-classic", "views/classic/ClassicIslandWindow.tsx"),
    ("灵动岛窗口 · 胶囊", "island-capsule", "views/capsule/CapsuleWindow.tsx"),
    ("供应商 · 连接", "providers", "views/ProvidersView.tsx"),
    ("供应商 · 账号管理", "providers-accounts", "components/providers/AccountsPanel.tsx"),
    ("一键签到", "checkin", "views/CheckinView.tsx"),
    ("一键签到 · 批量管理", "checkin-batch", "components/checkin/AccountBatchControls.tsx"),
    ("用量统计 · 概览", "usage", "views/UsageView.tsx"),
    ("用量统计 · 分析", "usage-analysis", "components/usage/AnalysisPanel.tsx"),
    ("用量统计 · 调用明细", "usage-calls", "components/usage/CallsPanel.tsx"),
]
LIGHT_WORK = [
    ("会话线程", "sessions", "views/SessionsView.tsx"),
    ("自动化 · 任务", "automation", "views/AutomationView.tsx"),
    ("自动化 · 运行记录", "automation-runs", "components/automation/RunList.tsx"),
    ("Skills 管理", "skills", "views/SkillsView.tsx"),
]
LIGHT_ROUTING = [
    ("Agent 列表", "agents", "views/AgentsView.tsx"),
    ("模型路由 · 概览", "gateway", "components/gateway/OverviewPanel.tsx"),
    ("模型路由 · 路由流水", "gateway-log", "components/gateway/RouteLogPanel.tsx"),
    ("模型路由 · 路由分组", "gateway-groups", "components/gateway/GroupsPanel.tsx"),
    ("模型路由 · 对外访问", "gateway-access", "components/gateway/AccessPanel.tsx"),
    ("模型路由 · 中间件", "gateway-middleware", "components/gateway/MiddlewarePanel.tsx"),
    ("模型路由 · 模型池", "gateway-pool", "components/gateway/PoolPanel.tsx"),
]
LIGHT_SETTINGS = [
    ("设置 · 常规", "settings", "components/settings/GeneralSection.tsx"),
    ("设置 · 外观", "settings-appearance", "components/settings/AppearanceSection.tsx"),
    ("设置 · 网络与代理", "settings-network", "components/settings/NetworkSection.tsx"),
    ("设置 · 数据与存储", "settings-data", "components/settings/DataSection.tsx"),
    ("设置 · 通知", "settings-notifications", "components/settings/NotificationSection.tsx"),
    ("设置 · 关于与更新", "settings-about", "components/settings/AboutSection.tsx"),
]
DARK = [
    ("深色 · 供应商", "dark-providers"),
    ("深色 · 用量统计", "dark-usage"),
    ("深色 · 模型路由", "dark-gateway"),
    ("深色 · 设置", "dark-settings"),
]

SHOT_W, SHOT_H = 1280, 720      # 主面板窗口 1280×720 的截图（含外壳留白）
COLS = 4
CARD_GAP = 48
SECTION_GAP = 96
LEFT = 100
TOP = 80
BAR_H = 76

INK = "#111111"
MUTED = "#8a8a8a"
LINE = "#e4e4e7"

_ids = [0]


def nid(prefix: str = "n") -> str:
    _ids[0] += 1
    return f"{prefix}{_ids[0]}"


def shot_path(file: str) -> Path:
    """主页面截图是 panel-<name>.png，子页与深色是 <name>.png。"""
    for candidate in (SHOTS / f"panel-{file}.png", SHOTS / f"{file}.png"):
        if candidate.exists():
            return candidate
    return SHOTS / f"{file}.png"


def text(content: str, size: int, weight: str = "400", color: str = INK,
         width: int = 900, height: int = 48, family: str = "Inter") -> dict:
    return {
        "type": "text", "name": content[:24], "id": nid("t"),
        "content": content, "fontFamily": family, "fontSize": size, "fontWeight": weight,
        "fill": color, "width": width, "height": height,
    }


def shot_card(label: str, file: str, note: str, x: int, y: int) -> dict:
    """一张对照卡：上截图、下名称条。截图用 image fill 引用相对路径。"""
    return {
        "type": "frame", "name": label, "id": nid("card"),
        "x": x, "y": y, "width": SHOT_W, "height": SHOT_H + BAR_H,
        "fill": "#ffffff", "stroke": LINE, "strokeWidth": 1, "cornerRadius": 12,
        "layout": "vertical", "gap": 0, "padding": [0, 0, 0, 0], "clip": True,
        "children": [
            {
                "type": "rectangle", "name": "截图", "id": nid("shot"),
                "width": SHOT_W, "height": SHOT_H,
                "fill": {"type": "image", "url": f"./jolu-new-assets/{file}.png", "mode": "cover"},
            },
            {
                "type": "frame", "name": "名称条", "id": nid("bar"),
                "width": SHOT_W, "height": BAR_H, "fill": "#ffffff",
                "layout": "horizontal", "justifyContent": "space_between",
                "alignItems": "center", "padding": [0, 24, 0, 24],
                "children": [
                    text(label, 22, "600", INK, width=760, height=34),
                    text(note, 15, "400", MUTED, width=420, height=26, family="JetBrains Mono"),
                ],
            },
        ],
    }


def section(title: str, items: list, x: int, y: int) -> tuple[dict, int]:
    """一组：组标题 + 若干行卡片。返回（组节点, 该组占用的高度）。"""
    rows = (len(items) + COLS - 1) // COLS
    cards = []
    for i, (label, file, note) in enumerate(items):
        cards.append(shot_card(label, file, note, (i % COLS) * (SHOT_W + CARD_GAP),
                               (i // COLS) * (SHOT_H + BAR_H + CARD_GAP)))
    height = rows * (SHOT_H + BAR_H + CARD_GAP) - CARD_GAP + 28
    node = {
        "type": "frame", "name": title, "id": nid("sec"),
        "x": x, "y": y, "width": COLS * SHOT_W + (COLS - 1) * CARD_GAP, "height": height,
        "fill": "#ffffff", "layout": "vertical", "gap": 28, "padding": [0, 0, 0, 0],
        "children": [text(title, 30, "600", INK, width=2400, height=44)] + cards,
    }
    return node, height


def dark_section(items: list, x: int, y: int) -> dict:
    cards = [
        shot_card(label, file, "深色主题", (i % COLS) * (SHOT_W + CARD_GAP), 0)
        for i, (label, file) in enumerate(items)
    ]
    return {
        "type": "frame", "name": "05 · 深色主题对照", "id": nid("sec"),
        "x": x, "y": y, "width": COLS * SHOT_W + (COLS - 1) * CARD_GAP,
        "height": SHOT_H + BAR_H + 28,
        "fill": "#ffffff", "layout": "vertical", "gap": 28, "padding": [0, 0, 0, 0],
        "children": [text("05 · 深色主题对照", 30, "600", INK, width=900, height=44)] + cards,
    }


def main() -> None:
    missing = [f for _, f, _ in
               LIGHT_MAIN + LIGHT_WORK + LIGHT_ROUTING + LIGHT_SETTINGS
               if not shot_path(f).exists()]
    if missing:
        raise SystemExit(
            "缺少截图：" + "、".join(missing) +
            "\n先运行：node scripts/dev/check-snapshot.mjs"
        )

    # 截图复制到 .pen 旁边（image fill 用相对路径）
    ASSETS.mkdir(parents=True, exist_ok=True)
    copied = 0
    for _, file, _ in LIGHT_MAIN + LIGHT_WORK + LIGHT_ROUTING + LIGHT_SETTINGS:
        shutil.copy2(shot_path(file), ASSETS / f"{file}.png")
        copied += 1
    for _, file in DARK:
        src = shot_path(file)
        if src.exists():
            shutil.copy2(src, ASSETS / f"{file}.png")
            copied += 1

    width = COLS * SHOT_W + (COLS - 1) * CARD_GAP
    top = []

    header = {
        "type": "frame", "name": "标题区", "id": nid("hdr"),
        "x": LEFT, "y": TOP, "width": width, "height": 150,
        "fill": "#ffffff", "layout": "vertical", "gap": 12, "padding": [0, 0, 0, 0],
        "children": [
            text("Jolu 前端界面 · 重构基线", 54, "700", INK, width=1400, height=72),
            text(
                "2026-11-22 当前实现的真实渲染截图（1280×720，含标题栏／侧栏／内容区）。"
                "重构时逐屏对照；样式与动效规范见 docs/frontend-ui-guide.md。",
                19, "400", MUTED, width=2200, height=52,
            ),
        ],
    }
    top.append(header)

    y = TOP + 150 + SECTION_GAP
    for title, items in [
        ("01 · 主流程：灵动岛 → 供应商 → 一键签到 → 用量统计", LIGHT_MAIN),
        ("02 · 会话、自动化与 Skills", LIGHT_WORK),
        ("03 · Agent 与模型路由", LIGHT_ROUTING),
        ("04 · 设置", LIGHT_SETTINGS),
    ]:
        node, height = section(title, items, LEFT, y)
        top.append(node)
        y += height + SECTION_GAP

    top.append(dark_section(DARK, LEFT, y))

    doc = {"version": "2.22", "children": top}
    PEN.write_text(json.dumps(doc, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    size_mb = PEN.stat().st_size / 1024 / 1024
    print(f"写入 {PEN.relative_to(ROOT)} · {size_mb:.2f} MB · 顶层 {len(top)} 个")
    print(f"截图 {copied} 张 → {ASSETS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
