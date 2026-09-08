import manifest from '../../public/table-assets/v1/manifest.json'
import { cardArt, type CardArt } from './tableArt'

interface VisualEntry {
  name: string
  art: { url: string; resolution: number[]; crop?: number[]; status: string } | null
  model: string
  material: string
  audio: string
  fallback: string
}
export const visualManifest = manifest
export function visualArt(id: string): CardArt | undefined {
  const entry: VisualEntry | undefined = (manifest.entries as Record<string, VisualEntry>)[id]
  if (!entry) return cardArt[id]
  if (!entry.art) return undefined
  const [x, y, width, height] = entry.art.crop ?? []
  return { file: entry.art.url.replace('/table-art/', ''),
    ...(x === undefined || y === undefined || width === undefined || height === undefined ? {} : { crop: [x, y, width, height] as [number, number, number, number] }) }
}
