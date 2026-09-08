import { mkdirSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

// Card-stock geometry, in the same world units as the tabletop. No external model.
const faces = [
  [[0, 1, 0], [[-.8, .0225, -1.08], [-.8, .0225, 1.08], [.8, .0225, 1.08], [.8, .0225, -1.08]]],
  [[0, -1, 0], [[-.8, -.0225, 1.08], [-.8, -.0225, -1.08], [.8, -.0225, -1.08], [.8, -.0225, 1.08]]],
  [[1, 0, 0], [[.8, -.0225, -1.08], [.8, .0225, -1.08], [.8, .0225, 1.08], [.8, -.0225, 1.08]]],
  [[-1, 0, 0], [[-.8, -.0225, 1.08], [-.8, .0225, 1.08], [-.8, .0225, -1.08], [-.8, -.0225, -1.08]]],
  [[0, 0, 1], [[.8, -.0225, 1.08], [.8, .0225, 1.08], [-.8, .0225, 1.08], [-.8, -.0225, 1.08]]],
  [[0, 0, -1], [[-.8, -.0225, -1.08], [-.8, .0225, -1.08], [.8, .0225, -1.08], [.8, -.0225, -1.08]]],
]
const positions = new Float32Array(faces.flatMap(([, vertices]) => vertices.flat()))
const normals = new Float32Array(faces.flatMap(([normal]) => Array.from({ length: 4 }, () => normal).flat()))
const indices = new Uint16Array(faces.flatMap((_, face) => [0, 1, 2, 0, 2, 3].map(index => face * 4 + index)))
const binary = Buffer.concat([Buffer.from(positions.buffer), Buffer.from(normals.buffer), Buffer.from(indices.buffer)])
const document = {
  asset: { version: '2.0', generator: 'Hogwarts table card-stock v1' },
  scene: 0, scenes: [{ nodes: [0] }], nodes: [{ name: 'card-stock', mesh: 0 }],
  meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1 }, indices: 2 }] }],
  buffers: [{ byteLength: binary.length }],
  bufferViews: [
    { buffer: 0, byteOffset: 0, byteLength: positions.byteLength, target: 34962 },
    { buffer: 0, byteOffset: positions.byteLength, byteLength: normals.byteLength, target: 34962 },
    { buffer: 0, byteOffset: positions.byteLength + normals.byteLength, byteLength: indices.byteLength, target: 34963 },
  ],
  accessors: [
    { bufferView: 0, componentType: 5126, count: 24, type: 'VEC3', min: [-.8, -.0225, -1.08], max: [.8, .0225, 1.08] },
    { bufferView: 1, componentType: 5126, count: 24, type: 'VEC3' },
    { bufferView: 2, componentType: 5123, count: 36, type: 'SCALAR' },
  ],
}
const json = Buffer.from(JSON.stringify(document).padEnd(Math.ceil(JSON.stringify(document).length / 4) * 4, ' '))
const header = Buffer.alloc(12)
header.writeUInt32LE(0x46546c67, 0)
header.writeUInt32LE(2, 4)
header.writeUInt32LE(12 + 8 + json.length + 8 + binary.length, 8)
function chunk(type, data) {
  const chunkHeader = Buffer.alloc(8)
  chunkHeader.writeUInt32LE(data.length)
  chunkHeader.writeUInt32LE(type, 4)
  return Buffer.concat([chunkHeader, data])
}
const directory = resolve(import.meta.dirname, '../apps/web/public/table-assets/v1')
mkdirSync(directory, { recursive: true })
writeFileSync(resolve(directory, 'card.glb'), Buffer.concat([header, chunk(0x4e4f534a, json), chunk(0x004e4942, binary)]))
