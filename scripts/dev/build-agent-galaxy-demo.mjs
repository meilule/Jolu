#!/usr/bin/env node
/**
 * 生成「Agent · 3D 深空星系与待接星坞拖曳入轨」交互式单文件预览 Demo（仅 3D 星系 + 列表两态）。
 *
 * 特性：
 * - WebGL2 GPU 着色器实时渲染 11,200 颗光谱恒星（银核膨隆 + 4 条对数旋臂 + 远景星穹）+ 425 团 FBM 体积发射星云与暗尘埃带；
 * - 浅色保持 R1-08-V1 矿物图鉴透亮质感，深色自动切换 R1-08-V2 深空天文台发光星云雷达；
 * - 3D 立体可交互升级：惯性轨道旋转、右键/Shift平移、宽幅缩放、点击行星飞镜聚焦追踪、3D 高度投影柱、卫星真三维公转、俯瞰/斜侧/银心预设视角；
 * - 左下「待接入星坞」拖出小星球在三维空间自由定位（滚轮可调高度 Y）松手即成偏心倾角新轨 + 一键直连。
 *
 * 用法：node scripts/dev/build-agent-galaxy-demo.mjs
 * 产物：output/agent-galaxy-demo.html（已被 Git 忽略）
 */
import { mkdir, readFile, stat, writeFile } from "node:fs/promises"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { GALAXY_WEBGL_SOURCE } from "./galaxy-webgl.mjs"
import { SPHERE_ATLAS_WEBGL_SOURCE } from "./sphere-atlas-webgl.mjs"

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const OUT = resolve(ROOT, "output/agent-galaxy-demo.html")

async function toDataUrl(relPath, mime) {
  const buf = await readFile(resolve(ROOT, relPath))
  return `data:${mime};base64,${buf.toString("base64")}`
}

const [
  interFont,
  monoFont,
  appIcon,
  claudeLogo,
  codexLogo,
  geminiLogo,
  opencodeLight,
  opencodeDark,
  deepseekLogo,
  qoderLight,
  qoderDark,
  kimiLight,
  kimiDark,
  workbuddyLogo,
  piLogo,
  zcodeLogo,
  qwenLogo,
  vscodeLogo,
  continueLogo,
  antigravityLogo,
  claudeRawSvg,
  codexRawSvg,
  opencodeRawSvg,
  deepseekRawSvg,
  qoderRawSvg,
  kimiRawSvg,
  workbuddyRawSvg,
  piRawSvg,
  qwenRawSvg,
  tokensCss,
] = await Promise.all([
  toDataUrl("frontend/public/fonts/Inter-Variable.woff2", "font/woff2"),
  toDataUrl("frontend/public/fonts/JetBrainsMono-Variable.woff2", "font/woff2"),
  toDataUrl("frontend/public/app-icon.png", "image/png"),
  toDataUrl("frontend/src/assets/platforms/claude-code.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/codex-agent.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/gemini-cli.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/opencode.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/opencode-dark.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/deepseek.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/qoder.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/qoder-dark.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/kimi.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/kimi-dark.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/workbuddy.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/agents/pi.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/platforms/zcode.png", "image/png"),
  toDataUrl("frontend/src/assets/platforms/qwen.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/agents/vscode.svg", "image/svg+xml"),
  toDataUrl("frontend/src/assets/agents/continue.png", "image/png"),
  toDataUrl("frontend/src/assets/platforms/antigravity.svg", "image/svg+xml"),
  readFile(resolve(ROOT, "frontend/src/assets/platforms/claude-code.svg"), "utf8"),
  readFile(resolve(ROOT, "frontend/src/assets/platforms/codex-agent.svg"), "utf8"),
  readFile(resolve(ROOT, "frontend/src/assets/platforms/opencode.svg"), "utf8"),
  readFile(resolve(ROOT, "frontend/src/assets/platforms/deepseek.svg"), "utf8"),
  readFile(resolve(ROOT, "frontend/src/assets/platforms/qoder.svg"), "utf8"),
  readFile(resolve(ROOT, "frontend/src/assets/platforms/kimi.svg"), "utf8"),
  readFile(resolve(ROOT, "frontend/src/assets/platforms/workbuddy.svg"), "utf8"),
  readFile(resolve(ROOT, "frontend/src/assets/agents/pi.svg"), "utf8"),
  readFile(resolve(ROOT, "frontend/src/assets/platforms/qwen.svg"), "utf8"),
  readFile(resolve(ROOT, "frontend/src/theme/tokens.css"), "utf8"),
])

const svgStrToDataUrl = (s) => `data:image/svg+xml;base64,${Buffer.from(s, "utf8").toString("base64")}`

