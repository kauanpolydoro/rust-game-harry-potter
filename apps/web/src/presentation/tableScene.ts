import { Engine } from '@babylonjs/core/Engines/engine'
import { Scene } from '@babylonjs/core/scene'
import { FreeCamera } from '@babylonjs/core/Cameras/freeCamera'
import { Camera } from '@babylonjs/core/Cameras/camera'
import { Vector3, Matrix } from '@babylonjs/core/Maths/math.vector'
import { Color3, Color4 } from '@babylonjs/core/Maths/math.color'
import { HemisphericLight } from '@babylonjs/core/Lights/hemisphericLight'
import { DirectionalLight } from '@babylonjs/core/Lights/directionalLight'
import { StandardMaterial } from '@babylonjs/core/Materials/standardMaterial'
import { DynamicTexture } from '@babylonjs/core/Materials/Textures/dynamicTexture'
import { Texture } from '@babylonjs/core/Materials/Textures/texture'
import { CreateBox } from '@babylonjs/core/Meshes/Builders/boxBuilder'
import { CreateGround } from '@babylonjs/core/Meshes/Builders/groundBuilder'
import { CreateCylinder } from '@babylonjs/core/Meshes/Builders/cylinderBuilder'
import { CreateSphere } from '@babylonjs/core/Meshes/Builders/sphereBuilder'
import { TransformNode } from '@babylonjs/core/Meshes/transformNode'
import { Mesh } from '@babylonjs/core/Meshes/mesh'
import type { Geometry } from '@babylonjs/core/Meshes/geometry'
import type { AssetContainer } from '@babylonjs/core/assetContainer'
import { LoadAssetContainerAsync } from '@babylonjs/core/Loading/sceneLoader'
import '@babylonjs/loaders/glTF/2.0/glTFLoader'
import type { GameProjectionResponse } from '../contracts/identity-access.generated'
import type { TableCard } from './tablePresentation'
import { visualArt, visualManifest } from './visualManifest'
import type { TableCue, TableVisualGroup } from './tableTimeline'
import { createTableQuality, qualityProfiles, type QualityPreference, type TableQuality } from './tableQuality'
import { acquireTableTextureDecoder } from './tableTextureDecoder'
import '@babylonjs/core/Culling/ray'
import { createDistribution } from './tableTelemetry'

export interface CardAnchor { id: string; x: number; y: number; width: number; height: number }
export interface TableMeasurements { frames: number; medianFrameMs: number; p95FrameMs: number; meshes: number; textures: number; quality: TableQuality; estimatedTextureBytes: number; compressedMaterials: number; loadedCardModel: boolean; decodedImages: number; frameDistribution: ReturnType<ReturnType<typeof createDistribution>['read']> }
export interface TableScene {
  update(game: GameProjectionResponse, cards: TableCard[], selectedId: string | null, handPage: number, playPage: number): void
  resize(): void
  pause(value: boolean): void
  present(group: TableVisualGroup, reducedMotion: boolean): void
  discardMotion(): void
  setQuality(value: QualityPreference): void
  resetMeasurements(): void
  measurements(): TableMeasurements
  dispose(): void
}

const colors = { hand: '#263e51', play: '#263e51', market: '#315b50', villain: '#603c58', location: '#4b5666', dark_arts: '#352c43' }
const labels = { hand: 'HOGWARTS', play: 'EM JOGO', market: 'HOGWARTS', villain: 'VILÃO', location: 'LOCAL', dark_arts: 'ARTES DAS TREVAS' }

