/**
 * 生成启动页的独立预览文件：把 startup.html + startup.css + 图标合成一个 HTML，
 * 去掉 Tauri 调用，双击即可看到真实动效（含重播）。
 *
 * 用途：不启动桌面应用、不开 dev server 也能看启动页。
 * 真实的启动窗口尺寸与「后端就绪即关闭」行为仍需 `pnpm tauri dev` 验证。
 *
 * 用法：node scripts/dev/build-startup-preview.mjs
 * 产物：output/startup-preview.html（已被 Git 忽略）
 */
import { readFile, writeFile, stat } from "node:fs/promises"
import { resolve, dirname } from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const OUT = resolve(ROOT, "output/startup-preview.html")

const html = await readFile(resolve(ROOT, "frontend/startup.html"), "utf8")
const css = await readFile(resolve(ROOT, "frontend/src/startup.css"), "utf8")
const icon = await readFile(resolve(ROOT, "frontend/public/app-icon.png"))
const iconUrl = `data:image/png;base64,${icon.toString("base64")}`

/** Inter 也内联：file:// 下 /fonts/ 会指向盘符根目录 */
const inter = await readFile(resolve(ROOT, "frontend/public/fonts/Inter-Variable.woff2"))
const interUrl = `data:font/woff2;base64,${inter.toString("base64")}`

/** 预览专用样式：把窗口画成一块 1280×720 的卡片，方便看真实边界与投影 */
const previewCss = `
/* ── 预览外壳（只在这个文件里存在，不进产品） ── */
html.preview, html.preview body { background: #e9eaec; }
html.preview[data-theme="dark"], html.preview[data-theme="dark"] body { background: #17181b; }
html.preview body {
  display: grid;
  height: 100vh;
  padding: 28px;
  place-items: center;
  overflow: hidden;
}
html.preview .startup {
  height: 720px;
  width: 1280px;
  max-height: 100%;
  max-width: 100%;
  box-shadow: 0 20px 60px rgba(0, 0, 0, .18);
}
html.preview[data-theme="dark"] .startup { box-shadow: 0 20px 60px rgba(0, 0, 0, .55); }
html.preview .preview-hint {
  position: fixed;
  right: 18px;
  bottom: 14px;
  color: #6b6b70;
  font: 12px/1.5 "Segoe UI", "Microsoft YaHei", sans-serif;
  opacity: .8;
  pointer-events: none;
}
`

/** 预览脚本：设主题、进就绪态、点击重播（替代 startup.ts 里的 Tauri 调用） */
const previewScript = `
const root = document.documentElement
root.classList.add('preview')
const theme = new URLSearchParams(location.search).get('theme')
root.dataset.theme = theme === 'dark' ? 'dark' : theme === 'system' ? 'system' : 'light'
// 真实启动页由 startup.ts 在首帧打上 .ready；这里直接进入
requestAnimationFrame(() => root.classList.add('ready'))
const replay = () => {
  root.classList.remove('ready')
  for (const a of document.getAnimations()) { try { a.cancel() } catch {} }
  requestAnimationFrame(() => requestAnimationFrame(() => {
    root.classList.add('ready')
    for (const a of document.getAnimations()) { try { a.play() } catch {} }
  }))
}
document.addEventListener('click', replay)
window.addEventListener('keydown', (event) => {
  if (event.key === 'r' || event.key === 'R' || event.key === ' ') { event.preventDefault(); replay() }
})
`

let out = html
  .replace('<link rel="stylesheet" href="/src/startup.css" />', `<style>${css}${previewCss}</style>`)
  .replace('<script type="module" src="/src/startup.ts"></script>', `<script>${previewScript}</script>`)
  .replaceAll('src="/app-icon.png"', `src="${iconUrl}"`)
  .replaceAll('url("/fonts/Inter-Variable.woff2")', `url("${interUrl}")`)
  .replace("</body>", `  <p class="preview-hint">点击任意处，或按 R / 空格重播</p>\n  </body>`)

await writeFile(OUT, out, "utf8")
const { size } = await stat(OUT)
console.log(`[startup-preview] ${OUT} · ${(size / 1024).toFixed(0)} KB`)
console.log("可选主题：?theme=light / ?theme=dark / ?theme=system")
