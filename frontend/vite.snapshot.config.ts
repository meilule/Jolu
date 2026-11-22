/**
 * 静态快照的构建配置（与主应用共用源码，但产物是单个可直接双击打开的 HTML）。
 *
 * 与 `vite.config.ts` 的差别：
 * - 入口只有 `snapshot.html`，输出到 `snapshot-dist/`，不污染桌面应用的 dist；
 * - 关掉代码分割与 modulePreload，JS / CSS 各只产出一个文件；
 * - `assetsInlineLimit` 放到最大，字体与图片内联成 data URL；
 * - 真正的「合成单文件」由 `scripts/dev/build-snapshot.mjs` 在构建后完成（不引入额外依赖）。
 *
 * 用法：`pnpm --dir frontend snapshot`
 */
import path from "node:path"
import { defineConfig } from "vite"
import react from "@vitejs/plugin-react"
import tailwindcss from "@tailwindcss/vite"

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  base: "./",
  build: {
    outDir: "snapshot-dist",
    emptyOutDir: true,
    assetsInlineLimit: Number.MAX_SAFE_INTEGER,
    cssCodeSplit: false,
    modulePreload: false,
    rollupOptions: {
      input: { snapshot: path.resolve(__dirname, "snapshot.html") },
      output: {
        inlineDynamicImports: true,
        entryFileNames: "snapshot.js",
        assetFileNames: "snapshot.[ext]",
      },
    },
  },
})