/** Owns every GPU resource; no scene objects enter Vue's reactive graph. */
export function mountTableScene(canvas: HTMLCanvasElement, anchors: (items: CardAnchor[]) => void,
  qualityChanged: (quality: TableQuality) => void = () => {}, frameDistribution = createDistribution()) : TableScene {
  const engine = new Engine(canvas, true, { stencil: false, preserveDrawingBuffer: false, powerPreference: 'low-power' })
  engine.setHardwareScalingLevel(1 / Math.min(window.devicePixelRatio || 1, 1.5))
  const scene = new Scene(engine)
  const releaseDecoder = acquireTableTextureDecoder()
  let modelContainer: AssetContainer | null = null
  let cardGeometry: Geometry | null = null
  const quality = createTableQuality()
  quality.resetWindow(performance.now())
  let appliedQuality: TableQuality = 'balanced'
  scene.clearColor = Color4.FromHexString('#0b111aff')
  const camera = new FreeCamera('table-camera', new Vector3(0, 14, -12), scene)
  camera.setTarget(new Vector3(0, 0, 0))
  camera.fovMode = Camera.FOVMODE_HORIZONTAL_FIXED
  camera.fov = 1.13
  camera.minZ = 0.1
  const light = new HemisphericLight('room-light', new Vector3(0, 1, 0), scene)
  light.intensity = 0.85
  light.groundColor = Color3.FromHexString('#161b27')
  const candlelight = new DirectionalLight('candlelight', new Vector3(-0.3, -1, 0.4), scene)
  candlelight.diffuse = Color3.FromHexString('#ffd3a0')
  candlelight.intensity = 0.55

  function material(name: string, color: string) {
    const value = new StandardMaterial(name, scene)
    value.diffuseColor = Color3.FromHexString(color)
    value.specularColor = new Color3(0.12, 0.10, 0.06)
    return value
  }
  const wood = material('dark-wood', '#38251e')
  function compressedTexture(target: StandardMaterial, url: string, fallback: string) {
    let settled = false
    const fallbackTexture = () => {
      if (settled || scene.isDisposed) return
      settled = true
      clearTimeout(timeout)
      texture.dispose()
      target.diffuseTexture = new Texture(fallback, scene)
    }
    // A silent worker/network failure must not leave a material pending forever.
    const timeout = setTimeout(fallbackTexture, 8000)
    const texture = new Texture(url, scene, false, true, Texture.TRILINEAR_SAMPLINGMODE,
      () => { settled = true; clearTimeout(timeout) }, fallbackTexture)
    scene.onDisposeObservable.addOnce(() => clearTimeout(timeout))
    target.diffuseTexture = texture
  }
  compressedTexture(wood, visualManifest.materials.wood.compressed.url, visualManifest.materials.wood.fallback.url)
  const leather = material('blue-leather', '#162732')
  leather.diffuseTexture = new Texture('/textures/stage-panel.jpg', scene)
  const brass = material('aged-brass', '#b59150')
  const edge = material('card-stock', '#b6a17b')
  const back = material('card-back', '#173042')
  compressedTexture(back, visualManifest.materials.back.compressed.url, visualManifest.materials.back.fallback.url)
  const shadow = material('contact-shadow', '#080e14')
  shadow.alpha = 0.5
  const table = CreateBox('table', { width: 25, depth: 14, height: 0.5 }, scene)
  table.position.y = -0.44
  table.material = wood
  const board = CreateBox('board', { width: 21.9, depth: 11.4, height: 0.15 }, scene)
  board.position.y = -0.1
  board.material = leather
  const wax = material('wax', '#ded0a6')
  const flame = material('flame', '#ffce70')
  flame.emissiveColor = Color3.FromHexString('#ffb747')
  for (const x of [-10, 10]) {
    const base = CreateCylinder('candle-base', { diameter: 0.65, height: 0.12, tessellation: 16 }, scene)
    base.position.set(x, 0.14, 4.6)
    base.material = brass
    const candle = CreateCylinder('candle', { diameter: 0.27, height: 0.9, tessellation: 12 }, scene)
    candle.position.set(x, 0.62, 4.6)
    candle.material = wax
    const wick = CreateSphere('candle-flame', { diameter: 0.18, segments: 6 }, scene)
    wick.scaling.y = 1.6
    wick.position.set(x, 1.18, 4.6)
    wick.material = flame
  }
  for (const x of [-10.95, 10.95]) {
    const trim = CreateBox('board-trim', { width: 0.05, depth: 11.45, height: 0.08 }, scene)
    trim.position.set(x, 0, 0)
    trim.material = brass
  }
  for (const z of [-5.72, 5.72]) {
    const trim = CreateBox('board-trim', { width: 22, depth: 0.05, height: 0.08 }, scene)
    trim.position.set(0, 0, z)
    trim.material = brass
  }

  const objects = new Map<string, { root: TransformNode; card: TableCard; texture: DynamicTexture; face: StandardMaterial }>()
  const markers: Mesh[] = []
  let selected: string | null = null
  let stopped = false
  let disposed = false
  let needsAnchors = true
  const intervals: number[] = []
  let lastFrame = 0
  const images = new Map<string, { image: HTMLImageElement; abort: AbortController }>()
  // The table owns its materials; skip the loader's unused PBR/embedded BRDF.
  void LoadAssetContainerAsync(visualManifest.models.card.url, scene, {
    pluginOptions: { gltf: { skipMaterials: true } },
  }).then(container => {
    if (disposed) { container.dispose(); return }
    modelContainer = container
    const source = container.meshes.find(mesh => mesh instanceof Mesh && mesh.geometry)
    if (!(source instanceof Mesh) || !source.geometry) return
    cardGeometry = source.geometry
    for (const object of objects.values()) {
      const stock = object.root.getChildMeshes().find(mesh => mesh.name.startsWith('stock-'))
      if (stock instanceof Mesh) cardGeometry.applyToMesh(stock)
    }
  }).catch(() => { /* Procedural card stock preserves identity and interaction. */ })
  let markerVersion = -1
  let markerGame = ''
  const sparks = { damage: material('damage-feedback', '#ffc098'), heal: material('healing-feedback', '#acd8ad'),
    resource: material('resource-feedback', '#dfc58e') }
  for (const value of Object.values(sparks)) value.emissiveColor = value.diffuseColor
  const motions: Array<{ mesh: Mesh; from: Vector3; to: Vector3; start: number; duration: number; arc: number }> = []
  function applyQuality() {
    appliedQuality = quality.current()
    const profile = qualityProfiles[appliedQuality]
    engine.setHardwareScalingLevel(1 / Math.min(window.devicePixelRatio || 1, profile.pixelRatio))
    shadow.alpha = profile.shadows ? 0.5 : 0
    qualityChanged(appliedQuality)
    needsAnchors = true
  }

  function zonePoint(zone: string | undefined): Vector3 {
    const points: Record<string, number[]> = {
      hero_hand: [-1.1, 0.3, -4.2], hero_play_area: [-1, 0.3, -0.55], hero_draw_pile: [-8.8, 0.3, -1.45],
      hero_discard_pile: [-8.8, 0.3, -3.55], market: [7.5, 0.3, 2], hogwarts_deck: [9.8, 0.3, -1.45],
      active_villains: [0, 0.3, 2.45], villain_deck: [9.8, 0.3, -3.55], villain_discard: [3, 0.3, 4.6],
      active_location: [-6.2, 0.3, 2.55], location_discard: [-8, 0.3, 4.6], dark_arts_deck: [-3.75, 0.3, 4.8],
      dark_arts_discard: [-3.75, 0.3, 2.55],
    }
    return Vector3.FromArray(points[zone ?? ''] ?? [0, 0.4, 0])
  }
  function cuePoint(cue: TableCue): Vector3 {
    const object = objects.get(cue.target) ?? (cue.resource === 'control' ? objects.get('current-location') : undefined)
    return object?.root.position.clone() ?? (cue.position ? new Vector3((cue.position - 2.5) * 3, 0.5, -4.9) : zonePoint(cue.to))
  }
  function discardMotion() {
    for (const motion of motions) motion.mesh.dispose()
    motions.length = 0
  }

  function faceTexture(card: TableCard) {
    const texture = new DynamicTexture(`face-${card.id}`, { width: 384, height: 512 }, scene, false)
    const ctx = texture.getContext() as CanvasRenderingContext2D
    ctx.fillStyle = '#e8deca'
    ctx.fillRect(0, 0, 384, 512)
    ctx.fillStyle = colors[card.zone]
    ctx.fillRect(12, 12, 360, 350)
    ctx.strokeStyle = '#c8ab6d'
    ctx.lineWidth = 3
    ctx.strokeRect(22, 22, 340, 330)
    ctx.textAlign = 'center'
    ctx.fillStyle = '#e8deca'
    ctx.font = '600 18px sans-serif'
    ctx.fillText(labels[card.zone], 192, 54)
    // Typographic identity is deliberately provisional until the asset delivery in #30.
    ctx.font = '110px Georgia, serif'
    ctx.fillText(card.name.split(' ').map((part) => part[0]).slice(0, 2).join(''), 192, 233)
    ctx.font = '600 20px sans-serif'
    ctx.fillText(card.health !== undefined ? `VIDA ${card.health}` : card.cost !== undefined ? `CUSTO ${card.cost}` : 'BATALHA DE HOGWARTS', 192, 316)
    ctx.fillStyle = '#1a2630'
    ctx.font = 'bold 26px sans-serif'
    const words = card.name.split(' ')
    let line = '', y = 402
    for (const word of words) {
      if (ctx.measureText(`${line} ${word}`).width > 325 && line) {
        ctx.fillText(line, 192, y); y += 32; line = word
      } else line = line ? `${line} ${word}` : word
    }
    ctx.fillText(line, 192, y)
    texture.update()
    const art = card.catalogId ? visualArt(card.catalogId) : undefined
    if (art) {
      let cached = images.get(art.file)
      if (!cached) {
        cached = { image: new Image(), abort: new AbortController() }
        cached.image.src = `/table-art/${art.file}`
      } else images.delete(art.file)
      images.set(art.file, cached)
      while (images.size > visualManifest.budgets.decodedImages) {
        const [key, evicted] = images.entries().next().value!
        evicted.abort.abort()
        evicted.image.src = ''
        images.delete(key)
      }
      const source = cached.image
      const paint = () => {
        if (disposed || objects.get(card.id)?.texture !== texture || !source.naturalWidth) return
        const [x, y, width, height] = art.crop ?? [0, 0, source.naturalWidth, source.naturalHeight]
        ctx.drawImage(source, x, y, width, height, 26, 72, 332, 215)
        texture.update()
      }
      if (source.complete) queueMicrotask(paint)
      else source.addEventListener('load', paint, { once: true, signal: cached.abort.signal })
    }
    return texture
  }

  function addCard(card: TableCard) {
    const root = new TransformNode(card.id, scene)
    const body = CreateBox(`stock-${card.id}`, { width: 1.6, depth: 2.16, height: 0.045 }, scene)
    cardGeometry?.applyToMesh(body)
    body.parent = root
    body.material = edge
    const underside = CreateGround(`back-${card.id}`, { width: 1.59, height: 2.15 }, scene)
    underside.parent = root
    underside.position.y = -0.025
    underside.rotation.x = Math.PI
    underside.material = back
    const top = CreateGround(`front-${card.id}`, { width: 1.56, height: 2.12 }, scene)
    top.parent = root
    top.position.y = 0.026
    const texture = faceTexture(card)
    const face = material(`ink-${card.id}`, '#ffffff')
    face.diffuseTexture = texture
    face.emissiveColor = new Color3(0.33, 0.33, 0.33)
    top.material = face
    const contact = CreateGround(`shadow-${card.id}`, { width: 1.69, height: 2.25 }, scene)
    contact.parent = root
    contact.position.set(0.05, -0.028, -0.05)
    contact.material = shadow
    const object = { root, card, texture, face }
    objects.set(card.id, object)
    return object
  }

  function place(cards: TableCard[], page: number, playPage: number) {
    const counts: Record<string, number> = {}
    const hand = cards.filter((card) => card.zone === 'hand').slice(page * 7, page * 7 + 7)
    const played = cards.filter((card) => card.zone === 'play').slice(playPage * 4, playPage * 4 + 4)
    for (const card of cards) {
      const object = objects.get(card.id) ?? addCard(card)
      const index = counts[card.zone] ?? 0
      counts[card.zone] = index + 1
      const { root } = object
      root.setEnabled(true)
      root.rotation.set(0, 0, 0)
      root.scaling.setAll(1)
      switch (card.zone) {
        case 'hand': {
          const handIndex = hand.findIndex((item) => item.id === card.id)
          root.setEnabled(handIndex >= 0)
          const offset = handIndex - (hand.length - 1) / 2
          root.position.set(offset * 1.65 - 1.1, 0.18 + handIndex * 0.018, -4.2 + offset * offset * 0.06)
          root.rotation.y = -offset * 0.075
          break
        }
        case 'market': root.position.set(5.6 + index % 3 * 1.92, 0.08, 3.8 - Math.floor(index / 3) * 3.8); root.scaling.setAll(0.95); break
        case 'villain': root.position.set(-0.2 + index * 1.9, 0.12, 2.45); root.scaling.setAll(1.1); break
        case 'location': root.position.set(-6.2, 0.1, 2.55); root.scaling.setAll(1.25); break
        case 'dark_arts': root.position.set(-3.75, 0.1, 2.55); break
        case 'play': {
          const playIndex = played.findIndex((item) => item.id === card.id)
          root.setEnabled(playIndex >= 0)
          root.position.set(-4.4 + playIndex * 2.3, 0.08, -0.55)
          root.scaling.setAll(0.8)
          break
        }
      }
      if (card.id === selected) root.position.y += 0.65
    }
  }

  function publishAnchors() {
    const width = canvas.clientWidth, height = canvas.clientHeight
    if (!width || !height) return
    const viewport = camera.viewport.toGlobal(width, height)
    const items: CardAnchor[] = []
    for (const [id, object] of objects) {
      if (!object.root.isEnabled()) continue
      const point = Vector3.Project(object.root.position, Matrix.Identity(), scene.getTransformMatrix(), viewport)
      const side = Vector3.Project(object.root.position.add(new Vector3(0.8, 0, 1.08)), Matrix.Identity(), scene.getTransformMatrix(), viewport)
      items.push({ id, x: point.x / width * 100, y: point.y / height * 100,
        width: Math.max(44, Math.abs(side.x - point.x) * 2), height: Math.max(44, Math.abs(side.y - point.y) * 2) })
    }
    anchors(items)
  }
  function render() {
    if (stopped || disposed || !canvas.clientWidth || !canvas.clientHeight) { lastFrame = 0; return }
    const now = performance.now()
    if (lastFrame && appliedQuality === 'basic' && now - lastFrame < 1000 / 30 - 1) return
    if (lastFrame) {
      frameDistribution.add(now - lastFrame)
      intervals.push(now - lastFrame)
      if (intervals.length > 3600) intervals.shift()
      quality.frame(now, now - lastFrame)
      if (quality.current() !== appliedQuality) applyQuality()
    }
    lastFrame = now
    for (let index = motions.length - 1; index >= 0; index--) {
      const motion = motions[index]!
      const elapsed = Math.min(1, (now - motion.start) / motion.duration)
      const progress = 1 - (1 - elapsed) ** 3
      motion.mesh.position.copyFrom(Vector3.Lerp(motion.from, motion.to, progress))
      motion.mesh.position.y += Math.sin(progress * Math.PI) * motion.arc
      motion.mesh.visibility = elapsed > 0.8 ? (1 - elapsed) / 0.2 : 1
      if (elapsed === 1) { motion.mesh.dispose(); motions.splice(index, 1) }
    }
    scene.render()
    if (needsAnchors) { publishAnchors(); needsAnchors = false }
  }
  engine.runRenderLoop(render)

  return {
    update(game, cards, selectedId, handPage, playPage) {
      selected = selectedId
      const hand = cards.filter(card => card.zone === 'hand').slice(handPage * 7, handPage * 7 + 7)
      const played = cards.filter(card => card.zone === 'play').slice(playPage * 4, playPage * 4 + 4)
      const visible = cards.filter(card => card.zone !== 'hand' && card.zone !== 'play' || hand.includes(card) || played.includes(card))
      for (const [id, object] of objects) {
        const next = visible.find((card) => card.id === id)
        if (!next || JSON.stringify(next) !== JSON.stringify(object.card)) {
          object.root.dispose(false)
          object.face.dispose(false, true)
          objects.delete(id)
        }
      }
      place(visible, 0, 0)
      needsAnchors = true
      if (markerVersion === game.snapshot.state_version && markerGame === game.game.id) return
      markerVersion = game.snapshot.state_version
      markerGame = game.game.id
      for (const marker of markers) marker.dispose()
      markers.length = 0
      const piles = [game.table.draw_pile_count, game.table.discard_pile_count, game.table.hogwarts_deck_count, game.table.villain_deck_count]
      piles.forEach((count, index) => {
        const pile = CreateBox(`pile-${index}`, { width: 1.25, depth: 1.65, height: Math.max(0.05, Math.min(count, 20) * 0.025) }, scene)
        pile.position.set(index < 2 ? -8.8 : 9.8, 0.2, index % 2 ? -3.55 : -1.45)
        pile.material = back
        markers.push(pile)
      })
      for (let index = 0; index < (game.table.current_location?.control ?? 0); index++) {
        const token = CreateCylinder(`control-${index}`, { diameter: 0.3, height: 0.12, tessellation: 12 }, scene)
        token.position.set(-7.1 + index * 0.34, 0.17, 0.76)
        token.material = brass
        markers.push(token)
      }
      needsAnchors = true
    },
    resize() {
      if (!canvas.clientWidth || !canvas.clientHeight) return
      engine.resize()
      // Keep the complete board visible on small tablets as well as wide phones.
      const aspect = canvas.clientWidth / Math.max(1, canvas.clientHeight)
      const distance = Math.max(1, aspect / 4.1)
      camera.position.set(0, (aspect > 3 ? 10 : 14) * distance, (aspect > 3 ? -17 : -12) * distance)
      camera.setTarget(new Vector3(0, 0, -0.65))
      needsAnchors = true
    },
    pause(value) {
      stopped = value
      lastFrame = 0
      quality.resetWindow(performance.now())
      if (value) { discardMotion(); engine.stopRenderLoop(render) }
      else if (!disposed) engine.runRenderLoop(render)
    },
    discardMotion,
    setQuality(value) { quality.select(value, performance.now()); applyQuality() },
    present(group, reducedMotion) {
      if (disposed || stopped) return
      for (const cue of group.cues) {
        if (motions.length >= 24 || cue.kind === 'phase') continue
        const movingCard = Boolean(cue.from && cue.to)
        const particleBudget = reducedMotion ? 0 : qualityProfiles[appliedQuality].particles
        if (!movingCard && motions.filter(motion => motion.mesh.name.startsWith('effect-')).length >= particleBudget) continue
        const to = movingCard ? objects.get(cue.target)?.root.position.clone() ?? zonePoint(cue.to) : cuePoint(cue)
        const from = reducedMotion ? to.clone() : movingCard ? zonePoint(cue.from)
          : objects.get(cue.source ?? '')?.root.position.clone() ?? new Vector3(-1, 0.5, -3)
        const mesh = movingCard ? CreateBox(`motion-${group.key}`, { width: 1.5, depth: 2, height: 0.05 }, scene)
          : CreateSphere(`effect-${group.key}`, { diameter: reducedMotion ? 0.5 : 0.22, segments: 6 }, scene)
        mesh.material = movingCard ? back : cue.kind === 'damage' || cue.kind === 'stun' ? sparks.damage
          : cue.kind === 'heal' ? sparks.heal : sparks.resource
        mesh.isPickable = false
        mesh.position.copyFrom(from)
        motions.push({ mesh, from, to, start: performance.now(), duration: reducedMotion ? 120 : movingCard ? 420 : 300,
          arc: reducedMotion ? 0 : movingCard ? 1.4 : 0.6 })
      }
    },
    resetMeasurements() { intervals.length = 0; frameDistribution.reset() },
    measurements() {
      const sorted = [...intervals].sort((a, b) => a - b)
      return { frames: sorted.length, medianFrameMs: sorted[Math.floor(sorted.length * 0.5)] ?? 0,
        p95FrameMs: sorted[Math.floor(sorted.length * 0.95)] ?? 0, meshes: scene.meshes.length, textures: scene.textures.length,
        quality: appliedQuality, compressedMaterials: [wood, back].filter(material => material.diffuseTexture instanceof Texture && material.diffuseTexture.url?.endsWith('.ktx2') && material.diffuseTexture.isReady()).length,
        loadedCardModel: cardGeometry !== null, decodedImages: images.size, frameDistribution: frameDistribution.read(), estimatedTextureBytes: scene.textures.reduce((bytes, texture) => {
          const size = texture.getSize()
          return bytes + size.width * size.height * 4 * (texture instanceof DynamicTexture ? 1 : 4 / 3)
        }, 0) }
    },
    dispose() {
      disposed = true
      discardMotion()
      releaseDecoder()
      modelContainer?.dispose()
      engine.stopRenderLoop(render)
      scene.dispose()
      engine.dispose()
      objects.clear()
      for (const cached of images.values()) { cached.abort.abort(); cached.image.src = '' }
      images.clear()
      anchors([])
    },
  }
}
