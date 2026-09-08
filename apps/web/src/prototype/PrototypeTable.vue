<script setup lang="ts">
import { ref } from 'vue'
import GameTable3D from '../components/GameTable3D.vue'
import type { CardAcquisitionDestination, EffectTargetBinding, EffectOutcomeSummary } from '../contracts/identity-access.generated'
import { prototypeGame, refreshPrototypeIntentions } from './prototypeGame'
import { createTableTimeline } from '../presentation/tableTimeline'

const game = ref(prototypeGame())
const table = ref<InstanceType<typeof GameTable3D> | null>(null)
const history = ref<string[]>([])
const measurements = ref<ReturnType<InstanceType<typeof GameTable3D>['measurements']>>(null)
const timeline = createTableTimeline()
const presentationEpoch = ref(0)
function resetPresentation() { timeline.reset(game.value.game.id, game.value.snapshot.sequence); presentationEpoch.value++ }
resetPresentation()
function restart() { game.value = prototypeGame(); history.value = []; resetPresentation() }
function advance(message: string, effects: EffectOutcomeSummary[] = []) {
  game.value.snapshot.state_version++
  game.value.snapshot.sequence++
  game.value.snapshot.cursor = game.value.snapshot.sequence
  timeline.append(game.value.game.id, [{ type: 'dark_arts_completed', event_version: 3,
    sequence: game.value.snapshot.sequence, state_version: game.value.snapshot.state_version,
    turn: game.value.turn.number, actor_position: game.value.participant.position, effects,
    effect_stop: 'stable', prng_counter: 0 }], performance.now())
  refreshPrototypeIntentions(game.value)
  history.value.unshift(message)
}
function playCard(id: string, targets: EffectTargetBinding[]) {
  const previous = { ...game.value.participant.resources }
  const previousHealth = new Map(game.value.participants.map(hero => [hero.position, hero.resources.health]))
  const card = game.value.table.hand.find((item) => item.instance_id === id)
  if (!card) return
  game.value.table.hand = game.value.table.hand.filter((item) => item.instance_id !== id)
  game.value.table.play_area.push(card)
  if (id === 'wand') game.value.participant.resources.attack += 3
  else if (id === 'owl') {
    const position = Number(targets[0]?.target_ids[0]?.split(':')[1])
    const hero = game.value.participants.find((participant) => participant.position === position)
    if (hero) hero.resources.health = Math.min(10, hero.resources.health + 1)
  } else game.value.participant.resources.influence += id === 'book' ? 2 : 1
  const effects: EffectOutcomeSummary[] = [{ type: 'moved', rule_id: 'prototype', target_id: id,
    target_position: game.value.participant.position, from: 'hero_hand', to: 'hero_play_area' }]
  for (const resource of ['attack', 'influence'] as const) {
    effects.push({ type: 'resource_changed', rule_id: 'prototype', target_id: `hero:${game.value.participant.position}`,
      target_position: game.value.participant.position, resource, before: previous[resource], after: game.value.participant.resources[resource], cause: 'effect' })
  }
  if (id === 'owl') {
    const hero = game.value.participants.find(participant => `hero:${participant.position}` === targets[0]?.target_ids[0])
    if (hero) effects.push({ type: 'resource_changed', rule_id: 'prototype', target_id: `hero:${hero.position}`,
      target_position: hero.position, resource: 'health', before: previousHealth.get(hero.position)!, after: hero.resources.health, cause: 'effect' })
  }
  advance(`${card.name} passou da Mão à área de jogo.`, effects)
}
function attack(id: string, amount: number) {
  const villain = game.value.table.active_villains.find((item) => item.instance_id === id)
  if (!villain) return
  villain.health -= amount
  game.value.participant.resources.attack -= amount
  advance(`${villain.name} sofreu ${amount} de dano. Vida: ${villain.health}.`, [{ type: 'resource_changed', rule_id: 'prototype',
    target_id: id, resource: 'health', before: villain.health + amount, after: villain.health, cause: 'effect' }])
}
function acquire(id: string, destination: CardAcquisitionDestination) {
  const index = game.value.table.market.findIndex((item) => item.instance_id === id)
  const card = game.value.table.market[index]
  if (!card) return
  game.value.participant.resources.influence -= card.cost
  if (destination === 'draw_pile') game.value.table.draw_pile_count++
  else game.value.table.discard_pile_count++
  game.value.table.hogwarts_deck_count--
  game.value.table.market[index] = { ...card, instance_id: `${id}-next`, name: 'Wingardium Leviosa', cost: 2 }
  advance(`${card.name} foi adquirida para ${destination === 'draw_pile' ? 'o topo do baralho' : 'o descarte'}. Mercado reposto.`, [
    { type: 'moved', rule_id: 'prototype', target_id: id, target_position: game.value.participant.position, from: 'market', to: `hero_${destination}` },
    { type: 'moved', rule_id: 'prototype', target_id: `${id}-next`, from: 'hogwarts_deck', to: 'market' },
  ])
}
function endTurn() {
  const current = game.value
  const effects: EffectOutcomeSummary[] = [
    ...current.table.hand.map(card => ({ type: 'moved' as const, rule_id: 'prototype', target_id: card.instance_id, target_position: current.participant.position, from: 'hero_hand', to: 'hero_discard_pile' })),
    ...current.table.play_area.map(card => ({ type: 'moved' as const, rule_id: 'prototype', target_id: card.instance_id, target_position: current.participant.position, from: 'hero_play_area', to: 'hero_discard_pile' })),
  ]
  current.table.discard_pile_count += current.table.hand.length + current.table.play_area.length
  current.table.play_area = []
  current.table.hand = prototypeGame().table.hand
  current.table.draw_pile_count = 0
  current.participant.resources.attack = 0
  current.participant.resources.influence = 0
  current.turn.active_position = current.turn.active_position % current.participants.length + 1
  current.turn.number++
  current.participant = current.participants[current.turn.active_position - 1]!
  current.participant.resources.health--
  effects.push(...current.table.hand.map(card => ({ type: 'moved' as const, rule_id: 'prototype', target_id: card.instance_id,
    target_position: current.participant.position, from: 'hero_draw_pile', to: 'hero_hand' })))
  effects.push({ type: 'resource_changed', rule_id: 'prototype', target_id: `hero:${current.participant.position}`,
    target_position: current.participant.position, resource: 'health', before: current.participant.resources.health + 1,
    after: current.participant.resources.health, cause: 'effect' })
  advance(`Descarte e compra de 5 cartas. Turno de ${current.participant.hero.name}; Petrificação reduz a Vida para ${current.participant.resources.health}.`, effects)
}
function damageAndHeal(event: MouseEvent) {
  (event.currentTarget as HTMLButtonElement).closest('details')?.removeAttribute('open')
  const hero = game.value.participant
  const health = hero.resources.health
  advance('Dano e cura no mesmo lote: a Vida oficial permanece atualizada.', [
    { type: 'resource_changed', rule_id: 'prototype-damage', target_id: `hero:${hero.position}`, target_position: hero.position,
      resource: 'health', before: health, after: health - 2, cause: 'effect' },
    { type: 'resource_changed', rule_id: 'prototype-heal', target_id: `hero:${hero.position}`, target_position: hero.position,
      resource: 'health', before: health - 2, after: health, cause: 'effect' },
  ])
}
function extensiveHand() {
  const sample = prototypeGame().table.hand[0]!
  game.value.table.hand = Array.from({ length: 20 }, (_, index) => ({ ...sample, instance_id: `long-${index}`, name: `Carta ${index + 1} - Encantamento de proteção`, description: 'Texto longo demonstrativo. '.repeat(45) }))
  advance('Cenário de leitura: 20 cartas e texto extenso.')
}
</script>

