import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { cardArt } from '../apps/web/src/presentation/tableArt.ts'

const root = resolve(import.meta.dirname, '..')
const publicRoot = resolve(root, 'apps/web/public')
const entries = JSON.parse(readFileSync(resolve(root, 'content/bundles/game-one-en-v1.json'))).entries
const localized = JSON.parse(readFileSync(resolve(root, 'content/locales/game-one.pt-BR.v1.json')))
function resource(url) {
  const bytes = readFileSync(resolve(publicRoot, url.slice(1)))
  return { url, bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') }
}
function dimensions(bytes) {
  if (bytes.toString('ascii', 1, 4) === 'PNG') return [bytes.readUInt32BE(16), bytes.readUInt32BE(20)]
  if (bytes.readUInt16BE(0) === 0xffd8) {
    let offset = 2
    while (offset + 9 < bytes.length) {
      if (bytes[offset] !== 0xff) break
      const marker = bytes[offset + 1]
      const size = bytes.readUInt16BE(offset + 2)
      if ([0xc0, 0xc1, 0xc2].includes(marker)) return [bytes.readUInt16BE(offset + 7), bytes.readUInt16BE(offset + 5)]
      offset += size + 2
    }
  }
  throw new Error('Unsupported image metadata')
}
const artwork = { ...cardArt,
  'hero:001': { file: 'heroes.jpg', crop: [118, 122, 80, 84] },
  'hero:004': { file: 'heroes.jpg', crop: [355, 120, 80, 84] },
  'hero:007': { file: 'heroes.jpg', crop: [505, 24, 80, 84] },
  'hero:010': { file: 'heroes.jpg', crop: [263, 26, 80, 84] },
}
const manifest = {
  version: 'game-one-visual-v1', locale: 'pt-BR',
  content: 'game-one-en-v1',
  budgets: { decodedImages: 16, textureBytes: 64 * 1024 * 1024, essentialTransferBytes: 8 * 1024 * 1024 },
  models: { card: { ...resource('/table-assets/v1/card.glb'), format: 'glb', vertices: 24, triangles: 12 } },
  materials: {
    wood: { color: '#38251e', compressed: resource('/table-assets/v1/wood.ktx2'), fallback: resource('/table-art/wood.png') },
    back: { color: '#173042', compressed: resource('/table-assets/v1/card-back.ktx2'), fallback: resource('/table-art/card-back.png') },
    stock: { color: '#b6a17b' },
  },
  audio: Object.fromEntries(['card', 'damage', 'heal', 'resource', 'victory', 'defeat', 'ambient'].map(id => [id, resource(`/table-audio/v1/${id}.wav`)])),
  entries: Object.fromEntries(entries.map(entry => {
    const art = artwork[entry.id]
    return [entry.id, {
      name: localized[entry.id]['pt-BR'],
      art: art ? { ...resource(`/table-art/${art.file}`), resolution: dimensions(readFileSync(resolve(publicRoot, 'table-art', art.file))),
        ...(art.crop ? { crop: art.crop } : {}), status: 'provisional', provenance: '/table-art/sources.json' } : null,
      model: 'card', material: 'stock', audio: entry.kind === 'villain' ? 'damage' : 'card',
      fallback: 'identity-and-text',
    }]
  })),
}
const serialized = JSON.stringify(manifest, null, 2) + '\n'
const output = resolve(publicRoot, 'table-assets/v1/manifest.json')
if (process.argv.includes('--check')) {
  if (readFileSync(output, 'utf8') !== serialized) throw new Error('Visual manifest is stale or an asset changed without regeneration')
} else writeFileSync(output, serialized)
