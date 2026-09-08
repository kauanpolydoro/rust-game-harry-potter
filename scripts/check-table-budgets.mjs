import { readFile, readdir } from 'node:fs/promises'
import { gzipSync } from 'node:zlib'
import { resolve } from 'node:path'

const root = resolve(import.meta.dirname, '../apps/web/dist')
const manifest = JSON.parse(await readFile(resolve(root, '.vite/manifest.json'), 'utf8'))
const initial = new Set(['index.html'])
const visited = new Set()
function include(key) {
  if (visited.has(key)) return
  visited.add(key)
  const chunk = manifest[key]
  initial.add(chunk.file)
  for (const file of [...chunk.css ?? [], ...chunk.assets ?? []]) initial.add(file)
  for (const imported of chunk.imports ?? []) include(imported)
}
include('index.html')
const files = await readdir(root, { recursive: true, withFileTypes: true })
const sizes = new Map()
for (const file of files.filter(file => file.isFile())) {
  const path = resolve(file.parentPath, file.name)
  const relative = path.slice(root.length + 1)
  const bytes = await readFile(path)
  sizes.set(relative, { raw: bytes.length, compressed: /\.(js|css|html|json)$/.test(file.name) ? gzipSync(bytes).length : bytes.length })
  if (relative.endsWith('.css') && initial.has(relative)) {
    for (const match of bytes.toString().matchAll(/url\(["']?\/([^"')]+)["']?\)/g)) initial.add(match[1])
  }
}
const sum = (paths, compressed = true) => [...paths].reduce((total, file) => {
  const size = sizes.get(file)
  if (!size) throw new Error(`Missing build resource: ${file}`)
  return total + (compressed ? size.compressed : size.raw)
}, 0)
const initialJs = [...initial].filter(file => file.endsWith('.js'))
const allJs = [...sizes.keys()].filter(file => file.endsWith('.js'))
const allCss = [...sizes.keys()].filter(file => file.endsWith('.css'))
const report = {
  shellJsGzipBytes: sum(initialJs),
  cssGzipBytes: sum(allCss),
  shellTransferBytes: sum(initial),
  additionalJsGzipBytes: sum(allJs.filter(file => !initial.has(file))),
  // Conservative: includes prototype, all fonts, all art, audio, fallbacks and
  // unused lazy chunks. This upper bound also covers any visible card subset.
  completeBuildTransferBytes: sum(sizes.keys()),
}
const limits = { shellJsGzipBytes: 200 * 1024, cssGzipBytes: 50 * 1024, shellTransferBytes: 1024 ** 2,
  additionalJsGzipBytes: 800 * 1024, completeBuildTransferBytes: 8 * 1024 ** 2 }
console.log(JSON.stringify({ measurements: report, limits }, null, 2))
for (const [key, limit] of Object.entries(limits)) {
  if (report[key] > limit) throw new Error(`${key}: ${report[key]} exceeds ${limit}`)
}