<template>
  <main class="prototype-shell">
    <header class="prototype-heading"><h1>Ensaio da mesa</h1><span>Dados fictícios · protótipo interativo · sem servidor</span><button type="button" @click="restart">Reiniciar</button></header>
    <GameTable3D ref="table" :game="game" :commands-disabled="false" :pending-overlay="null" :take-presentation="timeline.take" :presentation-epoch="presentationEpoch"
      @discard-presentation="resetPresentation" @play-card="playCard" @assign-attack="attack" @acquire-card="acquire" />
    <footer class="prototype-footer"><details><summary>Ensaio e histórico</summary><div class="prototype-menu-content"><button type="button" @click="table?.exportMeasurements()">Exportar medidas</button><button type="button" @click="damageAndHeal">Ensaiar dano e cura</button><button type="button" @click="extensiveHand">Mão extensa e texto longo</button><button type="button" @click="restart">Reiniciar ensaio</button><ol><li v-for="(event, index) in history" :key="index">{{ event }}</li></ol></div></details><button type="button" @click="endTurn">Encerrar ações do Herói</button><button type="button" @click="measurements = table?.measurements() ?? null">Medir renderização</button></footer>
    <output v-if="measurements" :data-measurements="JSON.stringify(measurements)" class="prototype-metrics">Emulação local: {{ measurements.frames }} frames · mediana {{ measurements.medianFrameMs.toFixed(1) }} ms · p95 {{ measurements.p95FrameMs.toFixed(1) }} ms · {{ measurements.meshes }} meshes · {{ measurements.textures }} texturas. Não comprova desempenho físico.</output>
  </main>
</template>

<style>
.prototype-shell { max-width: 1600px; margin: auto; }
.prototype-shell:has(.table-canvas) { display: flex; flex-direction: column; height: 100dvh; }
.prototype-shell > .prototype-heading, .prototype-shell > .prototype-footer { flex-shrink: 0; }
.prototype-heading { display: flex; align-items: center; gap: 16px; padding: 0 12px; min-height: 32px; }
.prototype-heading h1 { font: 650 20px 'Archivo Narrow Variable', sans-serif; margin: 0; }
.prototype-heading span { font-size: 12px; color: var(--chalk-muted); }
.prototype-heading button { margin-left: auto; }
.prototype-shell button, .prototype-shell summary { min-height: 44px; padding: 8px 12px; background: var(--ink-raised); color: var(--chalk); border: 1px solid var(--brass-quiet); border-radius: 6px; font: inherit; }
.prototype-footer { display: flex; align-items: flex-start; gap: 12px; padding: 4px 12px; }
.prototype-footer details { margin-right: auto; max-width: 50%; }
.prototype-footer ol { font-size: 14px; }
.prototype-menu-content { position: fixed; z-index: 4; top: 44px; bottom: 56px; left: 12px; width: min(380px, calc(100% - 24px)); padding: 16px; overflow-y: auto; background: var(--ink-raised); border: 1px solid var(--brass); border-radius: 8px; }
.prototype-metrics { position: fixed; z-index: 3; right: 12px; bottom: 56px; max-width: min(560px, calc(100% - 24px)); font-size: 13px; padding: 8px 12px; background: var(--ink-raised); pointer-events: none; }
@media (max-height: 500px) and (orientation: landscape) {
  .prototype-heading span { display: none; }
  .prototype-heading { min-height: 0; height: 24px; }
  .prototype-heading button { display: none; }
  .prototype-footer { font-size: 12px; }
}
@media (orientation: portrait) { .prototype-heading, .prototype-footer { flex-wrap: wrap; } }
</style>
