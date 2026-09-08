<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import type { CardAcquisitionDestination, EffectTargetBinding, GameProjectionResponse, ParticipantPresence } from '../contracts/identity-access.generated'
import { createTablePresentation, type TableIntent } from '../presentation/tablePresentation'
import { tableState } from '../presentation/tableProjection'
import type { CardAnchor, TableScene } from '../presentation/tableScene'
import type { PendingGameCommandOverlay } from '../stores/gameCommand'
import GameTable from './GameTable.vue'
import HeroVitals from './HeroVitals.vue'
import { createTableTelemetry } from '../presentation/tableTelemetry'
import type { TableVisualGroup } from '../presentation/tableTimeline'
import { cueLabel } from '../presentation/tableFeedback'
import { qualityProfiles, type QualityPreference, type TableQuality } from '../presentation/tableQuality'
import { createTableAudio, loadAudioSettings } from '../presentation/tableAudio'

const props = defineProps<{
  game: GameProjectionResponse
  commandsDisabled: boolean
  pendingOverlay: PendingGameCommandOverlay | null
  presence?: Record<number, ParticipantPresence['status']>
  decisionError?: string | null
  takePresentation?: (now: number) => TableVisualGroup | null
  presentationEpoch?: number
}>()
const emit = defineEmits<{
  playCard: [cardId: string, targets: EffectTargetBinding[]]
  assignAttack: [villainId: string, amount: number]
  acquireCard: [cardId: string, destination: CardAcquisitionDestination]
  modeChange: [accessible: boolean]
  discardPresentation: []
}>()
function savedAccessibleMode() {
  try { return localStorage.getItem('hogwarts.table-mode') === 'accessible' } catch { return false }
}
const accessible = ref(savedAccessibleMode())
const zones = [
  { id: 'hand', label: 'Sua mão' }, { id: 'play', label: 'Área de jogo' },
  { id: 'market', label: 'Mercado de Hogwarts' }, { id: 'villain', label: 'Vilões ativos' },
  { id: 'location', label: 'Local ativo' }, { id: 'dark_arts', label: 'Arte das Trevas revelada' },
] as const
const canvas = ref<HTMLCanvasElement | null>(null)
const inspection = ref<HTMLElement | null>(null)
const boardElement = ref<HTMLElement | null>(null)
const playZone = ref<HTMLElement | null>(null)
const anchors = shallowRef<CardAnchor[]>([])
const rendererStatus = ref<'loading' | 'ready' | 'failed'>('loading')
const handPage = ref(0)
const playPage = ref(0)
const notice = ref('')
const feedback = shallowRef<Array<{ key: string; kind: string; position?: number; target: string; before?: number; after?: number; label: string; expires: number }>>([])
const announcement = ref('')
const localEpoch = ref(0)
function savedQuality(): QualityPreference {
  try {
    const saved = localStorage.getItem('hogwarts.table-quality')
    if (saved === 'basic' || saved === 'balanced' || saved === 'high') return saved
  } catch { /* Preferences remain usable without storage. */ }
  return 'auto'
}
const qualityPreference = ref(savedQuality())
const activeQuality = ref<TableQuality>('balanced')
const audioSettings = ref(loadAudioSettings())
const audio = createTableAudio(audioSettings.value)
const telemetry = createTableTelemetry()
const diagnostics = new URLSearchParams(location.search).has('table-diagnostics')
function readMeasurements() { return scene ? { ...scene.measurements(), audio: audio.measurements(), telemetry: telemetry.read() } : null }
function resetMeasurements() { scene?.resetMeasurements(); telemetry.reset() }
function exportMeasurements() {
  const report = { capturedAt: new Date().toISOString(), userAgent: navigator.userAgent,
    viewport: { width: innerWidth, height: innerHeight, pixelRatio: devicePixelRatio },
    measurements: readMeasurements(), deviceConditions: null }
  const url = URL.createObjectURL(new Blob([JSON.stringify(report, null, 2)], { type: 'application/json' }))
  const link = document.createElement('a')
  link.href = url; link.download = `hogwarts-table-${Date.now()}.json`; link.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
let inputAt = 0
let inputFrame = 0
function observeInput(event: MouseEvent) {
  inputAt = event.timeStamp
  cancelAnimationFrame(inputFrame)
  void nextTick(() => { inputFrame = requestAnimationFrame(() => telemetry.input.add(performance.now() - event.timeStamp)) })
}
const audioReady = ref(false)
const audioNotice = ref('')
async function toggleAudio() {
  if (audioReady.value && audioSettings.value.enabled) {
    audioSettings.value.enabled = false
    audio.configure(audioSettings.value)
    return
  }
  audioSettings.value.enabled = true
  audio.configure(audioSettings.value)
  audioReady.value = await audio.unlock()
  audioNotice.value = audioReady.value ? '' : 'O navegador não liberou o som. Tente ativá-lo novamente.'
}
watch(audioSettings, settings => audio.configure(settings), { deep: true })
function changeQuality(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  if (value !== 'auto' && value !== 'basic' && value !== 'balanced' && value !== 'high') return
  qualityPreference.value = value
  scene?.setQuality(value)
  try { localStorage.setItem('hogwarts.table-quality', value) } catch { /* Keep the live preference. */ }
}
let feedbackFrame = 0
let nextGroupAt = 0
const motion = typeof window.matchMedia === 'function' ? window.matchMedia('(prefers-reduced-motion: reduce)') : null

function discardFeedback() {
  feedback.value = []
  announcement.value = ''
  nextGroupAt = 0
  localEpoch.value++
  scene?.discardMotion()
  audio.discardEffects()
}
function resetFeedback() { emit('discardPresentation'); discardFeedback() }
function updateFeedbackLoop() {
  cancelAnimationFrame(feedbackFrame)
  if (!document.hidden && rendererStatus.value === 'ready' && !accessible.value) {
    feedbackFrame = requestAnimationFrame(animateFeedback)
  }
}
watch([accessible, rendererStatus], updateFeedbackLoop)
function animateFeedback(now: number) {
  if (!document.hidden && rendererStatus.value === 'ready' && !accessible.value && canvas.value?.clientHeight) {
    if (feedback.value.some(item => item.expires <= now)) feedback.value = feedback.value.filter(item => item.expires > now)
    if (now >= nextGroupAt) {
      const group = props.takePresentation?.(now)
      if (group) {
        nextGroupAt = now + (group.summarized ? 80 : 160)
        const items = group.cues.map((cue, index) => ({ key: `${group.key}:${index}`, kind: cue.kind,
          ...(cue.position === undefined ? {} : { position: cue.position }), target: cue.resource === 'control' || cue.kind === 'location_lost' ? 'current-location' : cue.target,
          ...(cue.before === undefined ? {} : { before: cue.before }), ...(cue.after === undefined ? {} : { after: cue.after }),
          label: cueLabel(cue), expires: now + 1100 }))
        feedback.value = [...feedback.value, ...items].slice(-32)
        announcement.value = `${group.summarized ? `${group.summarized} acontecimentos resumidos. ` : ''}${items.map(item => {
          const hero = props.game.participants.find(participant => participant.position === item.position)
          return `${hero?.hero.name ?? projection.value.cards.find(card => card.id === item.target)?.name ?? ''}: ${item.label}`
        }).join('. ')}`
        if (!group.summarized) scene?.present(group, motion?.matches ?? false)
        audio.play(group)
      }
    }
  }
  feedbackFrame = requestAnimationFrame(animateFeedback)
}
const projection = computed(() => tableState(props.game, props.commandsDisabled))
const table = createTablePresentation(projection.value, dispatch)
const view = shallowRef(table.view())
let scene: TableScene | null = null
let resize: ResizeObserver | null = null
let generation = 0
let lastFocusedCard: string | null = null
let pointer: { id: number; card: string; x: number; y: number; dragged: boolean; element: HTMLElement } | null = null
let suppressClick = false
const orientation = typeof window.matchMedia === 'function' ? window.matchMedia('(orientation: portrait)') : null

function dispatch(intent: TableIntent) {
  if (intent.type === 'play_card') emit('playCard', intent.cardId, intent.targets)
  if (intent.type === 'assign_attack') emit('assignAttack', intent.villainId, intent.amount)
  if (intent.type === 'acquire_card') emit('acquireCard', intent.cardId, intent.destination)
  if (inputAt) telemetry.command.add(performance.now() - inputAt)
}
function refresh() {
  view.value = table.view()
  if (!document.hidden) scene?.update(props.game, projection.value.cards, view.value.selected?.id ?? null, handPage.value, playPage.value)
}
function select(id: string, event: MouseEvent) {
  if (suppressClick && event.detail > 0) { suppressClick = false; return }
  suppressClick = false
  notice.value = ''
  lastFocusedCard = id
  table.select(id)
  refresh()
  void nextTick(() => inspection.value?.focus())
}
function cancelPointer() {
  if (!pointer) return
  suppressClick = pointer.dragged
  if (pointer.element.hasPointerCapture(pointer.id)) pointer.element.releasePointerCapture(pointer.id)
  pointer = null
}
function cancel() {
  cancelPointer()
  table.cancel()
  refresh()
  void nextTick(() => {
    const button = [...(boardElement.value?.querySelectorAll<HTMLButtonElement>('[data-card-id]') ?? [])]
      .find((element) => element.dataset.cardId === lastFocusedCard)
    if (button) button.focus()
    else boardElement.value?.closest('.table-experience')?.querySelector<HTMLElement>('.table-toolbar')?.focus()
  })
}
function confirm() { table.confirm(); refresh() }
function toggleTarget(selector: string, target: string) { table.toggleTarget(selector, target); refresh() }
function changeAmount(value: number) { table.setAmount(value); refresh() }
function changeDestination(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  if (value === 'draw_pile' || value === 'discard_pile') table.setDestination(value)
  refresh()
}
function changePage(offset: number) {
  table.cancel()
  handPage.value = Math.max(0, Math.min(Math.ceil(props.game.table.hand.length / 7) - 1, handPage.value + offset))
  refresh()
}
function changePlayPage(offset: number) {
  cancelPointer()
  table.cancel()
  playPage.value = Math.max(0, Math.min(Math.ceil(props.game.table.play_area.length / 4) - 1, playPage.value + offset))
  refresh()
}
async function startScene() {
  resetFeedback()
  const request = ++generation
  scene?.dispose()
  scene = null
  rendererStatus.value = 'loading'
  await nextTick()
  const target = canvas.value
  if (!target || accessible.value) return
  try {
    if (typeof WebGL2RenderingContext === 'undefined') throw new Error('WebGL unavailable')
    const { mountTableScene } = await import('../presentation/tableScene')
    if (request !== generation) return
    scene = mountTableScene(target, (value) => { anchors.value = value }, value => { activeQuality.value = value }, telemetry.frames)
    telemetry.sceneStarted()
    scene.setQuality(qualityPreference.value)
    refresh()
    scene.resize()
    scene.pause(document.hidden)
    rendererStatus.value = 'ready'
    resize?.disconnect()
    resize = new ResizeObserver(() => scene?.resize())
    resize.observe(target)
  } catch {
    scene?.dispose()
    scene = null
    rendererStatus.value = 'failed'
  }
}
function toggleMode() {
  resetFeedback()
  cancelPointer()
  accessible.value = !accessible.value
  try { localStorage.setItem('hogwarts.table-mode', accessible.value ? 'accessible' : 'visual') } catch { /* The mode still works without browser storage. */ }
  emit('modeChange', accessible.value)
  table.cancel()
  if (accessible.value) { generation++; scene?.dispose(); scene = null; resize?.disconnect() }
  else void startScene()
  refresh()
}
function orientationChanged() {
  resetFeedback()
  cancelPointer()
  table.cancel()
  refresh()
  scene?.resize()
}
function visibilityChanged() {
  resetFeedback()
  audio.pause(document.hidden)
  cancelPointer()
  table.cancel()
  refresh()
  scene?.pause(document.hidden)
  updateFeedbackLoop()
}
function contextLost(event: Event) {
  event.preventDefault()
  resetFeedback()
  cancelPointer()
  table.cancel()
  refresh()
  scene?.pause(true)
  rendererStatus.value = 'failed'
}
function pointerDown(id: string, event: PointerEvent) {
  if (!event.isPrimary || event.button !== 0) return
  suppressClick = false
  const element = event.currentTarget as HTMLElement
  pointer = { id: event.pointerId, card: id, x: event.clientX, y: event.clientY, dragged: false, element }
  element.setPointerCapture(event.pointerId)
}
function pointerMove(event: PointerEvent) {
  if (!pointer || pointer.id !== event.pointerId || pointer.dragged) return
  if (Math.hypot(event.clientX - pointer.x, event.clientY - pointer.y) < 12) return
  pointer.dragged = true
  table.beginDrag(pointer.card)
  refresh()
}
function pointerUp(event: PointerEvent) {
  if (!pointer || pointer.id !== event.pointerId) return
  if (pointer.dragged) {
    observeInput(event)
    suppressClick = true
    const bounds = playZone.value?.getBoundingClientRect()
    const valid = bounds && event.clientX >= bounds.left && event.clientX <= bounds.right && event.clientY >= bounds.top && event.clientY <= bounds.bottom
    table.drop(valid ? 'play' : null)
    refresh()
    if (view.value.selected) void nextTick(() => inspection.value?.focus())
  }
  pointer = null
}
watch(projection, (value, previous) => {
  const selectedChanged = value.version !== previous.version && view.value.selected
  const restoreFocus = selectedChanged && inspection.value?.contains(document.activeElement)
  if (selectedChanged) notice.value = 'A mesa foi atualizada. Selecione novamente para decidir.'
  table.update(value)
  if (value.version !== previous.version || value.disabled) cancelPointer()
  handPage.value = Math.min(handPage.value, Math.max(0, Math.ceil(props.game.table.hand.length / 7) - 1))
  playPage.value = Math.min(playPage.value, Math.max(0, Math.ceil(props.game.table.play_area.length / 4) - 1))
  refresh()
  if (restoreFocus) void nextTick(() => boardElement.value?.closest('.table-experience')?.querySelector<HTMLElement>('.table-toolbar')?.focus())
})
watch(() => props.decisionError, (message) => {
  if (!message) return
  cancel()
  notice.value = message
})
onMounted(() => {
  emit('modeChange', accessible.value)
  void startScene()
  window.addEventListener('orientationchange', orientationChanged)
  orientation?.addEventListener('change', orientationChanged)
  window.addEventListener('blur', orientationChanged)
  document.addEventListener('visibilitychange', visibilityChanged)
  motion?.addEventListener('change', resetFeedback)
})
onBeforeUnmount(() => {
  audio.dispose()
  cancelAnimationFrame(feedbackFrame)
  cancelAnimationFrame(inputFrame)
  telemetry.dispose()
  motion?.removeEventListener('change', resetFeedback)
  cancelPointer()
  generation++
  resize?.disconnect()
  scene?.dispose()
  window.removeEventListener('orientationchange', orientationChanged)
  orientation?.removeEventListener('change', orientationChanged)
  window.removeEventListener('blur', orientationChanged)
  document.removeEventListener('visibilitychange', visibilityChanged)
})
watch(() => props.presentationEpoch, discardFeedback)
const positionedCards = computed(() => anchors.value.flatMap((anchor) => {
  const card = projection.value.cards.find((item) => item.id === anchor.id)
  return card ? [{ ...anchor, card }] : []
}))
const confirmationLabel = computed(() => {
  const selected = view.value.selected
  if (selected?.zone === 'villain') return `Atacar ${selected.name} com ${view.value.amount}`
  if (selected?.zone === 'market') return `Adquirir ${selected.name} por ${selected.cost} de Influência`
  return `Jogar ${selected?.name ?? 'carta'}`
})
defineExpose({ measurements: readMeasurements, exportMeasurements })
</script>

<template>
  <section class="table-experience" @click.capture="observeInput" aria-label="Mesa de Hogwarts">
    <div class="table-toolbar" :id="accessible ? undefined : 'game-table-heading'" tabindex="-1">
      <span v-if="!accessible"><b>Turno {{ game.turn.number }}</b> · {{ game.participants.find(p => p.position === game.turn.active_position)?.display_name }}<small class="table-phase">{{ ({ hero_actions: 'Ações do Herói', dark_arts: 'Artes das Trevas', villains: 'Vilões', end_turn: 'Fim do turno' })[game.turn.phase] }}</small></span>
      <div v-if="!accessible && game.table.play_area.length > 4" class="play-pagination">
        <button type="button" :disabled="playPage === 0" aria-label="Cartas jogadas anteriores" @click="changePlayPage(-1)">←</button>
        <span>Em jogo {{ playPage + 1 }}/{{ Math.ceil(game.table.play_area.length / 4) }}</span>
        <button type="button" :disabled="(playPage + 1) * 4 >= game.table.play_area.length" aria-label="Próximas cartas jogadas" @click="changePlayPage(1)">→</button>
      </div>
      <button type="button" :aria-pressed="accessible" @click="toggleMode">{{ accessible ? 'Voltar à mesa 3D' : 'Modo acessível' }}</button>
      <details class="table-settings">
        <summary>Imagem e som</summary>
        <div class="table-settings-content">
          <label>Qualidade da mesa
            <select :value="qualityPreference" @change="changeQuality">
              <option value="auto">Automática</option>
              <option value="basic">Básico · 30 fps</option>
              <option value="balanced">Equilibrado · 60 fps</option>
              <option value="high">Alto · 60 fps</option>
            </select>
          </label>
          <p>Perfil atual: {{ qualityProfiles[activeQuality].label }}. As metas de fps dependem do aparelho.</p>
          <button type="button" :aria-pressed="audioReady && audioSettings.enabled" @click="toggleAudio">{{ audioReady && audioSettings.enabled ? 'Desativar som' : 'Ativar som' }}</button>
          <label>Efeitos · {{ Math.round(audioSettings.effects * 100) }}%
            <input v-model.number="audioSettings.effects" type="range" min="0" max="1" step="0.05" />
          </label>
          <label>Ambiente · {{ Math.round(audioSettings.ambient * 100) }}%
            <input v-model.number="audioSettings.ambient" type="range" min="0" max="1" step="0.05" />
          </label>
          <label><input v-model="audioSettings.vibration" type="checkbox" /> Vibrar nos impactos, se disponível</label>
          <template v-if="diagnostics"><button type="button" @click="resetMeasurements">Iniciar medição</button><button type="button" @click="exportMeasurements">Exportar diagnóstico</button></template>
          <p v-if="audioNotice" role="status">{{ audioNotice }}</p>
        </div>
      </details>
    </div>
    <GameTable v-if="accessible" :game="game" :commands-disabled="commandsDisabled" :pending-overlay="pendingOverlay"
      @play-card="(id, targets) => emit('playCard', id, targets)"
      @assign-attack="(id, amount) => emit('assignAttack', id, amount)"
      @acquire-card="(id, destination) => emit('acquireCard', id, destination)" />
    <template v-else>
      <div class="hero-strip" aria-label="Heróis e recursos oficiais">
        <div v-for="hero in game.participants" :key="hero.position" class="hero-medallion" :class="{ 'hero-medallion--active': hero.position === game.turn.active_position }">
          <span class="hero-portrait" :class="`hero-portrait--${hero.hero.id}`" aria-hidden="true"></span>
          <span class="hero-identity"><b :title="hero.hero.name">{{ hero.hero.name }}</b><small :title="hero.display_name">{{ hero.position === game.participant.position ? 'Você' : hero.display_name }}</small><small v-if="hero.stunned" class="hero-stunned">Atordoado</small></span>
          <span v-if="presence?.[hero.position]" class="hero-presence" :class="`hero-presence--${presence[hero.position]}`" role="img" :aria-label="`${hero.hero.name}: ${presence[hero.position] === 'online' ? 'conectado' : presence[hero.position] === 'reconnecting' ? 'reconectando' : 'ausente'}`" />
          <HeroVitals :resources="hero.resources" :epoch="localEpoch" :damage="feedback.findLast(cue => cue.kind === 'damage' && cue.position === hero.position)" />
          <span class="hero-feedback" aria-hidden="true"><span v-for="item in feedback.filter(cue => cue.position === hero.position && !['play', 'draw', 'discard', 'acquire'].includes(cue.kind))" :key="item.key" class="table-effect" :class="`table-effect--${item.kind}`">{{ item.label }}</span></span>
        </div>
      </div>
      <div class="rotation-guidance" role="status">
        <strong>Gire o celular para abrir a mesa</strong>
        <p>A partida continua conectada. Você também pode jogar nesta orientação pelo modo acessível.</p>
        <button type="button" @click="toggleMode">Jogar no modo acessível</button>
      </div>
      <div ref="boardElement" class="table-viewport" @keydown.esc="cancel">
        <canvas ref="canvas" class="table-canvas" aria-hidden="true" @pointerdown="cancel" @webglcontextlost="contextLost" @webglcontextrestored="startScene" />
        <div v-if="rendererStatus !== 'ready'" class="table-render-status" role="status">
          <strong>{{ rendererStatus === 'loading' ? 'Preparando sua mesa…' : 'Não foi possível abrir a mesa 3D' }}</strong>
          <template v-if="rendererStatus === 'failed'">
            <p>Sua posição está preservada. Tente novamente ou continue no modo acessível.</p>
            <button type="button" @click="startScene">Tentar mesa 3D novamente</button>
            <button type="button" @click="toggleMode">Continuar no modo acessível</button>
          </template>
        </div>
        <template v-if="rendererStatus === 'ready'">
          <div class="board-feedback" aria-hidden="true">
            <span v-for="item in feedback.filter(cue => cue.position === undefined || ['play', 'draw', 'discard', 'acquire'].includes(cue.kind))" :key="item.key" class="table-effect"
              :class="`table-effect--${item.kind}`"
              :style="{ left: `${anchors.find(anchor => anchor.id === item.target)?.x ?? (item.kind === 'location_lost' ? 24 : 50)}%`, top: `${anchors.find(anchor => anchor.id === item.target)?.y ?? 40}%` }">{{ item.label }}</span>
          </div>
          <span class="zone-label zone-label--market">Mercado de Hogwarts</span>
          <span class="zone-label zone-label--play">Área de jogo</span>
          <div ref="playZone" class="table-drop-zone" :class="{ 'table-drop-zone--active': view.dragging }" aria-hidden="true"><span v-if="view.dragging">Solte para jogar ou escolher alvos</span></div>
          <span class="zone-label zone-label--hand">Sua mão · {{ game.table.hand.length }} cartas</span>
          <div class="table-pile-labels"><span>Compra {{ game.table.draw_pile_count }}</span><span>Descarte {{ game.table.discard_pile_count }}</span></div>
          <div v-for="zone in zones" :key="zone.id" role="group" :aria-label="zone.label">
          <button v-for="item in positionedCards.filter(candidate => candidate.card.zone === zone.id)" :key="item.id" type="button" class="card-hit-target"
            :class="{ 'card-hit-target--selected': view.selected?.id === item.id }"
            :data-card-id="item.id" :aria-label="`Inspecionar ${item.card.name}${item.card.cost !== undefined ? `, custo ${item.card.cost}` : ''}${item.card.health !== undefined ? `, Vida ${item.card.health}` : ''}`"
            :aria-pressed="view.selected?.id === item.id"
            :style="{ left: `${item.x}%`, top: `${item.y}%`, width: `${item.width}px`, height: `${item.height}px` }"
            @pointerdown="pointerDown(item.id, $event)" @pointermove="pointerMove" @pointerup="pointerUp" @pointercancel="cancel"
            @click="select(item.id, $event)">
            <span class="card-hit-target__label"><span class="card-hit-target__name">{{ item.card.name }}</span><small v-if="item.card.health !== undefined">Vida {{ item.card.health }}</small><small v-else-if="item.card.cost !== undefined">Custo {{ item.card.cost }}</small><small v-else-if="item.card.detail">{{ item.card.detail }}</small></span>
          </button>
          </div>
          <div v-if="game.table.hand.length > 7" class="hand-pagination">
            <button type="button" :disabled="handPage === 0" aria-label="Cartas anteriores" @click="changePage(-1)">←</button>
            <span>{{ handPage + 1 }} / {{ Math.ceil(game.table.hand.length / 7) }}</span>
            <button type="button" :disabled="(handPage + 1) * 7 >= game.table.hand.length" aria-label="Próximas cartas" @click="changePage(1)">→</button>
          </div>
        </template>
        <section v-if="view.selected && !view.dragging" ref="inspection" class="card-inspection" tabindex="-1" aria-labelledby="inspection-heading">
          <header><h3 id="inspection-heading">{{ view.selected.name }}</h3><button type="button" aria-label="Fechar inspeção" @click="cancel">×</button></header>
          <p class="inspection-copy">{{ view.selected.description || 'Sem texto adicional.' }}</p>
          <p v-if="view.selected.cost !== undefined">Custo {{ view.selected.cost }} · Sua Influência {{ game.participant.resources.influence }}</p>
          <p v-if="view.selected.health !== undefined">Vida {{ view.selected.health }} · Seu Ataque {{ game.participant.resources.attack }}</p>
          <fieldset v-for="slot in view.slots" :key="slot.selector_id" :disabled="view.blocked">
            <legend>Escolha de {{ slot.min }} a {{ slot.max }} alvos · {{ view.targets[slot.selector_id]?.length ?? 0 }} selecionados</legend>
            <label v-for="option in slot.options" :key="option.target_id" class="table-target-option">
              <input :type="slot.min === 1 && slot.max === 1 ? 'radio' : 'checkbox'" :name="`scene-${slot.selector_id}`"
                :checked="view.targets[slot.selector_id]?.includes(option.target_id) ?? false"
                :disabled="(view.targets[slot.selector_id]?.length ?? 0) >= slot.max && slot.max > 1 && !view.targets[slot.selector_id]?.includes(option.target_id)"
                @change="toggleTarget(slot.selector_id, option.target_id)" />{{ option.label }}
            </label>
          </fieldset>
          <div v-if="view.maxAmount" class="attack-quantity">
            <button type="button" aria-label="Menos Ataque" :disabled="view.blocked || view.amount <= 1" @click="changeAmount(view.amount - 1)">−</button>
            <output aria-label="Quantidade de Ataque">{{ view.amount }}</output>
            <button type="button" aria-label="Mais Ataque" :disabled="view.blocked || view.amount >= view.maxAmount" @click="changeAmount(view.amount + 1)">+</button>
          </div>
          <label v-if="view.destinations.length" class="acquisition-destination">Destino
            <select :value="view.destination" :disabled="view.blocked" @change="changeDestination">
              <option v-for="destination in view.destinations" :key="destination" :value="destination">{{ destination === 'draw_pile' ? 'Topo do baralho' : 'Descarte' }}</option>
            </select>
          </label>
          <button v-if="['hand', 'market', 'villain'].includes(view.selected.zone)" type="button" class="table-confirm" :disabled="!view.canConfirm" @click="confirm">{{ confirmationLabel }}</button>
          <p v-if="view.blocked" role="status">Aguarde a confirmação e a sincronização da mesa.</p>
          <p v-else-if="!view.canConfirm && !view.slots.length && ['hand', 'market', 'villain'].includes(view.selected.zone)">Esta ação não está disponível no estado atual.</p>
        </section>
      </div>
      <p class="table-notice" role="status">{{ notice || (pendingOverlay ? 'Intenção enviada. Aguardando confirmação.' : 'Toque em uma carta para ler. Jogar exige sua confirmação.') }}</p>
      <p class="visually-hidden" role="status" aria-label="Acontecimentos da mesa">{{ announcement }}</p>
    </template>
  </section>
</template>

<style src="../presentation/table.css"></style>
