import { clsx, type ClassValue } from "clsx"
import { extendTailwindMerge } from "tailwind-merge"

/**
 * index.css 的 @theme 定义了语义字号 `--text-note` / `--text-meta`。tailwind-merge 默认不认识它们，
 * 会把 `text-note` 当成文字颜色，和 `text-jolu-muted` 一起出现时把字号删掉——这里登记成字号。
 */
const twMerge = extendTailwindMerge({ extend: { theme: { text: ["note", "meta"] } } })

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