const wbPaths = [...workbuddyRawSvg.matchAll(/<path[^>]*d="([^"]+)"[^>]*>/g)].map((m) => m[1])
const wbBodyD = wbPaths.find((d) => d.startsWith("M18.456")) ?? ""
const wbEyesD = wbPaths.find((d) => d.startsWith("M13.882")) ?? ""
const makeWorkBuddySvg = (c1, c2, eye, gradId) => svgStrToDataUrl(
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0.5 0.5 31 30"><defs><linearGradient id="${gradId}" x1="3" y1="2" x2="29" y2="28" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="${c1}"/><stop offset="1" stop-color="${c2}"/></linearGradient></defs><path d="${wbBodyD}" fill="url(#${gradId})"/><path d="${wbEyesD}" fill="${eye}"/></svg>`,
)
const codexStrippedSvg = codexRawSvg.replace(/<path[^>]*fill="#fff"[^>]*><\/path>/, "")
const piScaledSvg = svgStrToDataUrl(piRawSvg.replace('viewBox="0 0 800 800"', 'viewBox="110 110 580 580"'))
const opencodeFixedLight = svgStrToDataUrl(opencodeRawSvg.replace(/fill="currentColor"/g, 'fill="#18201D"'))
const qoderFixedLight = svgStrToDataUrl(qoderRawSvg.replace(/fill="currentColor"/g, 'fill="#18201D"'))
const kimiFixedLight = svgStrToDataUrl(kimiRawSvg.replace(/fill="currentColor"/g, 'fill="#18201D"'))

// 剥离圆角矩形底板，保留各厂商真实品牌色彩（浅色/深色各自微调暗部明度），供 WebGL2 360° 三维晶雕与星坞小晶球共用
const PURE_GLYPHS_LIGHT = {
  core: svgStrToDataUrl('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><defs><linearGradient id="joluCoreL" x1="4" y1="4" x2="20" y2="20" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#059669"/><stop offset="1" stop-color="#16C972"/></linearGradient></defs><circle cx="12" cy="12" r="8.6" fill="none" stroke="url(#joluCoreL)" stroke-width="1.65" stroke-dasharray="8 3.2"/><path fill="url(#joluCoreL)" d="M12 4.2L13.85 10.15L19.8 12L13.85 13.85L12 19.8L10.15 13.85L4.2 12L10.15 10.15Z"/></svg>'),
  "claude-code": claudeLogo,
  codex: svgStrToDataUrl(codexStrippedSvg.replace(/#B1A7FF/gi, "#7E70FF").replace(/#7A9DFF/gi, "#597BFF")),
  opencode: opencodeFixedLight,
  gemini: svgStrToDataUrl('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><defs><linearGradient id="gemL" x1="20" y1="5" x2="4" y2="19" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#EE4D5D"/><stop offset="0.48" stop-color="#B381DD"/><stop offset="1" stop-color="#207CFE"/></linearGradient></defs><path fill="url(#gemL)" d="M4.8 6.4l8.8 4.6-8.8 4.6v3.1l11.6-6.0v-3.4L4.8 3.3v3.1zm9.8 10.6h4.6v2.5h-4.6V17z"/></svg>'),
  qoder: qoderFixedLight,
  dsh: deepseekLogo,
  kimi: kimiFixedLight,
  workbuddy: makeWorkBuddySvg("#0EC8A9", "#01C886", "#057A55", "wbL"),
  pi: piScaledSvg,
  zcode: svgStrToDataUrl('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><defs><linearGradient id="zcL" x1="4" y1="4" x2="20" y2="20" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#0891B2"/><stop offset="1" stop-color="#06B6D4"/></linearGradient></defs><path fill="url(#zcL)" d="M4.5 5.4h7.4L9.8 8.6H4.5V5.4zm9.6 0H19.8L10.2 18.6H4.5L14.1 5.4zm-.3 10.0h6.0v3.2h-8.4l2.4-3.2z"/></svg>'),
  qwen: qwenLogo,
}

const PURE_GLYPHS_DARK = {
  core: svgStrToDataUrl('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><defs><linearGradient id="joluCoreD" x1="4" y1="4" x2="20" y2="20" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#2EE88C"/><stop offset="1" stop-color="#86FFBE"/></linearGradient></defs><circle cx="12" cy="12" r="8.6" fill="none" stroke="url(#joluCoreD)" stroke-width="1.65" stroke-dasharray="8 3.2"/><path fill="url(#joluCoreD)" d="M12 4.2L13.85 10.15L19.8 12L13.85 13.85L12 19.8L10.15 13.85L4.2 12L10.15 10.15Z"/></svg>'),
  "claude-code": svgStrToDataUrl(claudeRawSvg.replace(/#D97757/gi, "#EA8A68")),
  codex: svgStrToDataUrl(codexStrippedSvg.replace(/#3941FF/gi, "#5872FF")),
  opencode: opencodeDark,
  gemini: svgStrToDataUrl('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><defs><linearGradient id="gemD" x1="20" y1="5" x2="4" y2="19" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#FF6574"/><stop offset="0.48" stop-color="#C496EE"/><stop offset="1" stop-color="#3C8FFF"/></linearGradient></defs><path fill="url(#gemD)" d="M4.8 6.4l8.8 4.6-8.8 4.6v3.1l11.6-6.0v-3.4L4.8 3.3v3.1zm9.8 10.6h4.6v2.5h-4.6V17z"/></svg>'),
  qoder: qoderDark,
  dsh: svgStrToDataUrl(deepseekRawSvg.replace(/#4D6BFE/gi, "#6886FF")),
  kimi: kimiDark,
  workbuddy: makeWorkBuddySvg("#2AF0BF", "#06D692", "#A7F3D0", "wbD"),
  pi: piScaledSvg,
  zcode: svgStrToDataUrl('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><defs><linearGradient id="zcD" x1="4" y1="4" x2="20" y2="20" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#22D3EE"/><stop offset="1" stop-color="#67E8F9"/></linearGradient></defs><path fill="url(#zcD)" d="M4.5 5.4h7.4L9.8 8.6H4.5V5.4zm9.6 0H19.8L10.2 18.6H4.5L14.1 5.4zm-.3 10.0h6.0v3.2h-8.4l2.4-3.2z"/></svg>'),
  qwen: svgStrToDataUrl(qwenRawSvg.replace(/#082dff/gi, "#456AFF")),
}

const GLYPH_FALLBACK_COLORS = {
  light: {
    core: "#14B86B", "claude-code": "#D97757", codex: "#7A9DFF", opencode: "#18201D",
    gemini: "#B381DD", qoder: "#2ADB5C", dsh: "#4D6BFE", kimi: "#18201D",
    workbuddy: "#0EC8A9", pi: "#F09082", zcode: "#06B6D4", qwen: "#082DFF",
  },
  dark: {
    core: "#46F29F", "claude-code": "#EA8A68", codex: "#89A5FF", opencode: "#F0F0F0",
    gemini: "#C496EE", qoder: "#2ADB5C", dsh: "#6886FF", kimi: "#F0F0F0",
    workbuddy: "#2AF0BF", pi: "#F1BE58", zcode: "#22D3EE", qwen: "#456AFF",
  },
}

const LOGOS = {
  "claude-code": { light: claudeLogo, dark: claudeLogo },
  codex: { light: codexLogo, dark: codexLogo },
  gemini: { light: geminiLogo, dark: geminiLogo },
  opencode: { light: opencodeFixedLight, dark: opencodeDark },
  dsh: { light: deepseekLogo, dark: deepseekLogo },
  qoder: { light: qoderFixedLight, dark: qoderDark },
  kimi: { light: kimiFixedLight, dark: kimiDark },
  workbuddy: { light: workbuddyLogo, dark: workbuddyLogo },
  pi: { light: piLogo, dark: piLogo },
  zcode: { light: zcodeLogo, dark: zcodeLogo },
  qwen: { light: qwenLogo, dark: qwenLogo },
  vscode: { light: vscodeLogo, dark: vscodeLogo },
  continue: { light: continueLogo, dark: continueLogo },
  antigravity: { light: antigravityLogo, dark: antigravityLogo },
  appIcon,
}

const tokenCssClean = tokensCss.split("/* 以下把变量接进 Tailwind 4")[0].trim()

const html = `<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>Jolu · Agent 3D 星系与列表交互 Demo</title>
<style>
@font-face { font-family: "Inter"; src: url("${interFont}") format("woff2"); font-weight: 100 900; font-display: swap; }
@font-face { font-family: "JetBrains Mono"; src: url("${monoFont}") format("woff2"); font-weight: 100 800; font-display: swap; }
${tokenCssClean}

:root {
  --demo-stage-bg: #E5EFEA;
  --galaxy-bg-1: #FCFFFD;
  --galaxy-bg-2: #EEF7F2;
  --galaxy-bg-3: #DCECE4;
  --galaxy-card-border: rgba(18, 62, 40, 0.10);
  --galaxy-glass: rgba(255, 255, 255, 0.86);
  --galaxy-glass-strong: rgba(255, 255, 255, 0.94);
  --galaxy-dock-shadow: 0 12px 32px rgba(15, 56, 36, 0.10), 0 2px 6px rgba(15, 56, 36, 0.05);
  --galaxy-pill-bg: rgba(255, 255, 255, 0.90);
  --galaxy-pill-border: rgba(18, 62, 40, 0.11);
  --galaxy-pill-shadow: 0 5px 16px rgba(16, 58, 38, 0.09);
  --motion-ease-spring: cubic-bezier(0.22, 1.24, 0.36, 1);
  --motion-ease-std: cubic-bezier(0.2, 0.8, 0.2, 1);
}

.dark {
  --demo-stage-bg: #030504;
  --galaxy-bg-1: #07140F;
  --galaxy-bg-2: #030907;
  --galaxy-bg-3: #010302;
  --galaxy-card-border: rgba(255, 255, 255, 0.10);
  --galaxy-glass: rgba(14, 21, 18, 0.80);
  --galaxy-glass-strong: rgba(15, 22, 19, 0.92);
  --galaxy-dock-shadow: 0 18px 46px rgba(0, 0, 0, 0.65), 0 0 0 1px rgba(255,255,255,0.06);
  --galaxy-pill-bg: rgba(12, 18, 15, 0.88);
  --galaxy-pill-border: rgba(255, 255, 255, 0.12);
  --galaxy-pill-shadow: 0 6px 18px rgba(0, 0, 0, 0.45);
}

* { box-sizing: border-box; }
html, body {
  margin: 0; height: 100%;
  font-family: "Inter", "Segoe UI", "Microsoft YaHei UI", system-ui, sans-serif;
  background: var(--demo-stage-bg); color: var(--jn-text);
  overflow: hidden; user-select: none; -webkit-font-smoothing: antialiased;
  transition: background-color 220ms ease, color 180ms ease;
}
.mono { font-family: "JetBrains Mono", Consolas, monospace; font-variant-numeric: tabular-nums; }
.tnum { font-variant-numeric: tabular-nums; }
.spacer { flex: 1; min-width: 8px; }

.demo-backdrop { height: 100vh; width: 100vw; display: grid; place-items: center; padding: 14px; }
.window-shell {
  width: min(1320px, 100%); height: min(860px, 100%);
  display: flex; border-radius: 18px;
  background: var(--jn-stage); border: 1px solid var(--jn-line);
  box-shadow: 0 24px 64px rgba(0, 0, 0, 0.16); overflow: hidden;
}
.dark .window-shell { box-shadow: 0 28px 80px rgba(0, 0, 0, 0.72); }

/* 侧栏 */
.sidebar {
  width: 196px; flex-shrink: 0; padding: 18px 10px 18px 12px;
  display: flex; flex-direction: column; gap: 4px; background: var(--jn-stage);
  transition: width 180ms var(--motion-ease-std);
}
.sidebar.collapsed { width: 68px; padding: 18px 10px; }
.sidebar-logo { height: 40px; padding: 0 6px; display: flex; align-items: center; gap: 9px; }
.sidebar.collapsed .sidebar-logo { justify-content: center; padding: 0; }
.sidebar-logo img { width: 28px; height: 28px; border-radius: 7px; flex-shrink: 0; }
.sidebar.collapsed .sidebar-wordmark, .sidebar.collapsed .nav-label, .sidebar.collapsed .sidebar-island-meta { display: none; }
.icon-btn {
  width: 30px; height: 30px; border: 0; border-radius: 9px; background: transparent;
  color: var(--jn-text-3); display: grid; place-items: center; cursor: pointer;
  transition: transform 140ms var(--motion-ease-spring), background-color 120ms ease, color 120ms ease;
}
.icon-btn:hover { background: var(--jn-surface); color: var(--jn-text); }
.icon-btn:active { transform: scale(0.92); }
.nav-list { display: flex; flex-direction: column; gap: 3px; padding-top: 12px; flex: 1; }
.nav-item {
  height: 38px; padding: 0 11px; border-radius: 999px; display: flex; align-items: center; gap: 10px;
  font-size: 13px; font-weight: 500; color: var(--jn-text-2); cursor: pointer; border: 0; background: transparent; text-align: left;
  transition: background-color 140ms ease, color 140ms ease;
}
.sidebar.collapsed .nav-item { justify-content: center; padding: 0; }
.nav-item:hover { background: var(--jn-well); color: var(--jn-text); }
.nav-item.active { background: var(--jn-primary); color: var(--jn-on-primary); font-weight: 600; box-shadow: 0 4px 12px var(--jn-glow); }
.nav-item svg { width: 16px; height: 16px; flex-shrink: 0; }
.sidebar-foot { display: flex; flex-direction: column; gap: 6px; }
.island-mini {
  border-radius: 16px; background: var(--jn-surface); border: 1px solid var(--jn-line);
  padding: 10px 11px; display: flex; align-items: center; gap: 9px; box-shadow: var(--jn-shadow-surface);
}
.sidebar.collapsed .island-mini { justify-content: center; padding: 8px; }
.pulse-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--jn-primary); box-shadow: 0 0 0 4px var(--jn-primary-soft); flex-shrink: 0; }

/* 主内容区 */
.main-col { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; padding: 0 18px 16px 4px; gap: 10px; }
.page-header { height: 60px; flex-shrink: 0; display: flex; align-items: center; gap: 10px; padding-top: 6px; }
.page-title { font-size: 24px; font-weight: 700; margin: 0; letter-spacing: -0.02em; }
.demo-badge {
  height: 24px; padding: 0 10px; border-radius: 999px;
  background: var(--jn-primary-soft); color: var(--jn-primary-ink);
  font-size: 11px; font-weight: 600; display: inline-flex; align-items: center; gap: 5px;
}
.btn {
  height: 33px; padding: 0 13px; border-radius: 999px; border: 1px solid var(--jn-line);
  background: var(--jn-surface); color: var(--jn-text); font-size: 12.5px; font-weight: 600;
  display: inline-flex; align-items: center; gap: 6px; cursor: pointer; white-space: nowrap;
  box-shadow: var(--jn-shadow-surface);
  transition: transform 150ms var(--motion-ease-spring), background-color 140ms ease;
}
.btn:hover { transform: translateY(-1px); background: var(--jn-well); }
.btn:active { transform: scale(0.96); }
.btn-primary { background: var(--jn-primary); border-color: var(--jn-primary); color: var(--jn-on-primary); }
.btn-primary:hover { background: var(--jn-primary-hover); }
.btn-danger { background: var(--jn-danger-soft); border-color: transparent; color: var(--jn-danger); }

/* 3D 舞台容器 */
.stage-card {
  position: relative; flex: 1; min-width: 0; min-height: 0; border-radius: 24px;
  border: 1px solid var(--galaxy-card-border);
  background: radial-gradient(circle at 50% 48%, var(--galaxy-bg-1) 0%, var(--galaxy-bg-2) 55%, var(--galaxy-bg-3) 100%);
  overflow: hidden; box-shadow: var(--jn-shadow-surface); cursor: grab;
  transition: background 280ms ease, border-color 220ms ease;
}
.stage-card.is-rotating { cursor: grabbing; }
.stage-card.is-panning { cursor: move; }
.stage-card.is-list { cursor: default; }

#galaxy-webgl, #galaxy-overlay, #galaxy-dom {
  position: absolute; inset: 0; width: 100%; height: 100%; display: block; pointer-events: none;
}
#galaxy-webgl { z-index: 1; }
#galaxy-overlay { z-index: 2; }
#galaxy-dom { z-index: 3; overflow: hidden; }

/* 顶部控条（单行紧凑防折行） */
.top-hud {
  position: absolute; top: 14px; left: 14px; right: 14px; z-index: 40;
  display: flex; align-items: center; gap: 6px; flex-wrap: nowrap; pointer-events: none;
}
.top-hud > * { pointer-events: auto; flex-shrink: 0; }
.seg-group {
  display: inline-flex; align-items: center; gap: 2px; padding: 3px; border-radius: 999px;
  background: var(--galaxy-glass-strong); border: 1px solid var(--galaxy-pill-border);
  backdrop-filter: blur(12px); box-shadow: var(--jn-shadow-surface);
}
.seg-btn {
  height: 28px; padding: 0 9.5px; border: 0; border-radius: 999px; background: transparent;
  color: var(--jn-text-2); font-size: 11.5px; font-weight: 600; cursor: pointer; white-space: nowrap;
  display: inline-flex; align-items: center; gap: 4px;
  transition: color 140ms ease, background-color 140ms ease, transform 140ms var(--motion-ease-spring);
}
.seg-btn:hover { color: var(--jn-text); }
.seg-btn.active { background: var(--jn-primary); color: var(--jn-on-primary); box-shadow: 0 2px 8px var(--jn-glow); }

.alert-pill {
  height: 34px; padding: 0 12px; border-radius: 999px;
  border: 1px solid rgba(224, 72, 77, 0.25); background: var(--jn-danger-soft); color: var(--jn-danger);
  font-size: 12px; font-weight: 600; display: inline-flex; align-items: center; gap: 6px; cursor: pointer;
  transition: transform 140ms var(--motion-ease-spring);
}
.alert-pill:hover { transform: translateY(-1px); }
.alert-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--jn-danger); animation: blink 1.8s ease-in-out infinite; }
@keyframes blink { 0%,100%{ opacity:1; transform:scale(1);} 50%{ opacity:.45; transform:scale(.82);} }

.search-box {
  height: 34px; width: 118px; border-radius: 999px;
  border: 1px solid var(--galaxy-pill-border); background: var(--galaxy-glass-strong);
  backdrop-filter: blur(12px); display: flex; align-items: center; gap: 6px; padding: 0 11px;
  color: var(--jn-text-3); box-shadow: var(--jn-shadow-surface);
}
.search-box input { width: 100%; border: 0; outline: none; background: transparent; color: var(--jn-text); font-size: 12px; font-family: inherit; }
.search-box input::placeholder { color: var(--jn-text-3); }

/* 中央 Jolu 纯净翡翠恒星核（方案四：100% 纯净无遮挡光球恒星 + 柔润星仪光环 + 底部解耦品牌胶囊） */
.jolu-core {
  position: absolute; transform: translate(-50%, -50%);
  width: 108px; height: 108px; border-radius: 50%;
  display: grid; place-items: center;
  pointer-events: auto; cursor: grab; touch-action: none;
  transition: filter 200ms ease;
}
.jolu-core:hover { filter: brightness(1.06); }
.jolu-core.scrubbing { cursor: grabbing; }
.core-globe {
  position: absolute; inset: -9.5%; width: 119%; height: 119%;
  border-radius: 50%; display: block; z-index: 1; pointer-events: none;
}
.core-gyros {
  position: absolute; inset: -24%; width: 148%; height: 148%;
  pointer-events: none; z-index: 2; overflow: visible;
}
.core-gyro-1 { transform-origin: 0 0; animation: slowSpin 22s linear infinite; }
.core-gyro-2 { transform-origin: 0 0; animation: slowSpinRev 32s linear infinite; }
@keyframes slowSpin { from{ transform: rotate(0deg);} to{ transform: rotate(360deg);} }
@keyframes slowSpinRev { from{ transform: rotate(360deg);} to{ transform: rotate(0deg);} }
.jolu-core.selected .core-focus-ring { opacity: 1; transform: scale(1.06); }
.jolu-core.magnet-active .core-focus-ring {
  opacity: 1; transform: scale(1.16); box-shadow: 0 0 0 8px var(--jn-primary-soft), 0 0 42px var(--jn-primary);
}
.core-focus-ring {
  position: absolute; inset: -2px; border-radius: 50%; z-index: 0; pointer-events: none;
  border: 1.6px solid var(--jn-primary); opacity: 0;
  box-shadow: 0 0 28px var(--jn-glow); transition: opacity 220ms ease, transform 220ms var(--motion-ease-spring);
}
.jolu-core-pill {
  position: absolute; top: calc(100% + 7px); left: 50%; transform: translateX(-50%); z-index: 4;
  padding: 3px 9px 3px 8px; border-radius: 999px; white-space: nowrap; pointer-events: auto;
  background: var(--galaxy-pill-bg); border: 1px solid var(--galaxy-pill-border);
  backdrop-filter: blur(10px); box-shadow: var(--galaxy-pill-shadow);
  display: flex; align-items: center; gap: 5px; font-size: 10.5px; color: var(--jn-text);
  transition: border-color 160ms ease, transform 160ms var(--motion-ease-spring);
}
.jolu-core:hover .jolu-core-pill, .jolu-core.selected .jolu-core-pill { border-color: var(--jn-primary); }
.jolu-core-pill .ok { width: 6.5px; height: 6.5px; border-radius: 50%; background: var(--jn-primary); flex-shrink: 0; }
.jolu-core-pill strong { font-weight: 700; }
.jolu-core-sub { font-size: 9.5px; font-weight: 600; color: var(--jn-text-3); }
.jolu-core-port { font-size: 9px; padding: 1px 5px; border-radius: 999px; background: var(--jn-primary-soft); color: var(--jn-primary-ink); font-weight: 600; }

/* 3D 折射晶核星球 + 极简状态铭牌（Logo 已三维内雕于晶核内部，底部胶囊不再重复小图标） */
.planet-node {
  position: absolute; transform: translate(-50%, -50%); pointer-events: auto; cursor: grab; touch-action: none;
  display: flex; flex-direction: column; align-items: center;
  transition: opacity 200ms ease, filter 200ms ease;
}
.planet-node.scrubbing { cursor: grabbing; }
.planet-node.dimmed { opacity: 0.22; filter: saturate(0.35); }
.planet-hit { position: relative; display: grid; place-items: center; border-radius: 50%; transition: transform 190ms var(--motion-ease-spring); }
.planet-sphere {
  position: absolute; left: -9.5%; top: -9.5%; width: 119%; height: 119%;
  border-radius: 50%; z-index: 1; pointer-events: none; overflow: hidden;
}
.planet-sphere canvas { width: 100%; height: 100%; display: block; border-radius: 50%; }
.planet-hit-ring {
  position: absolute; inset: 0; border-radius: 50%; z-index: 2; pointer-events: none;
  transition: box-shadow 180ms ease;
}
.planet-ring-svg { position: absolute; left: 50%; top: 50%; width: 206%; height: 206%; transform: translate(-50%, -50%); overflow: visible; pointer-events: none; }
.planet-ring-svg.back { z-index: 0; }
.planet-ring-svg.front { z-index: 3; }
.planet-node:hover .planet-hit { transform: scale(1.09); }
.planet-node.selected .planet-hit .planet-hit-ring { box-shadow: 0 0 0 1.8px var(--jn-primary), 0 0 26px var(--jn-glow); }
.planet-node.is-drift .planet-hit .planet-hit-ring { box-shadow: 0 0 0 1.8px var(--jn-danger), 0 0 22px rgba(224, 72, 77, 0.50); }
.planet-node.is-warn .planet-hit .planet-hit-ring { box-shadow: 0 0 0 1.4px rgba(245, 184, 61, 0.52); }

.planet-label {
  margin-top: 6px; padding: 3px 9px 3px 8px; border-radius: 999px;
  background: var(--galaxy-pill-bg); border: 1px solid var(--galaxy-pill-border);
  backdrop-filter: blur(10px); box-shadow: var(--galaxy-pill-shadow);
  display: flex; align-items: center; gap: 5px; white-space: nowrap; font-size: 10.5px; line-height: 1.2;
  transition: border-color 160ms ease, transform 160ms var(--motion-ease-spring);
}
.planet-node:hover .planet-label, .planet-node.selected .planet-label {
  border-color: var(--jn-primary); transform: translateY(1px);
}
.planet-node.is-drift .planet-label { border-color: var(--jn-danger); }
.planet-label strong { font-weight: 700; color: var(--jn-text); }
.planet-label .label-model { font-size: 9.5px; color: var(--jn-text-3); max-width: 92px; overflow: hidden; text-overflow: ellipsis; }
.planet-node.is-drift .planet-label .label-model { color: var(--jn-danger); font-weight: 600; }
.planet-node.is-warn .planet-label .label-model { color: var(--jn-warning); font-weight: 600; }
.label-inline-dot {
  display: inline-block; width: 6.5px; height: 6.5px; border-radius: 50%; background: var(--jn-primary); flex-shrink: 0;
}
.label-inline-dot.warn { background: var(--jn-warning); }
.label-inline-dot.danger { background: var(--jn-danger); }

/* 3D 卫星小标签（缩放接近或选中时浮现） */
.moon-pill {
  position: absolute; transform: translate(-50%, -140%);
  padding: 1px 6px; border-radius: 999px; font-size: 9.5px; font-weight: 600;
  background: var(--galaxy-glass-strong); border: 1px solid var(--galaxy-pill-border);
  color: var(--jn-text-2); pointer-events: none; white-space: nowrap;
}

/* 目标捕获占位槽 */
.capture-slot {
  position: absolute; transform: translate(-50%, -50%) scale(0.84);
  width: 62px; height: 62px; border-radius: 50%;
  border: 2px dashed var(--jn-primary);
  background: radial-gradient(circle, rgba(22, 168, 98, 0.24) 0%, rgba(22, 168, 98, 0.04) 70%, transparent 100%);
  display: none; place-items: center; pointer-events: none;
  transition: transform 160ms var(--motion-ease-spring);
}
.capture-slot.visible { display: grid; opacity: 0.85; transform: translate(-50%, -50%) scale(1); }
.capture-slot.hot { opacity: 1; transform: translate(-50%, -50%) scale(1.18); border-style: solid; box-shadow: 0 0 0 8px var(--jn-primary-soft), 0 0 28px var(--jn-glow); }
.capture-slot-tag {
  position: absolute; top: calc(100% + 7px); white-space: nowrap;
  padding: 3px 10px; border-radius: 999px; background: var(--jn-primary); color: var(--jn-on-primary);
  font-size: 10.5px; font-weight: 600; box-shadow: 0 6px 16px var(--jn-glow);
}

/* 左下角：待接入星坞小卡片 */
.hangar-card {
  position: absolute; left: 16px; bottom: 16px; z-index: 45;
  width: 278px; max-height: 396px; display: flex; flex-direction: column;
  border-radius: 20px; background: var(--galaxy-glass-strong); border: 1px solid var(--galaxy-pill-border);
  backdrop-filter: blur(16px); box-shadow: var(--galaxy-dock-shadow); overflow: hidden;
  transition: max-height 220ms var(--motion-ease-std), width 200ms var(--motion-ease-std);
}
.hangar-card.collapsed { max-height: 44px; width: 222px; }
.hangar-head { height: 44px; flex-shrink: 0; padding: 0 12px 0 14px; display: flex; align-items: center; gap: 8px; cursor: pointer; border-bottom: 1px solid var(--jn-line); }
.hangar-card.collapsed .hangar-head { border-bottom-color: transparent; }
.hangar-title { font-size: 12.5px; font-weight: 700; color: var(--jn-text); }
.hangar-count { min-width: 20px; height: 20px; padding: 0 6px; border-radius: 999px; background: var(--jn-primary-soft); color: var(--jn-primary-ink); font-size: 11px; font-weight: 700; display: grid; place-items: center; }
.hangar-hint { font-size: 10.5px; color: var(--jn-text-3); margin-left: auto; }
.hangar-body { padding: 7px; display: flex; flex-direction: column; gap: 5px; overflow-y: auto; }
.hangar-body::-webkit-scrollbar { width: 4px; }
.hangar-body::-webkit-scrollbar-thumb { background: var(--jn-line); border-radius: 4px; }
.hangar-item {
  display: flex; align-items: center; gap: 10px; padding: 7px 9px; border-radius: 14px;
  border: 1px solid transparent; background: var(--jn-well); cursor: grab; touch-action: none;
  transition: transform 150ms var(--motion-ease-spring), border-color 140ms ease, background-color 140ms ease;
}
.hangar-item:hover { background: var(--jn-surface); border-color: var(--jn-mint); transform: translateY(-1px); }
.hangar-item.launching { border: 1.5px dashed var(--jn-primary); background: var(--jn-primary-wash); opacity: 0.55; }
.hangar-orb {
  width: 34px; height: 34px; border-radius: 50%; flex-shrink: 0; display: grid; place-items: center;
  box-shadow: inset -4px -5px 8px rgba(0,0,0,0.16), inset 3px 3px 6px rgba(255,255,255,0.6), 0 3px 8px rgba(0,0,0,0.12);
}
.hangar-orb img { width: 19px; height: 19px; object-fit: contain; pointer-events: none; }
.hangar-info { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 1px; }
.hangar-name { font-size: 12.5px; font-weight: 700; color: var(--jn-text); }
.hangar-sub { font-size: 10.5px; color: var(--jn-text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.hangar-sub.warn { color: var(--jn-warning); }
.hangar-plus {
  width: 26px; height: 26px; border-radius: 999px; border: 1px solid var(--jn-line);
  background: var(--jn-surface); color: var(--jn-primary-ink); font-size: 15px; font-weight: 600;
  display: grid; place-items: center; cursor: pointer; flex-shrink: 0;
  transition: transform 150ms var(--motion-ease-spring), background-color 140ms ease, color 140ms ease;
}
.hangar-plus:hover { background: var(--jn-primary); border-color: var(--jn-primary); color: var(--jn-on-primary); transform: scale(1.1); }
.hangar-empty { padding: 14px 10px; text-align: center; font-size: 11.5px; color: var(--jn-text-2); display: flex; flex-direction: column; align-items: center; gap: 6px; }
.hangar-unsupported { margin-top: 2px; padding: 7px 9px; border-top: 1px dashed var(--jn-line); display: flex; align-items: center; gap: 8px; font-size: 11px; color: var(--jn-text-3); cursor: pointer; }
.hangar-unsupported:hover { color: var(--jn-text-2); }

/* 右侧聚焦详情设置抽屉（全景态右上完全留白，仅在点击星球拉近时滑出） */
.right-hud {
  position: absolute; top: 60px; right: 16px; z-index: 38; width: 312px;
  display: flex; flex-direction: column; gap: 9px; pointer-events: none;
}
.right-hud > * { pointer-events: auto; }
.inspect-back-pill {
  height: 26px; padding: 0 9px; border-radius: 999px; border: 1px solid var(--jn-line);
  background: var(--jn-well); color: var(--jn-text-2); font-size: 11px; font-weight: 600;
  display: inline-flex; align-items: center; gap: 3px; cursor: pointer; flex-shrink: 0;
  transition: background-color 140ms ease, color 140ms ease, transform 140ms var(--motion-ease-spring);
}
.inspect-back-pill:hover { background: var(--jn-primary-soft); color: var(--jn-primary-ink); border-color: var(--jn-mint); transform: translateX(-1px); }

.inspect-card {
  padding: 14px; border-radius: 20px; background: var(--galaxy-glass-strong); border: 1px solid var(--galaxy-pill-border);
  backdrop-filter: blur(16px); box-shadow: var(--galaxy-dock-shadow); display: none; flex-direction: column; gap: 10px;
  max-height: calc(100vh - 208px); overflow-y: auto;
  animation: popIn 200ms var(--motion-ease-spring) both;
}
.inspect-card::-webkit-scrollbar { width: 4px; }
.inspect-card::-webkit-scrollbar-thumb { background: var(--jn-line); border-radius: 4px; }
.inspect-card.open { display: flex; }
@keyframes popIn { from{ opacity:0; transform: translateY(6px) scale(.97);} to{ opacity:1; transform:none;} }
.inspect-head { display: flex; align-items: center; gap: 9px; }
.inspect-head img { width: 32px; height: 32px; object-fit: contain; }
.inspect-name { font-size: 14px; font-weight: 700; color: var(--jn-text); }
.inspect-path { font-size: 10.5px; color: var(--jn-text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.setting-section {
  border-radius: 14px; background: var(--jn-well); padding: 9px 10px; display: flex; flex-direction: column; gap: 8px;
}
.setting-sec-title { font-size: 10px; font-weight: 700; color: var(--jn-text-3); letter-spacing: 0.04em; text-transform: uppercase; display: flex; align-items: center; justify-content: space-between; }
.setting-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: 11.5px; }
.setting-row label { color: var(--jn-text-2); font-weight: 600; flex-shrink: 0; }
.setting-select {
  height: 27px; max-width: 158px; border-radius: 9px; border: 1px solid var(--jn-line);
  background: var(--jn-surface); color: var(--jn-text); font-size: 11px; font-weight: 600;
  padding: 0 7px; font-family: inherit; outline: none; cursor: pointer;
}
.setting-slider {
  width: 118px; accent-color: var(--jn-primary); cursor: pointer; height: 4px;
}
.seg-mini {
  display: inline-flex; padding: 2px; border-radius: 9px; background: var(--jn-surface); border: 1px solid var(--jn-line); gap: 2px;
}
.seg-mini button {
  height: 22px; padding: 0 7px; border: 0; border-radius: 7px; background: transparent; color: var(--jn-text-3);
  font-size: 10.5px; font-weight: 600; cursor: pointer;
}
.seg-mini button.active { background: var(--jn-primary); color: var(--jn-on-primary); }
.tier-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 5px; }
.tier-chip {
  padding: 5px 8px; border-radius: 10px; background: var(--jn-surface); border: 1px solid transparent;
  font-size: 10.5px; display: flex; flex-direction: column; gap: 1px; cursor: pointer; text-align: left;
  transition: border-color 140ms ease, transform 120ms ease;
}
.tier-chip:hover { border-color: var(--jn-mint); }
.tier-chip.active { border-color: var(--jn-primary); background: var(--jn-primary-wash); }
.tier-chip span { font-size: 9.5px; color: var(--jn-text-3); }
.tier-chip b { color: var(--jn-text); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.inspect-actions { display: flex; gap: 6px; flex-wrap: wrap; }
.inspect-actions .btn { height: 29px; font-size: 11.5px; padding: 0 10px; flex: 1; justify-content: center; }

/* 右下 3D 导航舱 + 不可接胶囊 */
.bottom-right-hud { position: absolute; right: 16px; bottom: 16px; z-index: 40; display: flex; align-items: center; gap: 8px; }
.nav-pod {
  display: inline-flex; align-items: center; gap: 2px; padding: 3px; border-radius: 999px;
  background: var(--galaxy-glass-strong); border: 1px solid var(--galaxy-pill-border);
  backdrop-filter: blur(12px); box-shadow: var(--jn-shadow-surface);
}
.pod-btn {
  height: 26px; padding: 0 9px; border: 0; border-radius: 999px; background: transparent;
  color: var(--jn-text-2); font-size: 11px; font-weight: 600; cursor: pointer; display: inline-flex; align-items: center; gap: 4px;
  transition: background-color 120ms ease, color 120ms ease;
}
.pod-btn:hover { background: var(--jn-well); color: var(--jn-text); }
.pod-btn.active { background: var(--jn-primary-soft); color: var(--jn-primary-ink); }
.mini-pill {
  height: 32px; padding: 0 11px; border-radius: 999px; border: 1px solid var(--galaxy-pill-border);
  background: var(--galaxy-glass-strong); backdrop-filter: blur(12px); color: var(--jn-text-2);
  font-size: 11px; font-weight: 600; display: inline-flex; align-items: center; gap: 5px; cursor: pointer;
  box-shadow: var(--jn-shadow-surface); transition: transform 150ms var(--motion-ease-spring), color 120ms ease;
}
.mini-pill:hover { transform: translateY(-1px); color: var(--jn-text); }

.stage-foot-hint {
  position: absolute; left: 50%; bottom: 16px; transform: translateX(-50%); z-index: 25;
  padding: 5px 12px; border-radius: 999px; background: var(--galaxy-glass); border: 1px solid var(--galaxy-pill-border);
  font-size: 11px; color: var(--jn-text-3); pointer-events: none; white-space: nowrap;
}

/* 悬浮拖拽球 */
.drag-orb {
  position: fixed; left: 0; top: 0; width: 52px; height: 52px; margin: -26px 0 0 -26px;
  border-radius: 50%; z-index: 999; pointer-events: none; display: none; place-items: center;
  box-shadow: 0 14px 32px rgba(22, 168, 98, 0.35), inset -6px -7px 14px rgba(0,0,0,0.22), inset 4px 4px 10px rgba(255,255,255,0.75);
}
.drag-orb.active { display: grid; }
.drag-orb img { width: 28px; height: 28px; object-fit: contain; }
.drag-orb-ring { position: absolute; inset: -7px; border-radius: 50%; border: 1.5px solid var(--jn-primary); opacity: 0.7; }

/* 列表模式（R1-08 结构化工作台：概览统计条 + 分组 3 列精致卡片 + 不可接折叠条 + 同步右侧详情抽屉） */
.stage-card.is-list { background: var(--jn-stage); border-color: var(--jn-line); }
.stage-card.is-list #btn-drift-jump { display: none !important; }
.stage-card.is-list .seg-group,
.stage-card.is-list .search-box {
  background: var(--jn-surface); border-color: var(--jn-line); backdrop-filter: none;
}
.stage-card.is-list .search-box { width: 184px; }
.stage-card.is-list .inspect-card {
  background: var(--jn-surface); border-color: var(--jn-line); backdrop-filter: none;
  box-shadow: var(--jn-shadow-surface);
}
.list-view {
  position: absolute; inset: 56px 0 0 0; overflow-y: auto; display: none;
  flex-direction: column; gap: 16px; padding: 12px 16px 24px; z-index: 30;
  transition: right 200ms var(--motion-ease-std);
}
.list-view > * { flex-shrink: 0; }
.list-view.active { display: flex; }
.list-view::-webkit-scrollbar { width: 5px; }
.list-view::-webkit-scrollbar-thumb { background: var(--jn-line); border-radius: 99px; }
.stage-card.is-list.list-has-inspect .list-view { right: 334px; }
.stage-card.is-list .right-hud { top: 64px; right: 16px; }

.list-overview {
  display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 10px 14px;
  padding: 12px 16px; border-radius: 18px;
  background: var(--jn-surface); border: 1px solid var(--jn-line); box-shadow: var(--jn-shadow-surface);
}
.list-overview-left { display: flex; align-items: center; gap: 10px; min-width: 0; flex-wrap: wrap; }
.list-overview-title { font-size: 14px; font-weight: 700; color: var(--jn-text); letter-spacing: -0.01em; }
.list-gateway-badge {
  height: 24px; padding: 0 9px; border-radius: 999px;
  background: var(--jn-primary-soft); color: var(--jn-primary-ink);
  font-size: 11px; font-weight: 600; display: inline-flex; align-items: center; gap: 5px;
}
.list-gateway-badge .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--jn-primary); }
.list-kpis { display: flex; align-items: center; gap: 7px; flex-wrap: wrap; }
.list-kpi-chip {
  height: 26px; padding: 0 10px; border-radius: 999px;
  background: var(--jn-well); border: 1px solid var(--jn-line); color: var(--jn-text-2);
  font-size: 11px; font-weight: 500; display: inline-flex; align-items: center; gap: 4px;
}
.list-kpi-chip b { color: var(--jn-text); font-weight: 700; font-variant-numeric: tabular-nums; }
.list-kpi-chip.danger {
  background: var(--jn-danger-soft); border-color: rgba(224, 72, 77, 0.25); color: var(--jn-danger);
  font-weight: 600; cursor: pointer; transition: transform 140ms var(--motion-ease-spring);
}
.list-kpi-chip.danger:hover { transform: translateY(-1px); }

.list-section { display: flex; flex-direction: column; gap: 10px; }
.list-sec-head { display: flex; align-items: center; gap: 8px; padding: 0 2px; }
.list-sec-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--jn-primary); box-shadow: 0 0 0 3px var(--jn-primary-soft); flex-shrink: 0; }
.list-sec-dot.muted { background: var(--jn-text-3); box-shadow: 0 0 0 3px var(--jn-well); }
.list-sec-title { font-size: 12.5px; font-weight: 700; color: var(--jn-text); }
.list-sec-count {
  min-width: 20px; height: 20px; padding: 0 6px; border-radius: 999px;
  background: var(--jn-primary-soft); color: var(--jn-primary-ink);
  font-size: 11px; font-weight: 700; display: inline-grid; place-items: center; font-variant-numeric: tabular-nums;
}
.list-sec-count.muted { background: var(--jn-well); color: var(--jn-text-2); border: 1px solid var(--jn-line); }
.list-sec-note { margin-left: auto; font-size: 11px; color: var(--jn-text-3); }

.tool-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
.stage-card.is-list.list-has-inspect .tool-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
@media (max-width: 1140px) { .tool-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }

.tool-card {
  position: relative; border-radius: 20px; border: 1px solid var(--jn-line);
  background: var(--jn-surface); padding: 15px 15px 12px;
  display: flex; flex-direction: column; gap: 11px; box-shadow: var(--jn-shadow-surface);
  transition: transform 180ms var(--motion-ease-spring), border-color 160ms ease, box-shadow 180ms ease;
}
.tool-card:hover { transform: translateY(-2px); border-color: var(--jn-mint-2); box-shadow: 0 10px 24px rgba(15, 42, 28, 0.08); }
.dark .tool-card:hover { box-shadow: 0 12px 26px rgba(0, 0, 0, 0.34); }
.tool-card.is-selected { border-color: var(--jn-primary); box-shadow: 0 0 0 2px var(--jn-focus), var(--jn-shadow-surface); }
.tool-card.is-danger { border-color: rgba(224, 72, 77, 0.38); }
.tool-card.is-warn { border-color: rgba(217, 142, 11, 0.34); }
.tool-card.is-unsupported { opacity: 0.88; }

.tool-card-top { display: flex; align-items: center; gap: 10px; }
.tool-logo-tile {
  width: 40px; height: 40px; border-radius: 12px; flex-shrink: 0;
  display: grid; place-items: center; border: 1px solid var(--jn-line);
}
.tool-logo-tile img { width: 22px; height: 22px; object-fit: contain; }
.tool-identity { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 2px; cursor: pointer; }
.tool-name-row { display: flex; align-items: center; gap: 6px; min-width: 0; }
.tool-name { font-size: 14.5px; font-weight: 700; color: var(--jn-text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.tool-ver {
  padding: 1px 6px; border-radius: 999px; background: var(--jn-well); color: var(--jn-text-3);
  font-size: 9.5px; font-weight: 600; flex-shrink: 0;
}
.tool-path { font-size: 10.5px; color: var(--jn-text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

.tool-card-well {
  border-radius: 14px; background: var(--jn-well); padding: 10px 12px;
  display: flex; flex: 1; flex-direction: column; gap: 4px; cursor: pointer; min-height: 76px; justify-content: center;
  border: 1px solid transparent; transition: border-color 140ms ease, background-color 140ms ease;
}
.tool-card:hover .tool-card-well { border-color: var(--jn-line); }
.tool-well-meta { display: flex; align-items: center; justify-content: space-between; gap: 6px; font-size: 10.5px; color: var(--jn-text-3); }
.tool-routes-pill {
  padding: 1px 6px; border-radius: 999px; background: var(--jn-primary-soft); color: var(--jn-primary-ink);
  font-size: 10px; font-weight: 600; font-variant-numeric: tabular-nums;
}
.tool-routes-pill.warn { background: var(--jn-warning-soft); color: var(--jn-warning); }
.tool-routes-pill.muted { background: var(--jn-surface); color: var(--jn-text-3); border: 1px solid var(--jn-line); }
.tool-well-val {
  font-size: 13.5px; font-weight: 700; color: var(--jn-text);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.tool-well-val.danger { color: var(--jn-danger); }
.tool-well-val.warn { color: var(--jn-warning); }
.tool-well-sub { font-size: 11px; color: var(--jn-text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.tool-tier-pills { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 1px; }
.tool-tier-pill {
  padding: 1px 6px; border-radius: 6px; font-size: 10px; font-weight: 600;
  background: var(--jn-surface); color: var(--jn-text-2); border: 1px solid var(--jn-line);
}
.tool-tier-pill.active { background: var(--jn-primary-soft); color: var(--jn-primary-ink); border-color: var(--jn-mint); }

.tool-card-foot { display: flex; align-items: center; gap: 6px; min-height: 24px; }
.tool-status-pill {
  height: 22px; padding: 0 8px; border-radius: 999px; font-size: 10.5px; font-weight: 600;
  display: inline-flex; align-items: center; gap: 5px; max-width: 155px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.tool-status-pill .dot { width: 5.5px; height: 5.5px; border-radius: 50%; flex-shrink: 0; }
.tool-status-pill.normal { background: var(--jn-primary-soft); color: var(--jn-primary-ink); }
.tool-status-pill.normal .dot { background: var(--jn-primary); }
.tool-status-pill.warn { background: var(--jn-warning-soft); color: var(--jn-warning); }
.tool-status-pill.warn .dot { background: var(--jn-warning); }
.tool-status-pill.danger { background: var(--jn-danger-soft); color: var(--jn-danger); }
.tool-status-pill.danger .dot { background: var(--jn-danger); }
.tool-status-pill.idle { background: var(--jn-well); color: var(--jn-text-2); }
.tool-status-pill.idle .dot { background: var(--jn-text-3); }
.tool-status-pill.muted { background: var(--jn-well); color: var(--jn-text-3); }
.tool-status-pill.muted .dot { background: var(--jn-text-3); opacity: 0.6; }

.tool-foot-actions { margin-left: auto; display: inline-flex; align-items: center; gap: 4px; flex-shrink: 0; }
.tool-mini-btn {
  height: 24px; padding: 0 8px; border-radius: 999px; border: 1px solid var(--jn-line);
  background: var(--jn-well); color: var(--jn-text-2); font-size: 11px; font-weight: 600; cursor: pointer;
  transition: background-color 140ms ease, color 140ms ease, border-color 140ms ease;
}
.tool-mini-btn:hover { background: var(--jn-surface); color: var(--jn-text); border-color: var(--jn-mint); }
.tool-mini-btn.danger { background: var(--jn-danger-soft); color: var(--jn-danger); border-color: transparent; }
.tool-mini-btn.danger:hover { border-color: var(--jn-danger); }
.tool-mini-btn.warn { background: var(--jn-warning-soft); color: var(--jn-warning); border-color: transparent; }
.tool-detail-btn {
  height: 24px; padding: 0 8px; border-radius: 999px; border: 0;
  background: transparent; color: var(--jn-primary-ink); font-size: 11.5px; font-weight: 600;
  cursor: pointer; display: inline-flex; align-items: center; gap: 2px;
  transition: background-color 140ms ease;
}
.tool-detail-btn:hover { background: var(--jn-primary-wash); }

.list-blocked {
  border-radius: 18px; border: 1px solid var(--jn-line); background: var(--jn-surface); overflow: hidden;
  box-shadow: var(--jn-shadow-surface);
}
.list-blocked-head {
  width: 100%; height: 44px; padding: 0 15px; border: 0; background: var(--jn-well);
  display: flex; align-items: center; gap: 8px; cursor: pointer; text-align: left; color: var(--jn-text);
  font-family: inherit;
}
.list-blocked-head .chev { font-size: 11px; color: var(--jn-text-3); transition: transform 160ms ease; }
.list-blocked.open .list-blocked-head .chev { transform: rotate(90deg); }
.list-blocked-body { display: none; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; padding: 12px; }
.stage-card.is-list.list-has-inspect .list-blocked-body { grid-template-columns: repeat(2, minmax(0, 1fr)); }
.list-blocked.open .list-blocked-body { display: grid; }
.blocked-mini-card {
  display: flex; align-items: center; gap: 10px; padding: 9px 11px; border-radius: 13px;
  border: 1px solid var(--jn-line); background: var(--jn-surface);
}
.blocked-mini-card img, .blocked-fallback { width: 30px; height: 30px; border-radius: 8px; object-fit: contain; flex-shrink: 0; }
.blocked-fallback { display: grid; place-items: center; background: var(--jn-well); border: 1px solid var(--jn-line); font-size: 11px; font-weight: 700; color: var(--jn-text-2); }

.list-empty {
  border-radius: 20px; border: 1px dashed var(--jn-line); background: var(--jn-surface);
  padding: 36px 20px; text-align: center; color: var(--jn-text-3); font-size: 12.5px;
  display: flex; flex-direction: column; align-items: center; gap: 10px;
}
.list-security-tip {
  display: flex; align-items: center; gap: 6px; padding: 2px 4px; font-size: 11px; color: var(--jn-text-3);
}

.switch-pill { margin-left: auto; width: 40px; height: 22px; border-radius: 999px; border: 0; padding: 2.5px; background: var(--jn-track); cursor: pointer; display: flex; align-items: center; flex-shrink: 0; transition: background-color 180ms ease; }
.switch-pill.on { background: var(--jn-primary); }
.switch-pill:disabled { opacity: 0.45; cursor: not-allowed; }
.switch-knob { width: 17px; height: 17px; border-radius: 50%; background: #fff; box-shadow: 0 1px 3px rgba(0,0,0,0.2); transform: translateX(0); transition: transform 220ms var(--motion-ease-spring); }
.switch-pill.on .switch-knob { transform: translateX(18px); }

/* Toast & Modal */
.toast-host { position: fixed; top: 24px; right: 24px; z-index: 1050; display: flex; flex-direction: column; gap: 8px; pointer-events: none; }
.toast { pointer-events: auto; min-width: 300px; max-width: 450px; padding: 11px 14px; border-radius: 16px; background: var(--jn-surface); border: 1px solid var(--jn-line); box-shadow: 0 14px 34px rgba(0,0,0,0.16); display: flex; align-items: center; gap: 10px; animation: toastIn 220ms var(--motion-ease-spring) both; }
@keyframes toastIn { from{ opacity:0; transform: translateY(-8px) scale(.96);} to{ opacity:1; transform:none;} }
.toast-dot { width: 9px; height: 9px; border-radius: 50%; background: var(--jn-primary); flex-shrink: 0; }
.toast-dot.warn { background: var(--jn-warning); }
.toast-msg { flex: 1; min-width: 0; font-size: 12px; }
.toast-msg b { display: block; color: var(--jn-text); font-weight: 700; }
.toast-msg span { color: var(--jn-text-3); font-size: 11px; }
.toast-link { border: 0; border-radius: 8px; background: var(--jn-primary-soft); color: var(--jn-primary-ink); font-size: 11px; font-weight: 600; padding: 4px 8px; cursor: pointer; }
.modal-backdrop { position: fixed; inset: 0; z-index: 1200; background: rgba(0,0,0,0.42); backdrop-filter: blur(3px); display: none; place-items: center; padding: 20px; }
.modal-backdrop.open { display: grid; }
.modal-panel { width: min(560px, 100%); border-radius: 24px; background: var(--jn-surface); border: 1px solid var(--jn-line); box-shadow: 0 24px 64px rgba(0,0,0,0.28); padding: 20px 22px; display: flex; flex-direction: column; gap: 14px; }
.diff-box { border-radius: 16px; border: 1px solid var(--jn-line); overflow: hidden; }
.diff-head { height: 38px; padding: 0 12px; background: var(--jn-well); border-bottom: 1px solid var(--jn-line); display: flex; align-items: center; gap: 8px; font-size: 12px; }
.diff-lines { padding: 6px 0; font-size: 11.5px; }
.diff-line { display: flex; gap: 10px; padding: 3px 12px; font-family: "JetBrains Mono", monospace; }
.diff-line.add { background: var(--jn-primary-wash); color: var(--jn-primary-ink); }
.diff-line.del { background: var(--jn-danger-soft); color: var(--jn-danger); }
.diff-line.ctx { color: var(--jn-text-3); }
</style>
</head>
<body>
<div class="demo-backdrop">
  <div class="window-shell">
    <aside class="sidebar" id="sidebar">
      <div class="sidebar-logo">
        <img src="${appIcon}" alt="Jolu" />
        <svg class="sidebar-wordmark" width="50" height="20" viewBox="0 0 67 27" fill="none" stroke-linecap="round" stroke-linejoin="round">
          <g stroke="var(--jn-text)" stroke-width="2.6"><path d="M4 4h11v12c0 9-12 9-12 1" /><path d="M44 3l-4 15q-1 6 5 4" /><path d="M51 11l-2 7q-1 6 5 5 5-1 7-12m0 0l-2 10q0 3 4 1" /></g>
          <path d="M29 10c-9 0-11 13-3 13 9 0 12-13 3-13z" stroke="var(--jn-primary)" stroke-width="2.6" />
        </svg>
        <span class="spacer"></span>
        <button class="icon-btn" id="btn-collapse" title="收起/展开侧栏" aria-label="收起或展开侧栏">
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><rect x="3" y="3" width="18" height="18" rx="3"/><line x1="9" y1="3" x2="9" y2="21"/></svg>
        </button>
      </div>
      <nav class="nav-list">
        <button class="nav-item" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="3" y="3" width="18" height="14" rx="2"/><rect x="12" y="10" width="7" height="5" rx="1"/></svg><span class="nav-label">灵动岛</span></button>
        <button class="nav-item" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg><span class="nav-label">供应商</span></button>
        <button class="nav-item" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M3 3v18h18"/><path d="M18 17V9"/><path d="M13 17V5"/><path d="M8 17v-3"/></svg><span class="nav-label">用量统计</span></button>
        <button class="nav-item" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg><span class="nav-label">会话线程</span></button>
        <button class="nav-item" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="12" cy="12" r="9"/><path d="m9 12 2 2 4-4"/></svg><span class="nav-label">自动化</span></button>
        <button class="nav-item" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20"/></svg><span class="nav-label">Skills 管理</span></button>
        <button class="nav-item active" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="3" y="3" width="18" height="18" rx="3"/><path d="m7 15 4-4-4-4"/><path d="M13 15h4"/></svg><span class="nav-label">Agent</span></button>
        <button class="nav-item" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="6" cy="19" r="3"/><path d="M9 19h8.5a3.5 3.5 0 0 0 0-7h-11a3.5 3.5 0 0 1 0-7H15"/><circle cx="18" cy="5" r="3"/></svg><span class="nav-label">模型路由</span></button>
      </nav>
      <div class="sidebar-foot">
        <div class="island-mini">
          <span class="pulse-dot"></span>
          <div class="sidebar-island-meta" style="min-width:0;flex:1;">
            <div style="font-size:11.5px;font-weight:700;">灵动岛已开启</div>
            <div class="mono" style="font-size:10px;color:var(--jn-text-3);">127.0.0.1:4141</div>
          </div>
        </div>
        <button class="nav-item" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.6.77 1 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg><span class="nav-label">设置</span></button>
      </div>
    </aside>

    <div class="main-col">
      <header class="page-header">
        <h1 class="page-title">Agent</h1>
        <span class="demo-badge">✦ 3D WebGL2 深空星系 + 列表双视图</span>
        <span class="spacer"></span>
        <button class="btn" id="btn-reset" type="button">↺ 重置星系</button>
        <button class="btn" id="btn-theme" type="button">🌙 深色天文台</button>
        <button class="btn" id="btn-detect" type="button">⟳ 重新检测</button>
        <button class="btn btn-primary" id="btn-add" type="button">＋ 添加工具</button>
      </header>

      <div class="stage-card" id="stage">
        <canvas id="galaxy-webgl"></canvas>
        <canvas id="galaxy-overlay"></canvas>
        <div id="galaxy-dom"></div>

        <div class="top-hud">
          <div class="seg-group" id="filter-group">
            <button class="seg-btn active" data-filter="all">全部 <span class="tnum" id="cnt-all">11</span></button>
            <button class="seg-btn" data-filter="wired">已接入 <span class="tnum" id="cnt-wired">6</span></button>
            <button class="seg-btn" data-filter="available"><span id="filter-avail-label">待接星坞</span> <span class="tnum" id="cnt-avail">5</span></button>
          </div>
          <button class="alert-pill" id="btn-drift-jump" type="button">
            <span class="alert-dot"></span>
            <span id="drift-pill-text">Gemini CLI 配置漂移</span>
          </button>
          <div class="search-box">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
            <input id="search-input" type="text" placeholder="定位并聚焦 ⌘K" />
          </div>
          <span class="spacer"></span>
          <button class="mini-pill" id="btn-shuffle" style="height:34px;" type="button" title="按自然螺旋重新随机散落行星轨道">🎲 重排星位</button>
          <div class="seg-group" id="mode-group">
            <button class="seg-btn active" data-mode="3d">◈ 3D 星系</button>
            <button class="seg-btn" data-mode="list">☰ 列表</button>
          </div>
        </div>

        <section class="hangar-card" id="hangar-card" aria-label="待接入星坞">
          <header class="hangar-head" id="hangar-toggle">
            <span style="font-size:14px;">💎</span>
            <span class="hangar-title">待接入星坞</span>
            <span class="hangar-count tnum" id="hangar-count">5</span>
            <span class="hangar-hint" id="hangar-hint">拖小晶核入轨 ▾</span>
          </header>
          <div class="hangar-body" id="hangar-body"></div>
        </section>

        <aside class="right-hud" id="right-hud">
          <div class="inspect-card" id="inspect-card"></div>
        </aside>

        <!-- 右下 3D 交互导航舱 -->
        <div class="bottom-right-hud">
          <div class="nav-pod" id="cam-pod" title="3D 相机视角控制">
            <button class="pod-btn" data-cam="top" type="button">俯瞰</button>
            <button class="pod-btn active" data-cam="oblique" type="button">斜侧</button>
            <button class="pod-btn" data-cam="core" type="button">银心</button>
            <button class="pod-btn" id="btn-spin-toggle" type="button" title="暂停/恢复星系缓缓公转">⏸</button>
            <button class="pod-btn" id="btn-camera-reset" type="button" title="重置聚焦与 3D 视角">⟲</button>
          </div>
          <button class="mini-pill" id="btn-blocked" type="button" title="Cline / Continue / VS Code 暂不支持自动接入">🛡 不可接 3 ›</button>
        </div>

        <div class="stage-foot-hint" id="stage-hint">
          360° 自由拨转晶核球体 · 360° 无死角环绕星云 · 滚轮缩放 · 点击星球拉近展开设置
        </div>

        <div class="list-view" id="list-view"></div>
      </div>
    </div>
  </div>
</div>

<div class="drag-orb" id="drag-orb"><span class="drag-orb-ring"></span><img id="drag-orb-img" src="" alt="" /></div>
<div class="toast-host" id="toast-host"></div>

<div class="modal-backdrop" id="diff-modal">
  <div class="modal-panel" role="dialog" aria-modal="true">
    <div style="display:flex;align-items:center;gap:10px;">
      <img id="diff-logo" src="" alt="" style="width:28px;height:28px;object-fit:contain;" />
      <div style="flex:1;min-width:0;">
        <div id="diff-title" style="font-size:15px;font-weight:700;">写入改动预览</div>
        <div style="font-size:11.5px;color:var(--jn-text-3);">密钥已遮住 · 断开时只还原 Jolu 写入的键</div>
      </div>
      <button class="icon-btn" id="diff-close" type="button" aria-label="关闭">✕</button>
    </div>
    <div class="diff-box">
      <div class="diff-head"><span class="mono" id="diff-path" style="font-weight:600;"></span><span class="spacer"></span><span style="font-size:11px;color:var(--jn-primary);font-weight:600;" id="diff-stats"></span></div>
      <div class="diff-lines" id="diff-lines"></div>
    </div>
    <div style="display:flex;justify-content:flex-end;"><button class="btn btn-primary" id="diff-ok" type="button">知道了</button></div>
  </div>
</div>

<script>
(() => {
  ${GALAXY_WEBGL_SOURCE}
  ${SPHERE_ATLAS_WEBGL_SOURCE}
  const LOGOS = ${JSON.stringify(LOGOS)};
  const PURE_GLYPHS_LIGHT = ${JSON.stringify(PURE_GLYPHS_LIGHT)};
  const PURE_GLYPHS_DARK = ${JSON.stringify(PURE_GLYPHS_DARK)};
  const GLYPH_FALLBACK_COLORS = ${JSON.stringify(GLYPH_FALLBACK_COLORS)};
  const GLYPH_SLOT_ORDER = ["core", "claude-code", "codex", "opencode", "gemini", "qoder", "dsh", "kimi", "workbuddy", "pi", "zcode", "qwen"];
  const GLYPH_SLOT_INDEX = Object.fromEntries(GLYPH_SLOT_ORDER.map((id, i) => [id, i]));
  const BASE_ORBIT_R = 235;
  const FOV = 780;

  const INITIAL_AGENTS = [
    {
      id: "claude-code", name: "Claude Code", group: "wired", supported: true,
      model: "sonnet-4-5", status: "已接入 · 配置一致", tone: "normal",
      routesToday: 142, ring: true, moons: ["Opus", "Sonnet", "Haiku", "Fable"],
      color: "#D97757", glow: "rgba(217, 119, 87, 0.45)", tilt: 14,
      style: 1, spinRate: 0.22, configMode: "keep",
      modelOptions: ["sonnet-4-5", "claude-opus-4-1", "haiku-4-5", "fable-preview"],
      configPath: "~/.claude/settings.json", version: "v1.0.34",
      subtitle: "6 档位 · /model 12 个模型",
      diff: [
        { op: "ctx", text: '"env": {' },
        { op: "add", text: '  "ANTHROPIC_BASE_URL": "http://127.0.0.1:4141",' },
        { op: "add", text: '  "ANTHROPIC_AUTH_TOKEN": "jolu-agent-••••9f2c",' },
        { op: "add", text: '  "ANTHROPIC_MODEL": "sonnet-4-5"' },
        { op: "ctx", text: "}" }
      ]
    },
    {
      id: "codex", name: "Codex", group: "wired", supported: true,
      model: "重开生效", status: "重开后生效", tone: "warn",
      routesToday: 38, ring: false, moons: [],
      color: "#6E78FF", glow: "rgba(110, 120, 255, 0.46)", tilt: -16,
      style: 3, spinRate: 0.18, configMode: "keep",
      modelOptions: ["gpt-5-codex", "o4-mini", "重开生效"],
      configPath: "~/.codex/config.toml", version: "v0.169.0",
      subtitle: "保留 ChatGPT 登录",
      diff: [
        { op: "del", text: '- model_provider = "openai"' },
        { op: "add", text: '+ model_provider = "jolu"' },
        { op: "add", text: '+ base_url = "http://127.0.0.1:4141/v1"' }
      ]
    },
    {
      id: "opencode", name: "OpenCode", group: "wired", supported: true,
      model: "配置一致", status: "已接入 · 配置一致", tone: "normal",
      routesToday: 64, ring: true, moons: [],
      color: "#10B981", glow: "rgba(16, 185, 129, 0.46)", tilt: 12,
      style: 2, spinRate: 0.20, configMode: "keep",
      modelOptions: ["配置一致", "claude-sonnet-4-5", "gpt-5"],
      configPath: "~/.config/opencode/opencode.json", version: "v0.9.12",
      subtitle: "保留原 provider",
      diff: [
        { op: "add", text: '+ "provider": { "jolu": { "baseURL": "http://127.0.0.1:4141/v1" } }' },
        { op: "ctx", text: '  "autoupdate": true' }
      ]
    },
    {
      id: "gemini", name: "Gemini CLI", group: "wired", supported: true,
      model: "配置漂移!", status: "配置被改动，未经路由", tone: "danger",
      routesToday: 9, ring: false, moons: [],
      color: "#5B7CFA", glow: "rgba(224, 72, 77, 0.56)", tilt: -22,
      style: 4, spinRate: 0.24, configMode: "managed",
      modelOptions: ["配置漂移!", "gemini-2.5-pro", "gemini-2.5-flash"],
      configPath: "~/.gemini/.env", version: "v0.4.8",
      subtitle: "地址已被其它程序覆盖",
      diff: [
        { op: "del", text: '- GEMINI_API_BASE="https://external-proxy.invalid"' },
        { op: "add", text: '+ GEMINI_API_BASE="http://127.0.0.1:4141/gemini"' },
        { op: "add", text: '+ GEMINI_API_KEY="jolu-agent-••••7c11"' }
      ]
    },
    {
      id: "qoder", name: "Qoder", group: "wired", supported: true,
      model: "glm-5-turbo", status: "已接入 · 配置一致", tone: "normal",
      routesToday: 27, ring: true, moons: [],
      color: "#18C964", glow: "rgba(42, 219, 92, 0.48)", tilt: 18,
      style: 1, spinRate: 0.23, configMode: "keep",
      modelOptions: ["glm-5-turbo", "qwen3-max", "deepseek-v3"],
      configPath: "~/.qoder/settings.json", version: "v1.4.2",
      subtitle: "Qoder 路由已绑定",
      diff: [
        { op: "add", text: '+ "selectedModel": "jolu/glm-5-turbo"' },
        { op: "add", text: '+ "providers.jolu.endpoint": "http://127.0.0.1:4141/v1"' }
      ]
    },
    {
      id: "dsh", name: "DeepSeek", group: "wired", supported: true,
      model: "deepseek-r1", status: "已接入 · 配置一致", tone: "normal",
      routesToday: 51, ring: false, moons: [],
      color: "#4D6BFE", glow: "rgba(77, 107, 254, 0.48)", tilt: -14,
      style: 3, spinRate: 0.21, configMode: "keep",
      modelOptions: ["deepseek-r1", "deepseek-chat"],
      configPath: "~/.cordis/cordis.patch.yml", version: "v2.1.0",
      subtitle: "DeepSeek Harness",
      diff: [
        { op: "add", text: '+ active_provider: "jolu"' },
        { op: "add", text: '+ default_model: "jolu/deepseek-r1"' }
      ]
    },
    {
      id: "kimi", name: "Kimi CLI", group: "available", supported: true,
      model: "kimi-k2-thinking", status: "可接入", tone: "normal",
      routesToday: 0, ring: true, moons: [],
      color: "#1783FF", glow: "rgba(23, 131, 255, 0.48)", tilt: 16,
      style: 1, spinRate: 0.22, configMode: "keep",
      modelOptions: ["kimi-k2-thinking", "moonshot-v1-auto"],
      configPath: "~/.kimi/config.toml", version: "v0.58",
      subtitle: "CLI 未在 PATH（不影响接入）", cliMissing: true,
      diff: [
        { op: "ctx", text: '[providers.jolu]' },
        { op: "add", text: '+ base_url = "http://127.0.0.1:4141/v1"' },
        { op: "add", text: '+ api_key = "jolu-agent-••••2a81"' },
        { op: "del", text: '- default_model = "moonshot-v1"' },
        { op: "add", text: '+ default_model = "jolu/kimi-k2-thinking"' }
      ]
    },
    {
      id: "workbuddy", name: "WorkBuddy", group: "available", supported: true,
      model: "gpt-4.1-mini", status: "可接入", tone: "normal",
      routesToday: 0, ring: false, moons: [],
      color: "#0EC8A9", glow: "rgba(14, 200, 169, 0.46)", tilt: -12,
      style: 2, spinRate: 0.19, configMode: "keep",
      modelOptions: ["gpt-4.1-mini", "jolu/auto"],
      configPath: "~/.workbuddy/models.json", version: "v1.1.0",
      subtitle: "用它自己的模型",
      diff: [
        { op: "add", text: '+ { "id": "jolu/auto", "baseUrl": "http://127.0.0.1:4141/v1" }' },
        { op: "ctx", text: '  // 原有模型与开关原样保留' }
      ]
    },
    {
      id: "pi", name: "Pi", group: "available", supported: true,
      model: "claude-sonnet-4-5", status: "可接入", tone: "normal",
      routesToday: 0, ring: false, moons: [],
      color: "#E69148", glow: "rgba(241, 190, 88, 0.46)", tilt: 19,
      style: 3, spinRate: 0.21, configMode: "keep",
      modelOptions: ["claude-sonnet-4-5", "gpt-5-mini"],
      configPath: "~/.pi/agent/models.json", version: "v0.19",
      subtitle: "用它自己的 provider",
      diff: [
        { op: "add", text: '+ "providers.jolu.baseUrl": "http://127.0.0.1:4141/v1"' },
        { op: "add", text: '+ "defaultProvider": "jolu"' }
      ]
    },
    {
      id: "zcode", name: "ZCode", group: "available", supported: true,
      model: "glm-5", status: "可接入", tone: "normal",
      routesToday: 0, ring: true, moons: [],
      color: "#06B6D4", glow: "rgba(6, 182, 212, 0.46)", tilt: -18,
      style: 1, spinRate: 0.23, configMode: "keep",
      modelOptions: ["glm-5", "glm-4.5-air"],
      configPath: "~/.zcode/settings.json", version: "v2.0.1",
      subtitle: "用 ZCode 自己的账号",
      diff: [
        { op: "add", text: '+ "joluGateway": "http://127.0.0.1:4141"' },
        { op: "add", text: '+ "defaultModel": "jolu/glm-5"' }
      ]
    },
    {
      id: "qwen", name: "Qwen", group: "available", supported: true,
      model: "qwen3-coder-plus", status: "可接入", tone: "normal",
      routesToday: 0, ring: true, moons: [],
      color: "#2D4EFF", glow: "rgba(45, 78, 255, 0.48)", tilt: 14,
      style: 2, spinRate: 0.20, configMode: "keep",
      modelOptions: ["qwen3-coder-plus", "qwen-max"],
      configPath: "~/.qwen/settings.json", version: "v0.7.3",
      subtitle: "用它自己的登录",
      diff: [
        { op: "add", text: '+ "modelProviders.openai.jolu": "http://127.0.0.1:4141/v1"' },
        { op: "add", text: '+ "selectedModel": "jolu/qwen3-coder-plus"' }
      ]
    },
    {
      id: "antigravity", name: "Antigravity CLI", group: "available", supported: false,
      model: "", status: "暂不支持专用接入", tone: "normal",
      routesToday: 0, ring: false, moons: [],
      color: "#6366F1", glow: "rgba(99,102,241,.35)", tilt: 0,
      style: 3, spinRate: 0.3, configMode: "keep", modelOptions: [],
      configPath: "~/.gemini/antigravity-cli/settings.json", version: "v0.3",
      subtitle: "按「添加工具」走通用接入", diff: []
    }
  ];

  const BLOCKED_AGENTS = [
    { id: "cline", name: "Cline", reason: "VS Code 扩展目录未找到" },
    { id: "continue", name: "Continue", reason: "本机未检测到配置" },
    { id: "vscode", name: "VS Code", reason: "需使用扩展内配置" },
  ];

  let agents = [];
  let dark = false;
  let mode = "3d"; // 只有 3D 星系与列表两态
  let autoSpin = true;
  let chaosSeed = 29;
  let filter = "all";
  let searchQuery = "";
  let selectedId = null;
  let focusId = null;
  let hoveredId = null;
  let hangarCollapsed = false;

  const coreBody = {
    selfSpin: -0.32, spinVel: 0, pitchSpin: 0, pitchVel: 0,
    spinMul: 0.0, tilt: 0.14, seed: 1.7,
    galaxySpeed: 1.0, preset: "日常编码 · 6 档位",
  };

  function resetAgentsState() {
    const startOffsets = [-0.14, 0.12, -0.10, 0.15, -0.12, 0.10, -0.13, 0.14, -0.11, 0.13, -0.09, 0.12];
    agents = structuredClone(INITIAL_AGENTS).map((a, idx) => ({
      ...a,
      selfSpin: -0.32 + startOffsets[idx % startOffsets.length],
      spinVel: 0,
      pitchSpin: 0,
      pitchVel: 0,
      spinMul: 0.0,
      activeTier: 1,
      seed: 1.2 + idx * 1.47,
    }));
  }
  resetAgentsState();

  const cam = {
    yaw: -0.32, pitch: 0.44, zoom: 1.0,
    targetYaw: -0.32, targetPitch: 0.44, targetZoom: 1.0,
    vYaw: 0, vPitch: 0,
    fx: 0, fy: 0, fz: 0,
    targetFx: 0, targetFy: 0, targetFz: 0,
    screenBiasX: 0, targetScreenBiasX: 0,
    parallaxX: 0, parallaxY: 0,
    dragMode: null, // null | "orbit" | "pan"
    moved: false, lastX: 0, lastY: 0,
    orbitSpin: 0,
    preset: "oblique",
  };

  const planetAnim = new Map();
  const shockwaves = [];
  const particles = [];
  let dragState = null;
  let scrubState = null;

  const $ = (s) => document.querySelector(s);
  const stage = $("#stage");
  const glCanvas = $("#galaxy-webgl");
  const overlayCanvas = $("#galaxy-overlay");
  const ctx = overlayCanvas.getContext("2d");
  const domLayer = $("#galaxy-dom");
  const rightHud = $("#right-hud");
  const hangarCard = $("#hangar-card");
  const hangarBody = $("#hangar-body");
  const inspectCard = $("#inspect-card");
  const dragOrb = $("#drag-orb");
  const dragOrbImg = $("#drag-orb-img");
  const listView = $("#list-view");

  const glRenderer = createGalaxyWebGLRenderer(glCanvas);
  const sphereAtlas = createSphereAtlasRenderer();

  function mulberry32(seed) {
    let a = seed >>> 0;
    return () => {
      a |= 0; a = (a + 0x6D2B79F5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function logoFor(id) {
    const entry = LOGOS[id];
    if (!entry) return LOGOS.appIcon;
    return dark ? entry.dark : entry.light;
  }
  function wiredAgents() { return agents.filter((a) => a.group === "wired"); }
  function dockAgents() { return agents.filter((a) => a.group === "available" && a.supported); }
  function unsupportedAgents() { return agents.filter((a) => a.group === "available" && !a.supported); }

  function stageFit(w, h) {
    return Math.max(0.68, Math.min(w / 1060, h / 720));
  }

  // 与 WebGL 着色器完全一致的三维世界点投影
  function project3D(wx, wy, wz, followFocus = 1, w = stage.clientWidth, h = stage.clientHeight, zoomOverride = null) {
    const cx = w * 0.50 + cam.screenBiasX * followFocus;
    const cy = h * 0.50;
    const effZoom = (zoomOverride ?? cam.zoom) * stageFit(w, h);
    const yaw = cam.yaw + cam.parallaxX;
    const pitch = cam.pitch + cam.parallaxY;
    const cyaw = Math.cos(yaw), syaw = Math.sin(yaw);
    const cp = Math.cos(pitch), sp = Math.sin(pitch);

    const dx = wx - cam.fx * followFocus;
    const dy = wy - cam.fy * followFocus;
    const dz = wz - cam.fz * followFocus;

    const x1 = dx * cyaw - dz * syaw;
    const z1 = dx * syaw + dz * cyaw;
    const y2 = dy * cp - z1 * sp;
    const z2 = dy * sp + z1 * cp;

    const denom = Math.max(95, FOV + z2);
    const scale = (FOV / denom) * effZoom;
    return {
      x: cx + x1 * scale,
      y: cy + y2 * scale,
      z: z2,
      scale,
      behind: FOV + z2 < 95,
    };
  }

  // 把世界差向量旋进视角光向 (+X右, +Y上, +Z朝向观察者)，驱动真三维球面受光与晨昏线
  function toViewLightDir(dx, dy, dz) {
    const yaw = cam.yaw + cam.parallaxX;
    const pitch = cam.pitch + cam.parallaxY;
    const cy = Math.cos(yaw), sy = Math.sin(yaw);
    const cp = Math.cos(pitch), sp = Math.sin(pitch);
    const x1 = dx * cy - dz * sy;
    const z1 = dx * sy + dz * cy;
    const y2 = dy * cp - z1 * sp;
    const z2 = dy * sp + z1 * cp;
    const vx = x1;
    const vy = -y2;
    const vz = -z2;
    const len = Math.max(1e-4, Math.hypot(vx, vy, vz));
    return [vx / len, vy / len, vz / len];
  }

  // 根据水平倾角+升交点+开普勒偏心椭圆算出世界坐标 (wx, wy, wz)
  function orbitWorldPos(spec, angle) {
    const r = spec.radius;
    const ecc = spec.ecc ?? 0;
    const peri = spec.peri ?? 0;
    const semiMinor = r * Math.sqrt(Math.max(0.42, 1 - ecc * ecc));
    const focal = r * ecc * 0.40;
    const th = angle - peri;
    const lpX = Math.cos(th) * r - focal;
    const lpZ = Math.sin(th) * semiMinor;
    const cp0 = Math.cos(peri), sp0 = Math.sin(peri);
    const ox = lpX * cp0 - lpZ * sp0;
    const oz = lpX * sp0 + lpZ * cp0;

    const asc = ((spec.ascDeg ?? 0) * Math.PI) / 180;
    const ca = Math.cos(asc), sa = Math.sin(asc);
    const nx = ox * ca + oz * sa;
    const nz = -ox * sa + oz * ca;
    const tilt = ((spec.tiltDeg ?? 0) * Math.PI) / 180;
    const ty = nz * Math.sin(tilt) + (spec.elevY ?? 0);
    const tz = nz * Math.cos(tilt);
    return {
      x: nx * ca - tz * sa,
      y: ty,
      z: nx * sa + tz * ca,
    };
  }

  // 把鼠标指针射线反投影到水平平面 y = planeY，实现 3D 所见即所落（拖动时滚轮还能上下改高度）
  function screenToWorldPlane(sx, sy, planeY = 0, w = stage.clientWidth, h = stage.clientHeight) {
    const cx = w * 0.50 + cam.screenBiasX, cy = h * 0.50;
    const effZoom = cam.zoom * stageFit(w, h);
    const yaw = cam.yaw + cam.parallaxX;
    const pitch = cam.pitch + cam.parallaxY;
    const cyaw = Math.cos(yaw), syaw = Math.sin(yaw);
    const cp = Math.cos(pitch), sp = Math.sin(pitch);

    const dxScr = (sx - cx) / Math.max(0.25, effZoom);
    const dyScr = (sy - cy) / Math.max(0.25, effZoom);
    const dyRel = planeY - cam.fy;

    // 由 y2 = dyRel*cp - z1*sp 与 dyScr = y2 * FOV / (FOV + dyRel*sp + z1*cp) 解 z1
    const numer = dyRel * cp * FOV - dyScr * (FOV + dyRel * sp);
    const denom = dyScr * cp + sp * FOV;
    let z1 = Math.abs(denom) < 40 ? -dyScr : numer / denom;
    z1 = Math.max(-360, Math.min(360, z1));
    const z2 = dyRel * sp + z1 * cp;
    const x1 = dxScr * ((FOV + z2) / FOV);

    return {
      x: cam.fx + (x1 * cyaw + z1 * syaw),
      y: planeY,
      z: cam.fz + (-x1 * syaw + z1 * cyaw),
    };
  }

  // 从自由空间坐标生成穿过该点的新偏心立体轨道
  function makeCustomOrbitAtWorld(wx, wy, wz, salt = 1) {
    const rng = mulberry32((chaosSeed * 131 + salt * 977) >>> 0);
    const horiz = Math.hypot(wx, wz);
    const angle = Math.atan2(wz, wx);
    const ecc = 0.10 + rng() * 0.16;
    const clampedH = Math.max(92, Math.min(325, horiz));
    const radius = clampedH / (1 - 0.40 * ecc);
    const speedMul = Math.max(0.45, Math.min(1.8, (BASE_ORBIT_R / Math.max(95, radius)) ** 1.18));
    return {
      radius,
      angle,
      ecc,
      peri: angle,
      tiltDeg: (rng() - 0.5) * 34,
      ascDeg: rng() * 360,
      elevY: wy,
      speedMul,
    };
  }

  // 为每颗行星生成立体不对称自然轨道，同时确保初始 2D 视窗里标签互不遮挡、避开星坞/右卡
  function ensureChaosOrbits(forceReseed = false) {
    const w = stage.clientWidth || 1080;
    const h = stage.clientHeight || 760;
    const placed = [];
    const sorted = [...agents].sort((a, b) => (a.group === "wired" ? 0 : 1) - (b.group === "wired" ? 0 : 1));

    sorted.forEach((a, idx) => {
      if (!forceReseed && a.chaosOrbit && a.chaosSeed === chaosSeed) {
        if (a.group === "wired") {
          const wp = orbitWorldPos(a.chaosOrbit, a.chaosOrbit.angle + cam.orbitSpin * a.chaosOrbit.speedMul);
          const p = project3D(wp.x, wp.y, wp.z, 0, w, h, 1.0);
          placed.push({ x: p.x, y: p.y });
        }
        return;
      }
      const idHash = a.id.split("").reduce((s, ch) => s * 31 + ch.charCodeAt(0), 17);
      const rng = mulberry32((chaosSeed * 997 + idHash + idx * 71) >>> 0);
      let best = null;
      for (let attempt = 0; attempt < 180; attempt++) {
        const ecc = 0.08 + rng() * 0.20;
        const peri = rng() * Math.PI * 2;
        const radius = 142 + rng() * 192;
        const angle = rng() * Math.PI * 2;
        const tiltDeg = (rng() - 0.5) * 44;
        const ascDeg = rng() * 360;
        const elevY = (rng() - 0.5) * 44;
        const cand = {
          radius, angle, ecc, peri, tiltDeg, ascDeg, elevY,
          speedMul: Math.max(0.42, Math.min(1.85, (BASE_ORBIT_R / radius) ** 1.18)),
        };
        const wp = orbitWorldPos(cand, cand.angle + cam.orbitSpin * cand.speedMul);
        const pt = project3D(wp.x, wp.y, wp.z, 0, w, h, 1.0);
        const corePt = project3D(0, 0, 0, 0, w, h, 1.0);

        const minSep = placed.length ? Math.min(...placed.map((p) => Math.hypot(pt.x - p.x, pt.y - p.y))) : 999;
        const inBounds = pt.x >= 84 && pt.x <= w - 84 && pt.y >= 90 && pt.y <= h - 78;
        const notCore = Math.hypot(pt.x - corePt.x, pt.y - corePt.y) >= 158 && cand.radius * (1 - 0.4 * ecc) >= 134;
        const notHangar = !(pt.x < 394 && pt.y > h - 432);
        const notBotRight = !(pt.x > w - 340 && pt.y > h - 92);
        const cleanHud = inBounds && notCore && notHangar && notBotRight;
        if (!best || (cleanHud && !best.cleanHud) || (cleanHud === best.cleanHud && minSep > best.minSep)) {
          best = { cand, pt, minSep, cleanHud };
        }
        if (cleanHud && placed.every((p) => !(Math.abs(pt.x - p.x) < 140 && Math.abs(pt.y - p.y) < 88))) {
          best = { cand, pt, minSep, cleanHud: true };
          break;
        }
      }
      a.chaosOrbit = best.cand;
      a.chaosSeed = chaosSeed;
      if (a.group === "wired") placed.push({ x: best.pt.x, y: best.pt.y });
    });
  }

  function getAgentWorldPos(a) {
    const co = a.chaosOrbit;
    if (!co) return { x: 0, y: 0, z: 0 };
    return orbitWorldPos(co, co.angle + cam.orbitSpin * co.speedMul);
  }

  function nextCaptureSpec(sx = null, sy = null) {
    const w = stage.clientWidth, h = stage.clientHeight;
    const aObj = agents.find((x) => x.id === dragState?.agentId);
    if (sx != null && sy != null && dragState?.captured) {
      const wp = screenToWorldPlane(sx, sy, dragState.altY ?? 0, w, h);
      const spec = makeCustomOrbitAtWorld(wp.x, wp.y, wp.z, (aObj?.name.length ?? 4) + 13);
      return { ...spec, custom: true, dropWorld: wp };
    }
    const fallback = aObj?.chaosOrbit ?? makeCustomOrbitAtWorld(170, 12, -105, 7);
    return { ...fallback, custom: false, dropWorld: orbitWorldPos(fallback, fallback.angle) };
  }

  let lastPointerFocusTs = 0;

  function startSphereScrub(e, el, targetId) {
    if (e.button !== 0 || mode !== "3d") return;
    e.stopPropagation();
    scrubState = {
      el,
      targetId,
      startX: e.clientX,
      startY: e.clientY,
      lastX: e.clientX,
      lastY: e.clientY,
      moved: false,
    };
  }

  function bindSphereClick(el, targetId) {
    el.addEventListener("pointerdown", (e) => startSphereScrub(e, el, targetId));
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      if (performance.now() - lastPointerFocusTs < 85) return;
      focusAgent(targetId);
    });
  }

  function syncDomNodes() {
    const w = stage.clientWidth, h = stage.clientHeight;
    if (mode === "list") return;
    ensureChaosOrbits(false);
    const wired = wiredAgents();

    // 1. 中心 Jolu 纯净翡翠恒星核（100% 无遮挡旋转光球 + 极细日冕环 + 下方带头像的品牌微胶囊）
    let coreEl = domLayer.querySelector(".jolu-core");
    if (!coreEl) {
      coreEl = document.createElement("div");
      coreEl.className = "jolu-core";
      coreEl.innerHTML = \`
        <span class="core-focus-ring"></span>
        <canvas class="core-globe" width="176" height="176"></canvas>
        <svg class="core-gyros" viewBox="-85 -85 170 170" aria-hidden="true">
          <g transform="rotate(-22)">
            <g class="core-gyro-1"><ellipse cx="0" cy="0" rx="68" ry="18" fill="none" stroke="var(--jn-primary)" stroke-width="0.95" stroke-dasharray="12 8" opacity="0.44" /></g>
          </g>
          <g transform="rotate(34)">
            <g class="core-gyro-2"><ellipse cx="0" cy="0" rx="74" ry="20" fill="none" stroke="currentColor" stroke-width="0.85" stroke-dasharray="5 9" opacity="0.30" /></g>
          </g>
        </svg>
        <div class="jolu-core-pill">
          <span class="ok"></span>
          <strong>Jolu</strong>
          <span class="jolu-core-sub tnum" id="core-count"></span>
          <span class="jolu-core-port mono">:4141</span>
        </div>
      \`;
      bindSphereClick(coreEl, "core");
      coreEl.addEventListener("mouseenter", () => { hoveredId = "core"; });
      coreEl.addEventListener("mouseleave", () => { if (hoveredId === "core") hoveredId = null; });
      domLayer.appendChild(coreEl);
    }
    const cp = project3D(0, 0, 0, 1, w, h);
    coreEl.style.display = cp.behind ? "none" : "grid";
    const cScale = Math.max(0.54, Math.min(1.85, cp.scale));
    coreEl.style.left = cp.x + "px";
    coreEl.style.top = cp.y + "px";
    coreEl.style.color = dark ? "#86EFAC" : "#16A862";
    coreEl.style.transform = \`translate(-50%, -50%) scale(\${cScale.toFixed(3)})\`;
    coreEl.style.zIndex = Math.round(500 - cp.z);
    coreEl.classList.toggle("selected", selectedId === "core");
    coreEl.classList.toggle("magnet-active", Boolean(dragState?.captured));
    const coreCountEl = coreEl.querySelector("#core-count");
    if (coreCountEl) coreCountEl.textContent = \`\${wired.length} 已接 · \${dockAgents().length} 待接\`;

    // 2. 捕获光环槽
    let slotEl = domLayer.querySelector(".capture-slot");
    if (!slotEl) {
      slotEl = document.createElement("div");
      slotEl.className = "capture-slot";
      slotEl.innerHTML = '<span class="capture-slot-tag" id="slot-tag">松手在此成轨</span>';
      domLayer.appendChild(slotEl);
    }
    if (dragState) {
      const spec = nextCaptureSpec(dragState.x, dragState.y);
      const wp = spec.dropWorld ?? orbitWorldPos(spec, spec.angle);
      const p = project3D(wp.x, wp.y, wp.z, 1, w, h);
      dragState.slotScreen = { x: p.x, y: p.y };
      dragState.slotSpec = spec;
      dragState.slotWorld = wp;
      slotEl.style.left = p.x + "px";
      slotEl.style.top = p.y + "px";
      slotEl.style.zIndex = Math.round(500 - p.z) + 5;
      slotEl.classList.add("visible");
      slotEl.classList.toggle("hot", Boolean(dragState.captured));
      const a = agents.find((x) => x.id === dragState.agentId);
      const altTxt = Math.abs(dragState.altY) > 2 ? \` · 高度 \${dragState.altY > 0 ? "+" : ""}\${Math.round(dragState.altY)}\` : "";
      slotEl.querySelector("#slot-tag").textContent = dragState.captured
        ? \`松手成轨 · \${a?.name ?? ""}\${altTxt}\`
        : \`预设空位 · \${a?.name ?? ""}\`;
    } else {
      slotEl.classList.remove("visible", "hot");
    }

    // 3. 已接入行星节点
    const activeIds = new Set(wired.map((a) => a.id));
    domLayer.querySelectorAll(".planet-node").forEach((el) => {
      if (!activeIds.has(el.dataset.id)) el.remove();
    });

    const visibleMatch = (a) => {
      if (filter === "available") return false;
      if (!searchQuery.trim()) return true;
      const q = searchQuery.trim().toLowerCase();
      return a.name.toLowerCase().includes(q) || a.id.includes(q) || a.model.toLowerCase().includes(q);
    };

    wired.forEach((a, idx) => {
      let node = domLayer.querySelector(\`.planet-node[data-id="\${a.id}"]\`);
      if (!node) {
        const ringTilt = (a.tilt * 0.65 - 16).toFixed(1);
        const ringHtml = (front) => !a.ring ? "" : \`
          <svg class="planet-ring-svg \${front ? "front" : "back"}" viewBox="-60 -60 120 120">
            <g transform="rotate(\${ringTilt})">
              <path d="\${front ? "M -50,0 A 50,13.5 0 0 0 50,0" : "M -50,0 A 50,13.5 0 0 1 50,0"}" fill="none" stroke="currentColor" stroke-width="2.1" opacity="\${front ? 0.68 : 0.30}" />
              <path d="\${front ? "M -43,0 A 43,10.8 0 0 0 43,0" : "M -43,0 A 43,10.8 0 0 1 43,0"}" fill="none" stroke="currentColor" stroke-width="0.95" opacity="\${front ? 0.38 : 0.16}" />
            </g>
          </svg>
        \`;
        node = document.createElement("div");
        node.className = "planet-node";
        node.dataset.id = a.id;
        node.innerHTML = \`
          <div class="planet-hit">
            \${ringHtml(false)}
            <div class="planet-sphere"><canvas class="planet-globe" width="176" height="176"></canvas></div>
            <div class="planet-hit-ring"></div>
            \${ringHtml(true)}
          </div>
          <div class="planet-label">
            <span class="label-inline-dot"></span>
            <strong></strong>
            <span class="label-model"></span>
          </div>
        \`;
        node.addEventListener("mouseenter", () => { hoveredId = a.id; });
        node.addEventListener("mouseleave", () => { if (hoveredId === a.id) hoveredId = null; });
        bindSphereClick(node, a.id);
        domLayer.appendChild(node);
      }

      const wp = getAgentWorldPos(a);
      const proj = project3D(wp.x, wp.y, wp.z, 1, w, h);
      if (proj.behind) { node.style.display = "none"; return; }
      node.style.display = "flex";
      node.classList.toggle("dimmed", !visibleMatch(a));
      node.classList.toggle("selected", selectedId === a.id);
      node.classList.toggle("is-drift", a.tone === "danger");
      node.classList.toggle("is-warn", a.tone === "warn");

      const lbl = node.querySelector(".planet-label");
      const toneCls = a.tone === "danger" ? " danger" : a.tone === "warn" ? " warn" : "";
      node.querySelector(".label-inline-dot").className = "label-inline-dot" + toneCls;
      lbl.querySelector("strong").textContent = a.name;
      lbl.querySelector(".label-model").textContent = a.model;

      let px = proj.x, py = proj.y, pScale = proj.scale;
      const anim = planetAnim.get(a.id);
      if (anim) { px += anim.dx; py += anim.dy; pScale *= anim.scale; }

      const massMul = a.id === "claude-code" ? 1.18 : (0.94 + Math.min(0.22, a.routesToday / 260));
      const size = Math.max(28, Math.min(156, (50 * massMul) * pScale));
      const hit = node.querySelector(".planet-hit");
      hit.style.width = size + "px";
      hit.style.height = size + "px";
      hit.style.color = dark ? "rgba(255,255,255,0.68)" : a.color;

      if (!sphereAtlas) {
        const sphereDiv = node.querySelector(".planet-sphere");
        sphereDiv.style.background = dark
          ? \`radial-gradient(circle at 35% 35%, rgba(255,255,255,0.28), \${a.color}55 55%, #09120E 100%)\`
          : \`radial-gradient(circle at 35% 35%, #ffffff, \${a.color}36 65%, \${a.color}88 100%)\`;
      }

      node.style.left = px + "px";
      node.style.top = py + "px";
      node.style.zIndex = Math.round(500 - proj.z);
      node.style.opacity = proj.z > 80 && selectedId !== a.id ? String(Math.max(0.62, 1 - (proj.z - 80) / 720)) : "";
    });
  }

  // 启动时将 12 枚去底板纯净矢量符号合成为「几何 SDF + 浅/深双套真彩品牌色图集」（外缘 RGB 形态学膨胀消黑边），上传至 WebGL2
  async function initGlyphAtlas() {
    if (!sphereAtlas) return;
    const CELL = sphereAtlas.cellSize;
    const makeEmptyAtlas = (fillBlack = false) => {
      const a = document.createElement("canvas");
      a.width = CELL * 4; a.height = CELL * 3;
      const c = a.getContext("2d");
      if (fillBlack) {
        c.fillStyle = "#000000";
        c.fillRect(0, 0, a.width, a.height);
      }
      return [a, c];
    };
    const [sdfAtlas, sactx] = makeEmptyAtlas(true);
    const [lightColorAtlas, lctx] = makeEmptyAtlas(false);
    const [darkColorAtlas, dctx] = makeEmptyAtlas(false);

    const loadImg = (url) => new Promise((res) => {
      const img = new Image();
      img.onload = () => res(img);
      img.onerror = () => res(null);
      img.src = url;
    });

    const makeChannelLayer = (srcCanvas, hexColor) => {
      const c = document.createElement("canvas");
      c.width = CELL; c.height = CELL;
      const cx = c.getContext("2d");
      cx.drawImage(srcCanvas, 0, 0);
      cx.globalCompositeOperation = "source-in";
      cx.fillStyle = hexColor;
      cx.fillRect(0, 0, CELL, CELL);
      return c;
    };

    const buildOpaqueColorCell = (img, fallbackHex) => {
      const cell = document.createElement("canvas");
      cell.width = CELL; cell.height = CELL;
      const cctx = cell.getContext("2d");
      const tmp = document.createElement("canvas");
      tmp.width = CELL; tmp.height = CELL;
      const tctx = tmp.getContext("2d");
      const pad = Math.round(CELL * 0.21);
      const drawSz = CELL - pad * 2;

      if (img) {
        cctx.drawImage(img, pad, pad, drawSz, drawSz);
        cctx.globalCompositeOperation = "destination-over";
        const dirs = [[-1, 0], [1, 0], [0, -1], [0, 1], [-2, 0], [2, 0], [0, -2], [0, 2]];
        for (let pass = 0; pass < 3; pass++) {
          tctx.clearRect(0, 0, CELL, CELL);
          tctx.drawImage(cell, 0, 0);
          for (const [ox, oy] of dirs) cctx.drawImage(tmp, ox, oy);
        }
      }
      cctx.globalCompositeOperation = "destination-over";
      cctx.fillStyle = fallbackHex || "#FFFFFF";
      cctx.fillRect(0, 0, CELL, CELL);
      cctx.globalCompositeOperation = "source-over";
      return cell;
    };

    await Promise.all(GLYPH_SLOT_ORDER.map(async (id, idx) => {
      const lUrl = PURE_GLYPHS_LIGHT[id];
      const dUrl = PURE_GLYPHS_DARK[id] || lUrl;
      if (!lUrl) return;
      const [lImg, dImg] = await Promise.all([loadImg(lUrl), dUrl === lUrl ? Promise.resolve(null) : loadImg(dUrl)]);
      const col = idx % 4;
      const row = Math.floor(idx / 4);

      if (lImg) {
        const sil = document.createElement("canvas");
        sil.width = CELL; sil.height = CELL;
        const sctx = sil.getContext("2d");
        const pad = Math.round(CELL * 0.21);
        const drawSz = CELL - pad * 2;
        sctx.drawImage(lImg, pad, pad, drawSz, drawSz);
        sctx.globalCompositeOperation = "source-in";
        sctx.fillStyle = "#ffffff";
        sctx.fillRect(0, 0, CELL, CELL);

        const redSharp = makeChannelLayer(sil, "#ff0000");
        const greenGlow = makeChannelLayer(sil, "#00ff00");
        const blueBevel = makeChannelLayer(sil, "#0000ff");

        const scell = document.createElement("canvas");
        scell.width = CELL; scell.height = CELL;
        const scc = scell.getContext("2d");
        scc.fillStyle = "#000000";
        scc.fillRect(0, 0, CELL, CELL);
        scc.globalCompositeOperation = "lighter";

        scc.filter = "blur(13px)";
        scc.drawImage(greenGlow, 0, 0);
        scc.drawImage(greenGlow, 0, 0);

        scc.filter = "blur(3.2px)";
        scc.drawImage(blueBevel, 0, 0);

        scc.filter = "none";
        scc.drawImage(redSharp, 0, 0);
        sactx.drawImage(scell, col * CELL, row * CELL);
      }

      lctx.drawImage(buildOpaqueColorCell(lImg, GLYPH_FALLBACK_COLORS.light[id]), col * CELL, row * CELL);
      dctx.drawImage(buildOpaqueColorCell(dImg || lImg, GLYPH_FALLBACK_COLORS.dark[id]), col * CELL, row * CELL);
    }));

    sphereAtlas.uploadGlyphAtlas(sdfAtlas, lightColorAtlas, darkColorAtlas);
  }
  initGlyphAtlas();

  // 单通道 WebGL2 360° 凝光折射晶核批量渲染
  function renderAndBlitSpheres(wired, timeSec) {
    if (!sphereAtlas) return;
    const atlasItems = [{
      glyphSlot: 0,
      isCore: true,
      spin: coreBody.selfSpin,
      pitchSpin: coreBody.pitchSpin ?? 0,
      tilt: coreBody.tilt,
      seed: coreBody.seed,
      color: "#14A860",
      toneNum: 0,
      focus: selectedId === "core",
      lx: 0.38, ly: 0.45, lz: 0.80,
    }];
    wired.forEach((a) => {
      const wp = getAgentWorldPos(a);
      const [lx, ly, lz] = toViewLightDir(-wp.x, -wp.y, -wp.z);
      atlasItems.push({
        glyphSlot: GLYPH_SLOT_INDEX[a.id] ?? 1,
        isCore: false,
        spin: a.selfSpin ?? 0,
        pitchSpin: a.pitchSpin ?? 0,
        tilt: ((a.tilt ?? 12) * Math.PI) / 180,
        seed: a.seed ?? 2.3,
        color: a.color,
        toneNum: a.tone === "danger" ? 2 : a.tone === "warn" ? 1 : 0,
        focus: selectedId === a.id,
        lx, ly, lz,
      });
    });

    const camAng = { yaw: cam.yaw + cam.parallaxX, pitch: cam.pitch + cam.parallaxY };
    sphereAtlas.render(atlasItems, camAng, timeSec, dark);

    const coreCanvas = domLayer.querySelector(".core-globe");
    if (coreCanvas) sphereAtlas.blitTile(0, coreCanvas.getContext("2d"), coreCanvas.width);
    wired.forEach((a, idx) => {
      const pCanvas = domLayer.querySelector(\`.planet-node[data-id="\${a.id}"] .planet-globe\`);
      if (pCanvas) sphereAtlas.blitTile(1 + idx, pCanvas.getContext("2d"), pCanvas.width);
    });
  }

  // 主循环
  let lastTs = performance.now();
  function frame(now) {
    const dt = Math.min(0.04, (now - lastTs) / 1000);
    lastTs = now;

    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const pauseOrbit = Boolean(!autoSpin || reducedMotion || hoveredId || dragState || cam.dragMode || scrubState || mode === "list");
    if (!pauseOrbit) {
      cam.orbitSpin += 0.052 * coreBody.galaxySpeed * dt;
    }

    // 中央恒星与每颗 Agent 折射晶核：默认呼吸灵动朝前可读 + 支持 360° × 360° 全向手动拨转惯性滑行（也可在面板调高持续自转倍速）
    if (!reducedMotion) {
      coreBody.selfSpin += (0.55 * coreBody.spinMul * (autoSpin ? 1 : 0) + coreBody.spinVel) * dt;
      coreBody.pitchSpin = (coreBody.pitchSpin ?? 0) + (coreBody.pitchVel ?? 0) * dt;
      coreBody.spinVel *= Math.exp(-3.4 * dt);
      coreBody.pitchVel = (coreBody.pitchVel ?? 0) * Math.exp(-3.6 * dt);
      if (Math.abs(coreBody.spinVel) < 0.002) coreBody.spinVel = 0;
      if (Math.abs(coreBody.pitchVel) < 0.002) coreBody.pitchVel = 0;

      wiredAgents().forEach((a) => {
        const rate = 0.65 * (a.spinMul ?? 0) * (autoSpin ? 1 : 0);
        a.selfSpin = (a.selfSpin ?? 0) + (rate + (a.spinVel ?? 0)) * dt;
        a.pitchSpin = (a.pitchSpin ?? 0) + (a.pitchVel ?? 0) * dt;
        a.spinVel = (a.spinVel ?? 0) * Math.exp(-3.3 * dt);
        a.pitchVel = (a.pitchVel ?? 0) * Math.exp(-3.5 * dt);
        if (Math.abs(a.spinVel) < 0.002) a.spinVel = 0;
        if (Math.abs(a.pitchVel) < 0.002) a.pitchVel = 0;
      });
    }

    // 360° 无死角星系轨道惯性滑行（解除俯仰角限制，可自由翻转俯瞰/仰视南天星云！）
    if (!cam.dragMode) {
      if (Math.abs(cam.vYaw) > 1e-4 || Math.abs(cam.vPitch) > 1e-4) {
        cam.targetYaw += cam.vYaw * dt * 60;
        cam.targetPitch += cam.vPitch * dt * 60;
        const damp = Math.exp(-5.4 * dt);
        cam.vYaw *= damp;
        cam.vPitch *= damp;
      }
      cam.yaw += (cam.targetYaw - cam.yaw) * Math.min(1, dt * 11);
      cam.pitch += (cam.targetPitch - cam.pitch) * Math.min(1, dt * 11);
    }

    // 选中焦点平滑追踪飞镜
    if (focusId === "core") {
      cam.targetFx = 0; cam.targetFy = 0; cam.targetFz = 0;
    } else if (focusId) {
      const fa = agents.find((x) => x.id === focusId && x.group === "wired");
      if (fa) {
        const fw = getAgentWorldPos(fa);
        cam.targetFx = fw.x; cam.targetFy = fw.y; cam.targetFz = fw.z;
      } else {
        focusId = null;
      }
    }
    const followK = Math.min(1, dt * 8.5);
    cam.fx += (cam.targetFx - cam.fx) * followK;
    cam.fy += (cam.targetFy - cam.fy) * followK;
    cam.fz += (cam.targetFz - cam.fz) * followK;
    cam.screenBiasX += (cam.targetScreenBiasX - cam.screenBiasX) * Math.min(1, dt * 9.5);
    cam.zoom += (cam.targetZoom - cam.zoom) * Math.min(1, dt * 10);

    // 液体弹簧积分
    for (const [id, st] of planetAnim.entries()) {
      const k = 170, d = 21.5, m = 0.9;
      const ax = (-k * st.dx - d * st.vx) / m;
      const ay = (-k * st.dy - d * st.vy) / m;
      const as = (-220 * (st.scale - 1) - 18 * st.vs) / 0.8;
      st.vx += ax * dt; st.vy += ay * dt; st.vs += as * dt;
      st.dx += st.vx * dt; st.dy += st.vy * dt; st.scale += st.vs * dt;
      if (Math.hypot(st.dx, st.dy, st.vx, st.vy) < 0.35 && Math.abs(st.scale - 1) < 0.005) planetAnim.delete(id);
    }

    for (let i = particles.length - 1; i >= 0; i--) {
      const p = particles[i];
      p.x += p.vx * dt; p.y += p.vy * dt; p.life -= dt;
      if (p.life <= 0) particles.splice(i, 1);
    }

    if (mode !== "list") {
      const w = stage.clientWidth, h = stage.clientHeight;
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      const wired = wiredAgents();

      if (glRenderer) {
        glRenderer.render({
          width: w, height: h, dpr,
          cx: w * 0.50 + cam.screenBiasX, cy: h * 0.50,
          fx: cam.fx, fy: cam.fy, fz: cam.fz,
          yaw: cam.yaw + cam.parallaxX,
          pitch: cam.pitch + cam.parallaxY,
          zoom: cam.zoom * stageFit(w, h),
          fov: FOV, time: now / 1000,
          diskSpin: cam.orbitSpin * 0.42,
          dark,
        });
      }
      drawOverlay(now, w, h, dpr);
      syncDomNodes();
      renderAndBlitSpheres(wired, now / 1000);
    }
    requestAnimationFrame(frame);
  }

  function drawOverlay(now, w, h, dpr) {
    if (overlayCanvas.width !== Math.round(w * dpr) || overlayCanvas.height !== Math.round(h * dpr)) {
      overlayCanvas.width = Math.round(w * dpr);
      overlayCanvas.height = Math.round(h * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const t = now / 1000;
    const wired = wiredAgents();
    const corePt = project3D(0, 0, 0, 1, w, h);

    // 银心日冕晕（增强 WebGL 与 Jolu 核之间的柔和光过渡）
    if (!corePt.behind) {
      const cR = 135 * Math.max(0.5, Math.min(1.6, corePt.scale));
      const cg = ctx.createRadialGradient(corePt.x, corePt.y, 18 * corePt.scale, corePt.x, corePt.y, cR);
      cg.addColorStop(0, dark ? "rgba(74, 240, 158, 0.24)" : "rgba(22, 168, 98, 0.14)");
      cg.addColorStop(0.55, dark ? "rgba(56, 189, 248, 0.06)" : "rgba(47, 123, 217, 0.04)");
      cg.addColorStop(1, "rgba(0,0,0,0)");
      ctx.fillStyle = cg;
      ctx.beginPath();
      ctx.arc(corePt.x, corePt.y, cR, 0, Math.PI * 2);
      ctx.fill();
    }

    // 如果缺少 WebGL2 退回轻星云提示点
    if (!glRenderer) {
      for (let i = 0; i < 90; i++) {
        const th = i * 2.39996;
        const rr = 30 + (i / 90) * 320;
        const p = project3D(Math.cos(th) * rr, Math.sin(i * 1.7) * 18, Math.sin(th) * rr, 1, w, h);
        if (p.behind) continue;
        ctx.fillStyle = dark ? "rgba(140, 240, 196, 0.45)" : "rgba(22, 112, 70, 0.28)";
        ctx.beginPath(); ctx.arc(p.x, p.y, 1.4 * p.scale, 0, Math.PI * 2); ctx.fill();
      }
    }

    const drawOrbitCurve = (spec, dashed, stroke, width, alpha, glow = null) => {
      ctx.save();
      ctx.globalAlpha = alpha;
      ctx.strokeStyle = stroke;
      ctx.lineWidth = width;
      if (glow) { ctx.shadowColor = glow; ctx.shadowBlur = 9; }
      if (dashed) ctx.setLineDash([4, 7]);
      ctx.beginPath();
      let started = false;
      const segs = 80;
      for (let i = 0; i <= segs; i++) {
        const a = (i / segs) * Math.PI * 2;
        const wp = orbitWorldPos(spec, a);
        const pt = project3D(wp.x, wp.y, wp.z, 1, w, h);
        if (pt.behind) { started = false; continue; }
        if (!started) { ctx.moveTo(pt.x, pt.y); started = true; }
        else ctx.lineTo(pt.x, pt.y);
      }
      ctx.stroke();
      ctx.restore();
    };

    // 行星微弱星轨（平常极淡退居幕后，悬停/选中时才绽放高亮）
    wired.forEach((a) => {
      const co = a.chaosOrbit;
      if (!co) return;
      const hot = selectedId === a.id || hoveredId === a.id;
      drawOrbitCurve(
        co, !hot,
        hot ? a.color : (dark ? "rgba(148, 224, 186, 0.14)" : "rgba(22, 112, 70, 0.11)"),
        hot ? 1.55 : 0.85,
        hot ? (dark ? 0.62 : 0.48) : 1,
        hot ? a.color : null
      );

      // 彗弧星尘尾迹
      const extra = cam.orbitSpin * co.speedMul;
      for (let s = 0; s < 9; s++) {
        const u0 = s / 9, u1 = (s + 1) / 9;
        const w0 = orbitWorldPos(co, co.angle + extra - 0.44 * (1 - u0));
        const w1 = orbitWorldPos(co, co.angle + extra - 0.44 * (1 - u1));
        const p0 = project3D(w0.x, w0.y, w0.z, 1, w, h);
        const p1 = project3D(w1.x, w1.y, w1.z, 1, w, h);
        if (p0.behind || p1.behind) continue;
        ctx.strokeStyle = a.color;
        ctx.globalAlpha = u1 * (hot ? 0.56 : (dark ? 0.36 : 0.24));
        ctx.lineWidth = 1.6 * u1 + 0.3;
        ctx.beginPath(); ctx.moveTo(p0.x, p0.y); ctx.lineTo(p1.x, p1.y); ctx.stroke();
      }
    });
    ctx.globalAlpha = 1;

    // 拖拽预览三维新轨 + 3D 垂直高度柱（直指银盘赤道平面 y=0）
    if (dragState?.captured && dragState.slotSpec) {
      drawOrbitCurve(dragState.slotSpec, true, dark ? "#6CF5AE" : "#16A862", 1.75, 0.82, "#16A862");
      if (dragState.slotWorld) {
        const wp = dragState.slotWorld;
        const topP = project3D(wp.x, wp.y, wp.z, 1, w, h);
        const baseP = project3D(wp.x, 0, wp.z, 1, w, h);
        if (!topP.behind && !baseP.behind) {
          ctx.save();
          ctx.strokeStyle = dark ? "rgba(108,245,174,0.65)" : "rgba(22,168,98,0.65)";
          ctx.lineWidth = 1.25; ctx.setLineDash([3, 3]);
          ctx.beginPath(); ctx.moveTo(topP.x, topP.y); ctx.lineTo(baseP.x, baseP.y); ctx.stroke();
          ctx.setLineDash([]);
          ctx.beginPath(); ctx.ellipse(baseP.x, baseP.y, 12 * baseP.scale, 5 * baseP.scale, 0, 0, Math.PI * 2); ctx.stroke();
          ctx.restore();
        }
      }
    }

    // 冲击波
    for (let i = shockwaves.length - 1; i >= 0; i--) {
      const sw = shockwaves[i];
      const p = (now - sw.start) / 620;
      if (p >= 1) { shockwaves.splice(i, 1); continue; }
      const ease = 1 - Math.pow(1 - p, 3);
      const sp = sw.world ? project3D(sw.world.x, sw.world.y, sw.world.z, 1, w, h) : corePt;
      ctx.strokeStyle = sw.color;
      ctx.globalAlpha = (1 - p) * 0.78;
      ctx.lineWidth = 3 * (1 - p * 0.6);
      ctx.beginPath(); ctx.arc(sp.x, sp.y, (28 + ease * 210) * sp.scale, 0, Math.PI * 2); ctx.stroke();
      ctx.globalAlpha = 1;
    }

    // 画家排序绘制各行星的引力丝线、流光粒子、垂直高度投影杆与 3D 伴生卫星
    const items = [];
    wired.forEach((a, idx) => {
      const wp = getAgentWorldPos(a);
      const proj = project3D(wp.x, wp.y, wp.z, 1, w, h);
      if (proj.behind) return;
      const anim = planetAnim.get(a.id);
      items.push({
        a, wp,
        x: proj.x + (anim ? anim.dx : 0),
        y: proj.y + (anim ? anim.dy : 0),
        z: proj.z, scale: proj.scale * (anim ? anim.scale : 1), idx,
      });
    });
    items.sort((u, v) => v.z - u.z);

    for (const item of items) {
      const { a, wp, x, y, scale, idx } = item;
      const hot = selectedId === a.id || hoveredId === a.id;
      const dimmed = searchQuery.trim() && !a.name.toLowerCase().includes(searchQuery.trim().toLowerCase());
      ctx.save();
      if (dimmed) ctx.globalAlpha = 0.22;

      // 1) 垂直 3D 高度落差线（悬停或选中时浮现，一目了然星体悬在银盘之上还是之下）
      if (hot && Math.abs(wp.y) > 4) {
        const foot = project3D(wp.x, 0, wp.z, 1, w, h);
        if (!foot.behind) {
          ctx.save();
          ctx.strokeStyle = a.color;
          ctx.globalAlpha = 0.45;
          ctx.setLineDash([3, 4]);
          ctx.lineWidth = 1.1;
          ctx.beginPath(); ctx.moveTo(x, y); ctx.lineTo(foot.x, foot.y); ctx.stroke();
          ctx.setLineDash([]);
          ctx.beginPath(); ctx.ellipse(foot.x, foot.y, 10 * foot.scale, 4.5 * foot.scale, 0, 0, Math.PI * 2); ctx.stroke();
          ctx.restore();
        }
      }

      // 2) 引力微弧能量连线 & 路由流光粒子
      if (!corePt.behind) {
        const mx = (corePt.x + x) * 0.5 + (y - corePt.y) * 0.11;
        const my = (corePt.y + y) * 0.5 - (x - corePt.x) * 0.11;
        ctx.beginPath();
        ctx.moveTo(corePt.x, corePt.y);
        ctx.quadraticCurveTo(mx, my, x, y);
        if (a.tone === "danger") {
          ctx.setLineDash([5, 5]); ctx.strokeStyle = dark ? "rgba(255,107,111,0.72)" : "rgba(224,72,77,0.62)"; ctx.lineWidth = 1.6;
        } else if (a.tone === "warn") {
          ctx.setLineDash([4, 4]); ctx.strokeStyle = dark ? "rgba(245,184,61,0.62)" : "rgba(217,142,11,0.56)"; ctx.lineWidth = 1.4;
        } else {
          ctx.strokeStyle = dark ? (hot ? "rgba(92,244,166,0.68)" : "rgba(61,214,140,0.28)") : (hot ? "rgba(22,168,98,0.60)" : "rgba(22,168,98,0.22)");
          ctx.lineWidth = hot ? 1.85 : 1.15;
        }
        ctx.stroke();
        ctx.setLineDash([]);

        if (a.routesToday > 0) {
          const speed = 0.34 + Math.min(0.46, a.routesToday / 280);
          const count = a.routesToday > 60 ? 2 : 1;
          for (let k = 0; k < count; k++) {
            const u = (t * speed + idx * 0.23 + k * 0.5) % 1;
            const inv = 1 - u;
            const bx = inv * inv * corePt.x + 2 * inv * u * mx + u * u * x;
            const by = inv * inv * corePt.y + 2 * inv * u * my + u * u * y;
            const pc = a.tone === "danger" ? "#FF6B6F" : a.tone === "warn" ? "#F5B83D" : (dark ? "#6CF5AE" : "#16A862");
            ctx.fillStyle = pc;
            ctx.shadowColor = pc; ctx.shadowBlur = 9;
            ctx.beginPath(); ctx.arc(bx, by, 2.7 * Math.max(0.7, scale), 0, Math.PI * 2); ctx.fill();
            ctx.shadowBlur = 0;
          }
        }
      }

      // 3) 真正 3D 倾角公转的 4 颗 Claude 档位卫星
      if (a.moons && a.moons.length) {
        const mR = 46;
        a.moons.forEach((mName, mi) => {
          const ma = t * 0.62 + (mi / a.moons.length) * Math.PI * 2;
          const mwx = wp.x + Math.cos(ma) * mR;
          const mwy = wp.y + Math.sin(ma) * mR * 0.38;
          const mwz = wp.z + Math.sin(ma) * mR * 0.92;
          const mp = project3D(mwx, mwy, mwz, 1, w, h);
          if (mp.behind) return;
          ctx.fillStyle = dark ? "#F2FAF6" : a.color;
          ctx.beginPath(); ctx.arc(mp.x, mp.y, 2.6 * mp.scale, 0, Math.PI * 2); ctx.fill();
          if (selectedId === a.id || cam.zoom > 1.28) {
            ctx.font = '600 9.5px "Inter", sans-serif';
            ctx.fillStyle = dark ? "rgba(232,245,238,0.82)" : "rgba(26,40,34,0.78)";
            ctx.fillText(mName, mp.x + 6, mp.y + 3);
          }
        });
      }

      ctx.restore();
    }

    // 拖拽牵引束与彗尾粒子
    if (dragState) {
      if (dragState.captured && dragState.slotScreen && !corePt.behind) {
        ctx.save();
        const grad = ctx.createLinearGradient(corePt.x, corePt.y, dragState.x, dragState.y);
        grad.addColorStop(0, dark ? "rgba(61,214,140,0.85)" : "rgba(22,168,98,0.82)");
        grad.addColorStop(1, dragState.color);
        ctx.strokeStyle = grad;
        ctx.lineWidth = 2.2; ctx.setLineDash([7, 5]); ctx.lineDashOffset = -t * 48;
        ctx.beginPath(); ctx.moveTo(corePt.x, corePt.y); ctx.lineTo(dragState.x, dragState.y); ctx.lineTo(dragState.slotScreen.x, dragState.slotScreen.y); ctx.stroke();
        ctx.restore();
      }
      for (const p of particles) {
        const alpha = Math.max(0, p.life / p.maxLife);
        ctx.fillStyle = p.color; ctx.globalAlpha = alpha * 0.72;
        ctx.beginPath(); ctx.arc(p.x, p.y, p.r * alpha, 0, Math.PI * 2); ctx.fill();
      }
      ctx.globalAlpha = 1;
    }
  }

  function renderHangar() {
    const list = dockAgents();
    const unsup = unsupportedAgents();
    $("#hangar-count").textContent = String(list.length);
    $("#cnt-all").textContent = String(wiredAgents().length + list.length);
    $("#cnt-wired").textContent = String(wiredAgents().length);
    $("#cnt-avail").textContent = String(list.length);
    $("#hangar-hint").textContent = hangarCollapsed ? "展开 ▸" : "拖小晶核入轨 ▾";
    hangarCard.classList.toggle("collapsed", hangarCollapsed);

    hangarBody.innerHTML = "";
    if (list.length === 0) {
      hangarBody.innerHTML = '<div class="hangar-empty"><span>✨ 全部支持工具已入轨星系</span><button class="btn" id="btn-inline-reset" style="height:28px;font-size:11px;" type="button">↺ 重置演示再拖一次</button></div>';
      hangarBody.querySelector("#btn-inline-reset")?.addEventListener("click", resetDemo);
    } else {
      list.forEach((a) => {
        const item = document.createElement("div");
        item.className = "hangar-item"; item.dataset.id = a.id;
        const glyphSrc = (dark ? PURE_GLYPHS_DARK : PURE_GLYPHS_LIGHT)[a.id] || logoFor(a.id);
        const orbBg = dark
          ? \`radial-gradient(circle at 36% 32%, rgba(255,255,255,0.20), rgba(14,26,21,0.94) 55%, \${a.color}66 86%, \${a.color}AA 100%)\`
          : \`radial-gradient(circle at 38% 36%, rgba(255,255,255,0.98) 0%, rgba(246,252,249,0.95) 56%, \${a.color}48 84%, \${a.color}A4 100%)\`;
        const imgFilter = dark ? \`drop-shadow(0 0 4px \${a.color}55)\` : "drop-shadow(0 1px 1.5px rgba(15,35,25,0.14))";
        item.innerHTML = \`
          <div class="hangar-orb" style="background:\${orbBg};border:1px solid \${a.color}55;">
            <img src="\${glyphSrc}" alt="" style="filter:\${imgFilter};" />
          </div>
          <div class="hangar-info"><div class="hangar-name">\${a.name}</div><div class="hangar-sub \${a.cliMissing ? "warn" : ""}">\${a.subtitle}</div></div>
          <button class="hangar-plus" type="button" title="一键飞入星轨接入">+</button>
        \`;
        item.querySelector(".hangar-plus").addEventListener("pointerdown", (e) => e.stopPropagation());
        item.querySelector(".hangar-plus").addEventListener("click", (e) => {
          e.stopPropagation();
          const r = item.querySelector(".hangar-orb").getBoundingClientRect();
          wireAgentFromOrigin(a.id, r.left + r.width / 2, r.top + r.height / 2, false);
        });
        item.addEventListener("pointerdown", (e) => startPlanetDrag(e, a, item));
        hangarBody.appendChild(item);
      });
    }
    if (unsup.length > 0) {
      const row = document.createElement("div");
      row.className = "hangar-unsupported";
      row.innerHTML = \`<span>⚙</span><span>\${unsup.map((x) => x.name).join("、")} 暂无专接</span><span class="spacer"></span><span>通用接入 ›</span>\`;
      row.addEventListener("click", () => toast("通用接入提示", "Antigravity CLI 需点击右上「+ 添加工具」配置键名映射", false));
      hangarBody.appendChild(row);
    }
    updateHudStats();
  }

  function startPlanetDrag(e, agent, itemEl) {
    if (e.button !== 0 || mode === "list") return;
    const startX = e.clientX, startY = e.clientY;
    let moved = false;

    const onMove = (ev) => {
      if (!moved && Math.hypot(ev.clientX - startX, ev.clientY - startY) > 4) {
        moved = true;
        itemEl.classList.add("launching");
        const sr = stage.getBoundingClientRect();
        dragState = {
          agentId: agent.id, color: agent.color,
          x: ev.clientX - sr.left, y: ev.clientY - sr.top,
          altY: (Math.random() - 0.5) * 24,
          captured: false, slotScreen: null, slotSpec: null, slotWorld: null,
        };
        dragOrbImg.src = (dark ? PURE_GLYPHS_DARK : PURE_GLYPHS_LIGHT)[agent.id] || logoFor(agent.id);
        dragOrbImg.style.filter = dark ? \`drop-shadow(0 0 6px \${agent.color}66)\` : "drop-shadow(0 1px 2px rgba(15,35,25,0.16))";
        dragOrb.style.background = dark
          ? \`radial-gradient(circle at 36% 32%, rgba(255,255,255,0.22), rgba(14,26,21,0.95) 55%, \${agent.color}77 86%, \${agent.color}BC 100%)\`
          : \`radial-gradient(circle at 38% 36%, rgba(255,255,255,0.99) 0%, rgba(246,252,249,0.96) 56%, \${agent.color}55 84%, \${agent.color}B4 100%)\`;
        dragOrb.style.border = \`1px solid \${agent.color}66\`;
        dragOrb.classList.add("active");
        $("#stage-hint").style.opacity = "0";
      }
      if (!moved || !dragState) return;
      const sr = stage.getBoundingClientRect();
      const sx = ev.clientX - sr.left, sy = ev.clientY - sr.top;
      dragState.x = sx; dragState.y = sy;

      particles.push({
        x: sx + (Math.random() - 0.5) * 6, y: sy + (Math.random() - 0.5) * 6,
        vx: (Math.random() - 0.5) * 28, vy: (Math.random() - 0.5) * 28,
        life: 0.40, maxLife: 0.40, color: dark ? "#6CF5AE" : agent.color, r: 4.5,
      });

      const hr = hangarCard.getBoundingClientRect();
      const overHangar = ev.clientX >= hr.left && ev.clientX <= hr.right && ev.clientY >= hr.top && ev.clientY <= hr.bottom;
      const overRightPanel = inspectCard.classList.contains("open") && sx > sr.width - 328 && sy < 520;
      const overHud = sy < 58 || overRightPanel || (sx > sr.width - 330 && sy > sr.height - 54);
      const inStage = ev.clientX >= sr.left && ev.clientX <= sr.right && ev.clientY >= sr.top && ev.clientY <= sr.bottom;
      dragState.captured = Boolean(inStage && !overHangar && !overHud);

      let drawX = ev.clientX, drawY = ev.clientY;
      if (dragState.captured && dragState.slotScreen) {
        const tx = sr.left + dragState.slotScreen.x, ty = sr.top + dragState.slotScreen.y;
        const pull = Math.max(0, 1 - Math.hypot(tx - ev.clientX, ty - ev.clientY) / 160) * 0.25;
        drawX += (tx - ev.clientX) * pull; drawY += (ty - ev.clientY) * pull;
      }
      dragOrb.style.transform = \`translate3d(\${drawX}px, \${drawY}px, 0) scale(\${dragState.captured ? 1.14 : 1.04})\`;
    };

    const onWheelWhileDrag = (ev) => {
      if (!dragState) return;
      ev.preventDefault();
      dragState.altY = Math.max(-85, Math.min(85, dragState.altY - ev.deltaY * 0.12));
    };

    const onUp = (ev) => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("wheel", onWheelWhileDrag);
      itemEl.classList.remove("launching");
      dragOrb.classList.remove("active");
      $("#stage-hint").style.opacity = "";
      if (!moved || !dragState) { dragState = null; return; }
      const wasCap = dragState.captured;
      const altY = dragState.altY;
      dragState = null;
      if (wasCap) wireAgentFromOrigin(agent.id, ev.clientX, ev.clientY, true, altY);
      else toast("未进入星系捕获区", \`已将 \${agent.name} 放回待接入星坞\`, true);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("wheel", onWheelWhileDrag, { passive: false });
  }

  function wireAgentFromOrigin(id, clientX, clientY, fromDrop = false, altY = 0) {
    const a = agents.find((x) => x.id === id);
    if (!a || a.group === "wired") return;
    const sr = stage.getBoundingClientRect();
    const ox = clientX - sr.left, oy = clientY - sr.top;

    if (fromDrop) {
      const wp = screenToWorldPlane(ox, oy, altY, stage.clientWidth, stage.clientHeight);
      const custom = makeCustomOrbitAtWorld(wp.x, wp.y, wp.z, (a.name.length * 17) + wiredAgents().length);
      custom.angle -= cam.orbitSpin * custom.speedMul;
      custom.peri = custom.angle;
      a.chaosOrbit = custom;
      a.chaosSeed = chaosSeed;
    } else {
      a.chaosSeed = -1;
      ensureChaosOrbits(false);
    }

    a.group = "wired"; a.status = "已接入 · 配置一致"; a.tone = "normal"; a.routesToday = Math.max(1, a.routesToday);
    a.selfSpin = cam.targetYaw; a.pitchSpin = 0; a.spinVel = Math.PI * 2 * 3.3;
    const wp = getAgentWorldPos(a);
    const target = project3D(wp.x, wp.y, wp.z, 1, stage.clientWidth, stage.clientHeight);
    planetAnim.set(id, { dx: ox - target.x, dy: oy - target.y, vx: 0, vy: 0, scale: fromDrop ? 1.24 : 1.34, vs: 0 });
    shockwaves.push({ start: performance.now(), world: fromDrop ? wp : null, color: dark ? "#3DD68C" : "#16A862" });

    if (mode !== "list") focusAgent(id, !fromDrop ? false : true);
    else if (selectedId === id) focusAgent(id, false);
    renderHangar(); renderListView();
    toast(\`已一键直连 \${a.name}\`, \`已写入 \${a.configPath} · 模型路由 :4141 联动\`, false, [
      { label: "查看改动", onClick: () => openDiffModal(id) },
      { label: "撤回", onClick: () => unwireAgent(id, true) },
    ]);
  }

  function unwireAgent(id, silent = false) {
    const a = agents.find((x) => x.id === id);
    if (!a || a.group !== "wired") return;
    a.group = "available"; a.status = "可接入";
    if (selectedId === id) focusAgent(null);
    renderHangar(); renderListView();
    if (!silent) toast(\`已断开 \${a.name}\`, "已还原配置文件，小星球退回待接入星坞", true);
  }

  // 点击行星或中央 Jolu 恒星核（列表模式则从右侧展开同一份详情抽屉）
  let hangarBeforeFocus = null;
  let listBlockedOpen = false;
  function focusAgent(id, shouldZoom = true) {
    if (mode === "list" && id === "core") id = null;
    selectedId = id;
    focusId = id;
    const w = stage.clientWidth || 1080;

    if (!id) {
      inspectCard.classList.remove("open");
      stage.classList.remove("list-has-inspect");
      cam.targetScreenBiasX = 0;
      if (hangarBeforeFocus !== null) {
        hangarCollapsed = hangarBeforeFocus;
        hangarBeforeFocus = null;
        renderHangar();
      }
      $("#stage-hint").textContent = "360° 自由拨转晶核球体 · 360° 无死角环绕星云 · 滚轮缩放 · 点击星球拉近展开设置";
      if (mode !== "list") {
        cam.targetFx = 0; cam.targetFy = 0; cam.targetFz = 0;
        if (cam.targetZoom > 1.15) cam.targetZoom = 1.0;
        if (cam.preset === "oblique" && Math.abs(cam.targetPitch - 0.44) > 0.02) cam.targetPitch = 0.44;
      }
      if (mode === "list") renderListView();
      return;
    }

    if (mode === "list") {
      stage.classList.add("list-has-inspect");
    } else if (hangarBeforeFocus === null) {
      hangarBeforeFocus = hangarCollapsed;
      if (!hangarCollapsed) {
        hangarCollapsed = true;
        renderHangar();
      }
    }

    const biasX = -Math.min(126, w * 0.102);

    // 1) 聚焦中央 Jolu 折射晶核：拉近检视并打开网关全局设置
    if (id === "core") {
      cam.targetFx = 0; cam.targetFy = 0; cam.targetFz = 0;
      cam.targetScreenBiasX = biasX;
      if (shouldZoom) {
        cam.targetZoom = Math.max(cam.targetZoom, 1.72);
        cam.targetPitch = 0.36;
      }
      inspectCard.classList.add("open");
      $("#stage-hint").textContent = "已拉近聚焦 Jolu 晶核 · 可全向 360° 拖拽球体旋转 · Esc / 点击星空返回全景";
      inspectCard.innerHTML = \`
        <div class="inspect-head">
          <img src="\${LOGOS.appIcon}" alt="" style="border-radius:9px;" />
          <div style="min-width:0;flex:1;">
            <div class="inspect-name">Jolu 模型路由核心 <span class="mono" style="font-size:10.5px;color:var(--jn-primary-ink);font-weight:600;">:4141</span></div>
            <div class="inspect-path">本地代理枢纽 · 已直连 \${wiredAgents().length} 个 Agent</div>
          </div>
          <button class="inspect-back-pill" id="inspect-back" type="button">← 全景</button>
          <button class="icon-btn" id="inspect-close" type="button" aria-label="返回全景">✕</button>
        </div>
        <div class="setting-section">
          <div class="setting-sec-title"><span>路由策略预设</span><span class="mono">:4141</span></div>
          <div class="setting-row">
            <label for="core-preset-sel">当前档位预设</label>
            <select class="setting-select" id="core-preset-sel">
              \${["日常编码 · 6 档位", "深度推理 · Opus 优先", "极速省钱 · Flash/Haiku"].map((p) => \`<option value="\${p}" \${coreBody.preset === p ? "selected" : ""}>\${p}</option>\`).join("")}
            </select>
          </div>
          <div class="setting-row">
            <label>整体星系公转速度</label>
            <input class="setting-slider" id="core-galaxy-speed" type="range" min="0" max="2.5" step="0.1" value="\${coreBody.galaxySpeed}" />
          </div>
        </div>
        <div class="setting-section">
          <div class="setting-sec-title"><span>360° 晶核自转控制</span><span class="tnum" id="core-spin-readout">\${coreBody.spinMul.toFixed(1)}×</span></div>
          <div class="setting-row">
            <label>核心自转倍速</label>
            <input class="setting-slider" id="core-spin-mul" type="range" min="0" max="3" step="0.1" value="\${coreBody.spinMul}" />
          </div>
        </div>
        <div class="inspect-actions">
          <button class="btn" id="btn-core-spin-kick" type="button">↻ 360° 拨转</button>
          <button class="btn" id="btn-core-spin-reset" type="button">↺ 正面复位</button>
          <button class="btn btn-primary" id="btn-core-align" type="button">全部扫描对齐</button>
        </div>
      \`;
      inspectCard.querySelector("#inspect-back").addEventListener("click", () => focusAgent(null));
      inspectCard.querySelector("#inspect-close").addEventListener("click", () => focusAgent(null));
      inspectCard.querySelector("#core-preset-sel").addEventListener("change", (e) => {
        coreBody.preset = e.target.value;
        toast("已切换模型路由预设", \`当前方案：\${coreBody.preset}\`);
      });
      inspectCard.querySelector("#core-galaxy-speed").addEventListener("input", (e) => {
        coreBody.galaxySpeed = Number(e.target.value);
      });
      inspectCard.querySelector("#core-spin-mul").addEventListener("input", (e) => {
        coreBody.spinMul = Number(e.target.value);
        inspectCard.querySelector("#core-spin-readout").textContent = coreBody.spinMul.toFixed(1) + "×";
      });
      inspectCard.querySelector("#btn-core-spin-kick").addEventListener("click", () => {
        coreBody.spinVel += Math.PI * 2 * 3.4;
      });
      inspectCard.querySelector("#btn-core-spin-reset").addEventListener("click", () => {
        coreBody.selfSpin = cam.yaw; coreBody.pitchSpin = 0; coreBody.spinVel = 0; coreBody.pitchVel = 0;
      });
      inspectCard.querySelector("#btn-core-align").addEventListener("click", () => {
        toast("全部工具校验通过", \`已确认 \${wiredAgents().length} 个已接 Agent 指向 http://127.0.0.1:4141\`);
      });
      return;
    }

    // 2) 聚焦某个已接入 Agent 晶核星球：直接拉近至特写景深 + 右侧展开「详情与设置」面板
    const a = agents.find((x) => x.id === id && x.group === "wired");
    if (!a) {
      focusAgent(null);
      return;
    }

    if (mode !== "list") {
      cam.targetScreenBiasX = biasX;
      if (shouldZoom) {
        cam.targetZoom = Math.max(cam.targetZoom, 2.02);
        cam.targetPitch = 0.34;
      }
      $("#stage-hint").textContent = \`已拉近 \${a.name} · 上下左右 360° 拖拽晶核旋转 · 右侧可实时调参\`;
    }
    inspectCard.classList.add("open");

    const modelOpts = Array.from(new Set([a.model, ...(a.modelOptions || [])].filter(Boolean)));
    const elevVal = Math.round(a.chaosOrbit?.elevY ?? 0);
    const spinVal = (a.spinMul ?? 1.0).toFixed(1);
    const tierModels = ["claude-opus-4-1", "sonnet-4-5", "haiku-4-5", "fable-preview"];
    const tiers = a.moons?.length
      ? \`<div class="tier-grid">\${a.moons.map((m, i) => \`<button class="tier-chip \${a.activeTier === i ? "active" : ""}" data-tier-idx="\${i}" type="button"><span>\${m} \${mode === "list" ? "档位模型" : "档卫星"}</span><b>\${tierModels[i]}</b></button>\`).join("")}</div>\`
      : "";
    const fixBtn = a.tone === "danger"
      ? '<button class="btn btn-primary" id="btn-fix-drift" type="button">一键修复漂移</button>'
      : a.tone === "warn" ? '<button class="btn btn-primary" id="btn-restart-ok" type="button">标记已重开生效</button>' : "";
    const is3d = mode !== "list";

    inspectCard.innerHTML = \`
      <div class="inspect-head">
        <img src="\${logoFor(a.id)}" alt="" />
        <div style="min-width:0;flex:1;">
          <div class="inspect-name">\${a.name} <span class="mono" style="font-size:10.5px;color:var(--jn-text-3);font-weight:500;">\${a.version}</span></div>
          <div class="inspect-path mono" title="\${a.configPath}">\${a.configPath}</div>
        </div>
        <button class="inspect-back-pill" id="inspect-back" type="button">\${is3d ? "← 全景" : "← 收起"}</button>
        <button class="icon-btn" id="inspect-close" type="button" aria-label="关闭详情">✕</button>
      </div>

      <div style="display:flex;align-items:center;gap:6px;font-size:11.5px;">
        <span style="padding:2px 8px;border-radius:999px;font-weight:600;background:\${a.tone==="danger"?"var(--jn-danger-soft)":a.tone==="warn"?"var(--jn-warning-soft)":"var(--jn-primary-soft)"};color:\${a.tone==="danger"?"var(--jn-danger)":a.tone==="warn"?"var(--jn-warning)":"var(--jn-primary-ink)"};">\${a.status}</span>
        <span class="spacer"></span>
        <span class="tnum" style="color:var(--jn-text-3);">今日 \${a.routesToday} 次</span>
      </div>

      <div class="setting-section">
        <div class="setting-sec-title"><span>代理接入与模型设置</span></div>
        <div class="setting-row">
          <label>启用 Jolu 模型路由</label>
          <button class="switch-pill on" id="setting-wire-toggle" type="button" aria-label="切换接入"><span class="switch-knob"></span></button>
        </div>
        <div class="setting-row">
          <label for="setting-model-sel">默认主模型</label>
          <select class="setting-select" id="setting-model-sel">
            \${modelOpts.map((m) => \`<option value="\${m}" \${a.model === m ? "selected" : ""}>\${m}</option>\`).join("")}
          </select>
        </div>
        <div class="setting-row">
          <label>写入策略</label>
          <div class="seg-mini" id="setting-cfg-mode">
            <button type="button" data-cfg="keep" class="\${a.configMode !== "managed" ? "active" : ""}">保留原账号</button>
            <button type="button" data-cfg="managed" class="\${a.configMode === "managed" ? "active" : ""}">完全托管</button>
          </div>
        </div>
      </div>

      \${tiers ? \`<div class="setting-section"><div class="setting-sec-title"><span>\${is3d ? "3D 伴生档位卫星" : "模型档位预设"}</span><span>点击联动切换</span></div>\${tiers}</div>\` : ""}

      \${is3d ? \`
      <div class="setting-section">
        <div class="setting-sec-title">
          <span>360° 晶核与轨道控制</span>
          <span style="display:inline-flex;gap:4px;">
            <button class="toast-link" id="btn-planet-spin-kick" type="button">↻ 360° 拨转</button>
            <button class="toast-link" id="btn-planet-spin-reset" type="button">↺ 正面</button>
          </span>
        </div>
        <div class="setting-row">
          <label>持续自转 <span class="mono" id="spin-readout">\${a.spinMul > 0.02 ? spinVal + "×" : "呼吸"}</span></label>
          <input class="setting-slider" id="setting-spin-mul" type="range" min="0" max="3" step="0.1" value="\${spinVal}" />
        </div>
        <div class="setting-row">
          <label>轨道高度 <span class="mono" id="elev-readout">\${elevVal > 0 ? "+" : ""}\${elevVal}</span></label>
          <input class="setting-slider" id="setting-elev-y" type="range" min="-70" max="70" step="2" value="\${elevVal}" />
        </div>
      </div>\` : ""}

      <div class="inspect-actions">\${fixBtn}<button class="btn" id="btn-inspect-diff" type="button">查看改动 Diff</button><button class="btn btn-danger" id="btn-inspect-unwire" type="button">断开归坞</button></div>
    \`;

    inspectCard.querySelector("#inspect-back").addEventListener("click", () => focusAgent(null));
    inspectCard.querySelector("#inspect-close").addEventListener("click", () => focusAgent(null));
    inspectCard.querySelector("#btn-inspect-diff").addEventListener("click", () => openDiffModal(a.id));
    inspectCard.querySelector("#btn-inspect-unwire").addEventListener("click", () => unwireAgent(a.id));
    inspectCard.querySelector("#setting-wire-toggle").addEventListener("click", () => unwireAgent(a.id));

    inspectCard.querySelector("#setting-model-sel").addEventListener("change", (e) => {
      a.model = e.target.value;
      a.spinVel += 2.2;
      renderListView();
      toast(\`已更新 \${a.name} 模型\`, \`已实时切换默认模型为 \${a.model}\`);
    });

    inspectCard.querySelector("#setting-cfg-mode").addEventListener("click", (e) => {
      const btn = e.target.closest("[data-cfg]");
      if (!btn) return;
      a.configMode = btn.dataset.cfg;
      inspectCard.querySelectorAll("#setting-cfg-mode button").forEach((b) => b.classList.toggle("active", b === btn));
      renderListView();
      toast("已切换写入策略", a.configMode === "managed" ? "将由 Jolu 统一托管完整配置块" : "仅注入模型网关字段，保留原有登录态");
    });

    inspectCard.querySelectorAll("[data-tier-idx]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const ti = Number(btn.dataset.tierIdx);
        a.activeTier = ti;
        a.model = tierModels[ti] ?? a.model;
        a.spinVel += 2.6;
        focusAgent(a.id, false);
        renderListView();
      });
    });

    inspectCard.querySelector("#setting-spin-mul")?.addEventListener("input", (e) => {
      a.spinMul = Number(e.target.value);
      inspectCard.querySelector("#spin-readout").textContent = a.spinMul > 0.02 ? a.spinMul.toFixed(1) + "×" : "呼吸";
    });
    inspectCard.querySelector("#setting-elev-y")?.addEventListener("input", (e) => {
      if (a.chaosOrbit) a.chaosOrbit.elevY = Number(e.target.value);
      const ev = Math.round(Number(e.target.value));
      inspectCard.querySelector("#elev-readout").textContent = (ev > 0 ? "+" : "") + ev;
    });
    inspectCard.querySelector("#btn-planet-spin-kick")?.addEventListener("click", () => {
      a.spinVel += Math.PI * 2 * 3.3;
    });
    inspectCard.querySelector("#btn-planet-spin-reset")?.addEventListener("click", () => {
      a.selfSpin = cam.yaw; a.pitchSpin = 0; a.spinVel = 0; a.pitchVel = 0;
    });

    inspectCard.querySelector("#btn-fix-drift")?.addEventListener("click", () => {
      a.tone = "normal"; a.status = "已接入 · 配置一致"; a.model = "gemini-2.5-pro";
      shockwaves.push({ start: performance.now(), color: dark ? "#3DD68C" : "#16A862" });
      focusAgent(a.id, false); updateHudStats(); renderListView();
      toast("已修复 Gemini CLI 漂移", "已重新对齐 ~/.gemini/.env 网关地址");
    });
    inspectCard.querySelector("#btn-restart-ok")?.addEventListener("click", () => {
      a.tone = "normal"; a.status = "已接入 · 配置一致"; a.model = "gpt-5-codex";
      focusAgent(a.id, false); updateHudStats(); renderListView();
      toast("Codex 已生效", "已检测到新会话走本机模型路由");
    });

    if (mode === "list") renderListView();
  }

  function updateHudStats() {
    const err = wiredAgents().filter((a) => a.tone === "danger").length;
    $("#btn-drift-jump").style.display = err > 0 ? "inline-flex" : "none";
  }

  function renderListView() {
    listView.classList.toggle("active", mode === "list");
    if (mode !== "list") return;

    const q = searchQuery.trim().toLowerCase();
    const matchesQ = (a) => !q || a.name.toLowerCase().includes(q) || a.id.includes(q) || (a.model || "").toLowerCase().includes(q) || (a.configPath || "").toLowerCase().includes(q);
    const allWired = wiredAgents();
    const supportedTotal = allWired.length + agents.filter((x) => x.group === "available" && x.supported).length;
    const totalRoutes = allWired.reduce((sum, x) => sum + (x.routesToday || 0), 0);
    const driftItem = allWired.find((x) => x.tone === "danger");

    const wiredRows = filter === "available" ? [] : allWired.filter(matchesQ);
    const availRows = filter === "wired"
      ? []
      : agents.filter((a) => a.group === "available" && matchesQ(a))
        .sort((x, y) => (x.supported === y.supported ? 0 : x.supported ? -1 : 1));
    const blockedRows = (filter === "wired")
      ? []
      : BLOCKED_AGENTS.filter((b) => !q || b.name.toLowerCase().includes(q) || b.reason.toLowerCase().includes(q));

    const glyphForTile = (a) => (dark ? PURE_GLYPHS_DARK : PURE_GLYPHS_LIGHT)[a.id] || logoFor(a.id);

    const resolveWellModel = (a) => {
      if (a.group !== "wired") return a.supported ? (a.model ? \`\${a.model} · 待接入\` : "可直连 :4141 网关") : "按「添加工具」通用接入";
      if (a.model === "配置漂移!") return "外部覆盖 · 未经 :4141 路由";
      if (a.model === "重开生效") return a.modelOptions?.find((m) => m !== "重开生效") || "gpt-5-codex";
      if (a.model === "配置一致") return a.modelOptions?.find((m) => m !== "配置一致") || "claude-sonnet-4-5";
      return a.model;
    };

    const resolveWellSub = (a) => {
      if (a.group === "wired" && a.tone === "danger") return "应指向 :4141/gemini · 请点击一键修复";
      if (a.group === "wired" && a.tone === "warn") return "配置已写入 · 重开会话即可生效";
      return a.subtitle;
    };

    const renderToolCard = (a) => {
      const wired = a.group === "wired";
      const selected = selectedId === a.id;
      const toneCls = !wired ? (a.supported ? "idle" : "muted") : (a.tone === "danger" ? "danger" : a.tone === "warn" ? "warn" : "normal");
      const tileBg = dark ? \`color-mix(in srgb, \${a.color} 15%, var(--jn-well))\` : \`color-mix(in srgb, \${a.color} 8%, var(--jn-surface))\`;
      const tileBorder = \`color-mix(in srgb, \${a.color} 28%, var(--jn-line))\`;

      const wellMetaRight = wired
        ? (a.routesToday > 0
          ? \`<span class="tool-routes-pill">今日 \${a.routesToday} 次</span>\`
          : \`<span class="tool-routes-pill muted">\${a.configMode === "managed" ? "完全托管" : "保留原账号"}</span>\`)
        : a.cliMissing
          ? '<span class="tool-routes-pill warn">未在 PATH</span>'
          : (a.supported ? '<span class="tool-routes-pill muted">支持一键直连</span>' : '<span class="tool-routes-pill muted">通用映射</span>');

      const wellValCls = wired ? (a.tone === "danger" ? " danger" : a.tone === "warn" ? " warn" : "") : "";
      const wellValText = resolveWellModel(a);

      const wellExtra = a.moons?.length && wired
        ? \`<div class="tool-tier-pills">\${a.moons.map((m, i) => \`<span class="tool-tier-pill\${a.activeTier === i ? " active" : ""}">\${m}</span>\`).join("")}</div>\`
        : \`<span class="tool-well-sub">\${resolveWellSub(a)}</span>\`;

      const quickFixBtn = wired && a.tone === "danger"
        ? \`<button class="tool-mini-btn danger" data-fix="\${a.id}" type="button">一键修复</button>\`
        : wired && a.tone === "warn"
          ? \`<button class="tool-mini-btn warn" data-restart="\${a.id}" type="button">已重开</button>\`
          : "";

      const footActions = a.supported
        ? \`\${quickFixBtn}<button class="tool-mini-btn" data-diff="\${a.id}" type="button">Diff</button>\${wired ? \`<button class="tool-detail-btn" data-select="\${a.id}" type="button">详细 ›</button>\` : \`<button class="tool-detail-btn" data-wire="\${a.id}" type="button">直连 ›</button>\`}\`
        : '<button class="tool-detail-btn" data-add-custom="1" type="button">配置 ›</button>';

      return \`
        <article class="tool-card\${selected ? " is-selected" : ""}\${a.tone === "danger" ? " is-danger" : a.tone === "warn" ? " is-warn" : ""}\${!a.supported ? " is-unsupported" : ""}" data-card-id="\${a.id}">
          <div class="tool-card-top">
            <div class="tool-logo-tile" style="background:\${tileBg};border-color:\${tileBorder};">
              <img src="\${glyphForTile(a)}" alt="" />
            </div>
            <div class="tool-identity" data-open="\${a.id}">
              <div class="tool-name-row">
                <strong class="tool-name">\${a.name}</strong>
                <span class="tool-ver mono">\${a.version}</span>
              </div>
              <div class="tool-path mono" title="\${a.configPath}">\${a.configPath}</div>
            </div>
            <button class="switch-pill \${wired ? "on" : ""}" data-toggle="\${a.id}" \${a.supported ? "" : "disabled"} type="button" aria-label="\${wired ? "断开" : "接入"} \${a.name}"><span class="switch-knob"></span></button>
          </div>

          <div class="tool-card-well" data-open="\${a.id}">
            <div class="tool-well-meta">
              <span>\${wired ? "新会话默认模型" : (a.supported ? "本地检测摘要" : "暂无专用模板")}</span>
              \${wellMetaRight}
            </div>
            <div class="tool-well-val\${wellValCls}">\${wellValText}</div>
            \${wellExtra}
          </div>

          <div class="tool-card-foot">
            <span class="tool-status-pill \${toneCls}"><span class="dot"></span>\${a.status}</span>
            <div class="tool-foot-actions">\${footActions}</div>
          </div>
        </article>
      \`;
    };

    const overviewHtml = \`
      <div class="list-overview">
        <div class="list-overview-left">
          <span class="list-overview-title">接入模型路由，为每个工具配置顺手的模型</span>
          <span class="list-gateway-badge mono"><span class="dot"></span>127.0.0.1:4141</span>
        </div>
        <div class="list-kpis">
          <span class="list-kpi-chip">已接入 <b>\${allWired.length}</b> / \${supportedTotal}</span>
          <span class="list-kpi-chip">今日路由 <b>\${totalRoutes}</b> 次</span>
          \${driftItem ? \`<button class="list-kpi-chip danger" id="list-drift-btn" type="button">⚠ \${driftItem.name} 配置漂移 ›</button>\` : ""}
        </div>
      </div>
    \`;

    const wiredSecHtml = wiredRows.length ? \`
      <section class="list-section">
        <div class="list-sec-head">
          <span class="list-sec-dot"></span>
          <span class="list-sec-title">已接入路由</span>
          <span class="list-sec-count">\${wiredRows.length}</span>
          <span class="list-sec-note">点击卡片可展开右侧设置面板</span>
        </div>
        <div class="tool-grid">\${wiredRows.map(renderToolCard).join("")}</div>
      </section>
    \` : "";

    const availSecHtml = availRows.length ? \`
      <section class="list-section">
        <div class="list-sec-head">
          <span class="list-sec-dot muted"></span>
          <span class="list-sec-title">可接入与扩展</span>
          <span class="list-sec-count muted">\${availRows.length}</span>
          <span class="list-sec-note">开启开关即可直连 :4141 · 或回 3D 星系拖曳入轨</span>
        </div>
        <div class="tool-grid">\${availRows.map(renderToolCard).join("")}</div>
      </section>
    \` : "";

    const emptyHtml = (!wiredRows.length && !availRows.length) ? \`
      <div class="list-empty">
        <div>\${q ? \`没有找到匹配「\${q}」的工具\` : "当前筛选分类下暂无工具"}</div>
        <button class="btn" id="list-clear-filter" type="button">重置搜索与筛选</button>
      </div>
    \` : "";

    const blockedHtml = blockedRows.length ? \`
      <section class="list-blocked \${listBlockedOpen ? "open" : ""}" id="list-blocked-sec">
        <button class="list-blocked-head" id="list-blocked-toggle" type="button">
          <span class="chev">▸</span>
          <span style="font-size:12.5px;font-weight:700;">暂不能自动接入</span>
          <span class="list-sec-count muted">\${blockedRows.length}</span>
          <span class="spacer"></span>
          <span id="list-blocked-hint" style="font-size:11px;color:var(--jn-text-3);">\${listBlockedOpen ? "收起" : "查看原因"}</span>
        </button>
        <div class="list-blocked-body">
          \${blockedRows.map((b) => {
            const bLogo = LOGOS[b.id] ? logoFor(b.id) : null;
            return \`
              <div class="blocked-mini-card">
                \${bLogo ? \`<img src="\${bLogo}" alt="" />\` : \`<span class="blocked-fallback">\${b.name.slice(0, 2).toUpperCase()}</span>\`}
                <div style="min-width:0;flex:1;">
                  <div style="font-size:12.5px;font-weight:700;color:var(--jn-text);">\${b.name}</div>
                  <div style="font-size:11px;color:var(--jn-text-3);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">\${b.reason}</div>
                </div>
              </div>
            \`;
          }).join("")}
        </div>
      </section>
    \` : "";

    const tipHtml = \`
      <div class="list-security-tip">
        <span>🛡</span>
        <span>开启前可点击「Diff」预览改动文件，断开后仅还原 Jolu 写入的字段、保留工具原登录与其它设置。</span>
      </div>
    \`;

    listView.innerHTML = overviewHtml + wiredSecHtml + availSecHtml + emptyHtml + blockedHtml + tipHtml;

    listView.querySelector("#list-drift-btn")?.addEventListener("click", () => { if (driftItem) focusAgent(driftItem.id); });
    listView.querySelector("#list-blocked-toggle")?.addEventListener("click", () => {
      listBlockedOpen = !listBlockedOpen;
      listView.querySelector("#list-blocked-sec")?.classList.toggle("open", listBlockedOpen);
      const hint = listView.querySelector("#list-blocked-hint");
      if (hint) hint.textContent = listBlockedOpen ? "收起" : "查看原因";
    });
    listView.querySelector("#list-clear-filter")?.addEventListener("click", () => {
      filter = "all"; searchQuery = ""; $("#search-input").value = "";
      $("#filter-group").querySelectorAll(".seg-btn").forEach((b) => b.classList.toggle("active", b.dataset.filter === "all"));
      renderListView();
    });

    listView.querySelectorAll("[data-toggle]").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const id = btn.dataset.toggle, a = agents.find((x) => x.id === id);
        if (!a?.supported) return;
        if (a.group === "wired") unwireAgent(id);
        else wireAgentFromOrigin(id, stage.clientWidth / 2, stage.clientHeight / 2, false);
      });
    });
    listView.querySelectorAll("[data-wire]").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        wireAgentFromOrigin(btn.dataset.wire, stage.clientWidth / 2, stage.clientHeight / 2, false);
      });
    });
    listView.querySelectorAll("[data-diff]").forEach((btn) => {
      btn.addEventListener("click", (e) => { e.stopPropagation(); openDiffModal(btn.dataset.diff); });
    });
    listView.querySelectorAll("[data-select], [data-open]").forEach((el) => {
      el.addEventListener("click", (e) => {
        e.stopPropagation();
        const id = el.dataset.select || el.dataset.open;
        const a = agents.find((x) => x.id === id);
        if (!a) return;
        if (a.group === "wired") focusAgent(selectedId === id ? null : id);
        else if (a.supported) openDiffModal(id);
        else toast("通用自定义接入", "Antigravity CLI 需点击右上「+ 添加工具」配置键名映射", false);
      });
    });
    listView.querySelectorAll("[data-fix]").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const a = agents.find((x) => x.id === btn.dataset.fix); if (!a) return;
        a.tone = "normal"; a.status = "已接入 · 配置一致"; a.model = "gemini-2.5-pro";
        if (selectedId === a.id) focusAgent(a.id, false);
        updateHudStats(); renderListView();
        toast("已修复 Gemini CLI 漂移", "已重新对齐 ~/.gemini/.env 网关地址");
      });
    });
    listView.querySelectorAll("[data-restart]").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const a = agents.find((x) => x.id === btn.dataset.restart); if (!a) return;
        a.tone = "normal"; a.status = "已接入 · 配置一致"; a.model = "gpt-5-codex";
        if (selectedId === a.id) focusAgent(a.id, false);
        updateHudStats(); renderListView();
        toast("Codex 已生效", "已检测到新会话走本机模型路由");
      });
    });
    listView.querySelectorAll("[data-add-custom]").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        toast("通用自定义接入", "正式版打开「添加自定义工具」弹窗配置 JSON/TOML/env 键位");
      });
    });
  }

  function openDiffModal(id) {
    const a = agents.find((x) => x.id === id); if (!a) return;
    $("#diff-logo").src = logoFor(a.id); $("#diff-title").textContent = \`\${a.name} · 写入改动\`;
    $("#diff-path").textContent = a.configPath;
    $("#diff-stats").textContent = \`+\${a.diff.filter((d) => d.op === "add").length} −\${a.diff.filter((d) => d.op === "del").length}\`;
    $("#diff-lines").innerHTML = a.diff.map((d) => \`<div class="diff-line \${d.op}"><span>\${d.text}</span></div>\`).join("");
    $("#diff-modal").classList.add("open");
  }

  function toast(title, sub, warn = false, actions = []) {
    const el = document.createElement("div"); el.className = "toast";
    el.innerHTML = \`<span class="toast-dot \${warn ? "warn" : ""}"></span><div class="toast-msg"><b>\${title}</b><span>\${sub}</span></div>\`;
    actions.forEach((act) => {
      const b = document.createElement("button"); b.className = "toast-link"; b.textContent = act.label;
      b.addEventListener("click", () => { act.onClick(); el.remove(); });
      el.appendChild(b);
    });
    $("#toast-host").appendChild(el);
    setTimeout(() => { el.style.opacity = "0"; el.style.transition = "opacity 180ms ease"; setTimeout(() => el.remove(), 200); }, 4200);
  }

  function applyCameraPreset(preset) {
    cam.preset = preset;
    document.querySelectorAll("[data-cam]").forEach((b) => b.classList.toggle("active", b.dataset.cam === preset));
    cam.vYaw = 0; cam.vPitch = 0;
    if (preset === "top") {
      cam.targetPitch = 1.18; cam.targetYaw = -0.15; cam.targetZoom = 0.86;
      focusAgent(null);
    } else if (preset === "core") {
      cam.targetPitch = 0.24; cam.targetYaw = 0.42; cam.targetZoom = 1.62;
      focusAgent(null);
    } else {
      cam.targetPitch = 0.44; cam.targetYaw = -0.32; cam.targetZoom = 1.0;
      focusAgent(null);
    }
  }

  function resetDemo() {
    resetAgentsState();
    coreBody.selfSpin = -0.32; coreBody.spinVel = 0; coreBody.pitchSpin = 0; coreBody.pitchVel = 0;
    coreBody.spinMul = 0.0; coreBody.galaxySpeed = 1.0;
    cam.orbitSpin = 0; focusAgent(null);
    ensureChaosOrbits(false);
    renderHangar(); renderListView();
    toast("已重置 3D 星系示例", "5 颗未接入小晶核已回到左下星坞");
  }

  $("#btn-collapse").addEventListener("click", () => $("#sidebar").classList.toggle("collapsed"));
  $("#btn-reset").addEventListener("click", resetDemo);
  $("#btn-theme").addEventListener("click", () => {
    dark = !dark;
    document.documentElement.classList.toggle("dark", dark);
    $("#btn-theme").textContent = dark ? "☀ 浅色星盘" : "🌙 深色天文台";
    renderHangar(); if (selectedId) focusAgent(selectedId, false); renderListView();
  });
  $("#btn-drift-jump").addEventListener("click", () => { const d = wiredAgents().find((a) => a.tone === "danger"); if (d) focusAgent(d.id); });
  $("#btn-blocked").addEventListener("click", () => toast("暂不能接入 3 项", BLOCKED_AGENTS.map((b) => \`\${b.name}: \${b.reason}\`).join(" · "), true));
  $("#btn-detect").addEventListener("click", () => toast("本地工具扫描完成", "已确认 11 个可管理工具 · 状态为最新"));
  $("#btn-add").addEventListener("click", () => toast("通用自定义接入", "正式版打开「添加自定义工具」弹窗配置 JSON/TOML/env 键位"));
  $("#hangar-toggle").addEventListener("click", () => { hangarCollapsed = !hangarCollapsed; renderHangar(); });
  $("#diff-close").addEventListener("click", () => $("#diff-modal").classList.remove("open"));
  $("#diff-ok").addEventListener("click", () => $("#diff-modal").classList.remove("open"));
  $("#diff-modal").addEventListener("click", (e) => { if (e.target.id === "diff-modal") $("#diff-modal").classList.remove("open"); });

  $("#btn-shuffle").addEventListener("click", () => {
    focusAgent(null);
    chaosSeed = (chaosSeed + 17) % 9973;
    cam.orbitSpin = 0;
    const prevPos = new Map();
    wiredAgents().forEach((a) => {
      const wp = getAgentWorldPos(a);
      prevPos.set(a.id, project3D(wp.x, wp.y, wp.z, 1));
    });
    ensureChaosOrbits(true);
    if (glRenderer) glRenderer.reseed(chaosSeed);
    wiredAgents().forEach((a) => {
      const oldP = prevPos.get(a.id);
      const wp = getAgentWorldPos(a);
      const newP = project3D(wp.x, wp.y, wp.z, 1);
      if (oldP) planetAnim.set(a.id, { dx: oldP.x - newP.x, dy: oldP.y - newP.y, vx: 0, vy: 0, scale: 1.08, vs: 0 });
    });
    toast("已重排星系相位", "11,200 颗星辰与各行星轨道已按新螺旋种子重散");
  });

  $("#mode-group").addEventListener("click", (e) => {
    const btn = e.target.closest("[data-mode]"); if (!btn) return;
    mode = btn.dataset.mode;
    $("#mode-group").querySelectorAll(".seg-btn").forEach((b) => b.classList.toggle("active", b === btn));
    const isList = mode === "list";
    stage.classList.toggle("is-list", isList);
    hangarCard.style.display = isList ? "none" : "flex";
    $(".right-hud").style.display = "flex";
    $(".bottom-right-hud").style.display = isList ? "none" : "flex";
    $("#btn-shuffle").style.display = isList ? "none" : "inline-flex";
    $("#btn-reset").style.display = isList ? "none" : "inline-flex";
    $("#filter-avail-label").textContent = isList ? "待接入" : "待接星坞";
    $("#stage-hint").style.display = isList ? "none" : "block";
    glCanvas.style.display = isList ? "none" : "block";
    overlayCanvas.style.display = isList ? "none" : "block";
    domLayer.style.display = isList ? "none" : "block";
    if (isList) {
      if (selectedId === "core") focusAgent(null);
      else if (selectedId) focusAgent(selectedId, false);
      else stage.classList.remove("list-has-inspect");
    } else {
      stage.classList.remove("list-has-inspect");
      if (selectedId) focusAgent(selectedId, false);
    }
    renderListView();
  });

  $("#filter-group").addEventListener("click", (e) => {
    const btn = e.target.closest("[data-filter]"); if (!btn) return;
    filter = btn.dataset.filter;
    $("#filter-group").querySelectorAll(".seg-btn").forEach((b) => b.classList.toggle("active", b === btn));
    if (filter === "available" && hangarCollapsed) { hangarCollapsed = false; renderHangar(); }
    renderListView();
  });
  $("#search-input").addEventListener("input", (e) => { searchQuery = e.target.value; renderListView(); });
  $("#search-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && searchQuery.trim() && mode === "3d") {
      const q = searchQuery.trim().toLowerCase();
      const match = wiredAgents().find((a) => a.name.toLowerCase().includes(q) || a.id.includes(q));
      if (match) focusAgent(match.id);
    }
  });

  // 3D 交互：左键惯性旋转、右键/Shift平移、滚轮缩放、点击星海复位聚焦
  stage.addEventListener("contextmenu", (e) => { if (mode === "3d") e.preventDefault(); });
  stage.addEventListener("pointerdown", (e) => {
    if (mode !== "3d") return;
    if (e.target.closest(".planet-node, .hangar-card, .top-hud, .right-hud, .bottom-right-hud, .jolu-core")) return;
    cam.dragMode = (e.button === 2 || e.button === 1 || e.shiftKey) ? "pan" : "orbit";
    cam.moved = false;
    cam.lastX = e.clientX; cam.lastY = e.clientY;
    cam.vYaw = 0; cam.vPitch = 0;
    stage.classList.toggle("is-rotating", cam.dragMode === "orbit");
    stage.classList.toggle("is-panning", cam.dragMode === "pan");
  });
  window.addEventListener("pointermove", (e) => {
    if (scrubState) {
      const dx = e.clientX - scrubState.lastX;
      const dy = e.clientY - scrubState.lastY;
      scrubState.lastX = e.clientX;
      scrubState.lastY = e.clientY;
      if (!scrubState.moved && Math.hypot(e.clientX - scrubState.startX, e.clientY - scrubState.startY) > 4.5) {
        scrubState.moved = true;
        scrubState.el.classList.add("scrubbing");
      }
      if (scrubState.moved) {
        if (scrubState.targetId === "core") {
          coreBody.selfSpin += dx * 0.026;
          coreBody.spinVel = dx * 1.55;
          coreBody.pitchSpin = (coreBody.pitchSpin ?? 0) + dy * 0.024;
          coreBody.pitchVel = dy * 1.45;
        } else {
          const tAgent = agents.find((x) => x.id === scrubState.targetId);
          if (tAgent) {
            tAgent.selfSpin = (tAgent.selfSpin ?? 0) + dx * 0.028;
            tAgent.spinVel = dx * 1.65;
            tAgent.pitchSpin = (tAgent.pitchSpin ?? 0) + dy * 0.024;
            tAgent.pitchVel = dy * 1.45;
          }
        }
      }
      return;
    }
    if (cam.dragMode) {
      const dx = e.clientX - cam.lastX, dy = e.clientY - cam.lastY;
      cam.lastX = e.clientX; cam.lastY = e.clientY;
      if (Math.hypot(dx, dy) > 1.5) cam.moved = true;
      if (cam.dragMode === "orbit") {
        // 360° × 360° 全向自由环绕，水平与垂直均无硬性夹角锁死（越过极点自动顺手翻转水平转向）
        const hemi = Math.cos(cam.pitch) >= 0 ? 1 : -1;
        const dyaw = dx * 0.0064 * hemi, dpitch = dy * 0.0058;
        cam.yaw += dyaw; cam.targetYaw = cam.yaw;
        cam.pitch += dpitch; cam.targetPitch = cam.pitch;
        cam.vYaw = dyaw * 0.88; cam.vPitch = dpitch * 0.88;
      } else {
        focusId = null;
        const speed = 1.05 / Math.max(0.45, cam.zoom);
        const cy = Math.cos(cam.yaw), sy = Math.sin(cam.yaw);
        const hemi = Math.cos(cam.pitch) >= 0 ? 1 : -1;
        cam.targetFx = Math.max(-340, Math.min(340, cam.targetFx + (-dx * cy + dy * hemi * sy * 0.85) * speed));
        cam.targetFz = Math.max(-340, Math.min(340, cam.targetFz + (dx * sy + dy * hemi * cy * 0.85) * speed));
        cam.fx = cam.targetFx; cam.fz = cam.targetFz;
      }
      return;
    }
    if (mode === "3d" && !dragState) {
      const sr = stage.getBoundingClientRect();
      const nx = (e.clientX - (sr.left + sr.width * 0.5)) / Math.max(1, sr.width * 0.5);
      const ny = (e.clientY - (sr.top + sr.height * 0.5)) / Math.max(1, sr.height * 0.5);
      cam.parallaxX = Math.max(-1, Math.min(1, nx)) * 0.048;
      cam.parallaxY = Math.max(-1, Math.min(1, ny)) * 0.035;
    } else {
      cam.parallaxX = 0; cam.parallaxY = 0;
    }
  });
  window.addEventListener("pointerup", () => {
    if (scrubState) {
      const s = scrubState;
      scrubState = null;
      s.el.classList.remove("scrubbing");
      lastPointerFocusTs = performance.now();
      if (!s.moved) {
        focusAgent(s.targetId);
      }
    }
    if (cam.dragMode) {
      const wasMoved = cam.moved;
      cam.dragMode = null;
      stage.classList.remove("is-rotating", "is-panning");
      if (!wasMoved) focusAgent(null);
    }
  });
  stage.addEventListener("dblclick", (e) => {
    if (mode !== "3d" || e.target.closest(".planet-node, .hangar-card, .top-hud, .right-hud, .bottom-right-hud")) return;
    applyCameraPreset("oblique");
  });
  stage.addEventListener("wheel", (e) => {
    if (mode !== "3d" || dragState) return;
    e.preventDefault();
    cam.targetZoom = Math.max(0.42, Math.min(2.45, cam.targetZoom * Math.exp(-e.deltaY * 0.0011)));
  }, { passive: false });

  $("#cam-pod").addEventListener("click", (e) => {
    const pBtn = e.target.closest("[data-cam]");
    if (pBtn) applyCameraPreset(pBtn.dataset.cam);
  });
  $("#btn-spin-toggle").addEventListener("click", () => {
    autoSpin = !autoSpin;
    $("#btn-spin-toggle").textContent = autoSpin ? "⏸" : "▶";
  });
  $("#btn-camera-reset").addEventListener("click", () => applyCameraPreset("oblique"));

  window.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault(); $("#search-input").focus();
    } else if (e.key === "Escape") {
      $("#diff-modal").classList.remove("open"); focusAgent(null);
    }
  });

  ensureChaosOrbits(false);
  renderHangar();
  requestAnimationFrame(frame);
})();
</script>
</body>
</html>
`

await mkdir(dirname(OUT), { recursive: true })
await writeFile(OUT, html, "utf8")
const { size } = await stat(OUT)
console.log(`[agent-galaxy-demo] ${OUT} · ${(size / 1024).toFixed(0)} KB`)
