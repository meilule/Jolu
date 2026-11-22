// 把 vite 产出的 HTML / JS / CSS 合成一个可以直接双击打开的单文件。
// 不引入新依赖：只做「读文件 → 内联 → 写文件」。
//
// 除 <script> / <link> 外还要处理两类漏网资源：
//   1. CSS 里的 `url(./fonts/x.woff2)`——来自 `public/`，Vite 只拷贝不内联；
//   2. JS 里的 `"/app-icon.png"`——业务代码写的绝对路径，打包器不会改。
// 这两类在 file:// 下都会 404（后者甚至会指向盘符根目录），因此一并换成 data URL。
import { readFile, writeFile, stat, readdir } from "node:fs/promises"
import { resolve, dirname, extname, join, relative } from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const DIST = resolve(ROOT, "frontend/snapshot-dist")
const OUT = resolve(ROOT, "output/jolu-ui-snapshot.html")

const MIME = {
  ".woff2": "font/woff2",
  ".woff": "font/woff",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".webp": "image/webp",
  ".gif": "image/gif",
}

/** DIST 下除 html/css/js 之外的所有文件：文件名 → data URL */
async function assetTable() {
  const table = new Map()
  async function walk(dir) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name)
      if (entry.isDirectory()) { await walk(full); continue }
      const ext = extname(entry.name).toLowerCase()
      const mime = MIME[ext]
      if (!mime) continue
      const data = await readFile(full)
      table.set(relative(DIST, full).replaceAll("\\", "/"), `data:${mime};base64,${data.toString("base64")}`)
    }
  }
  await walk(DIST)
  return table
}

const assets = await assetTable()

/** 把文本里对某个资源的引用换成 data URL（同时覆盖 `./x`、`x`、`/x` 三种写法） */
function inlineRefs(text) {
  let result = text
  for (const [path, dataUrl] of assets) {
    const escaped = path.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
    // CSS url(./fonts/x.woff2) 与 JS 的 "/app-icon.png"、"app-icon.png"
    result = result.replace(new RegExp(`url\\((["']?)(?:\\./)?${escaped}\\1\\)`, "g"), `url("${dataUrl}")`)
    result = result.replace(new RegExp(`(["'\`])/(${escaped})\\1`, "g"), (_, quote) => `${quote}${dataUrl}${quote}`)
    result = result.replace(new RegExp(`(["'\`])\\./${escaped}\\1`, "g"), (_, quote) => `${quote}${dataUrl}${quote}`)
  }
  return result
}

const html = await readFile(resolve(DIST, "snapshot.html"), "utf8")

async function replaceAsync(input, pattern, replacer) {
  const jobs = []
  input.replace(pattern, (match, ...groups) => { jobs.push(replacer(match, ...groups)); return match })
  const values = await Promise.all(jobs)
  let index = 0
  return input.replace(pattern, () => values[index++])
}

async function inline(html) {
  let result = html
  // <script type="module" crossorigin src="./snapshot.js"></script>
  result = await replaceAsync(result, /<script[^>]*src="([^"]+)"[^>]*><\/script>/g, async (match, src) => {
    const code = await readFile(resolve(DIST, decodeURIComponent(src).replace(/^\.\//, "")), "utf8")
    // 防止 `</script>` 提前关闭标签
    return `<script type="module">${inlineRefs(code).replace(/<\/script>/gi, "<\\/script>")}</script>`
  })
  // <link rel="stylesheet" crossorigin href="./snapshot.css">
  result = await replaceAsync(result, /<link[^>]*rel="stylesheet"[^>]*href="([^"]+)"[^>]*>/g, async (match, href) => {
    const css = await readFile(resolve(DIST, decodeURIComponent(href).replace(/^\.\//, "")), "utf8")
    return `<style>${inlineRefs(css)}</style>`
  })
  return inlineRefs(result)
}

const single = await inline(html)
await writeFile(OUT, single, "utf8")
const { size } = await stat(OUT)
const leftover = [...assets.keys()].filter((path) => single.includes(`"${path}"`) || single.includes(`/${path}"`))
console.log(`[snapshot] ${OUT} · ${(size / 1024 / 1024).toFixed(2)} MB · 内联资源 ${assets.size} 个`)
if (leftover.length) console.warn(`[snapshot] 仍有未内联的引用：${leftover.join(", ")}`)
