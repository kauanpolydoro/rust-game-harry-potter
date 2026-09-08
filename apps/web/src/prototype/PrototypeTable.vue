<script setup lang="ts">
import { ref } from 'vue'
import GameTable3D from '../components/GameTable3D.vue'
import type { CardAcquisitionDestination, EffectTargetBinding } from '../contracts/identity-access.generated'
import { prototypeGame, refreshPrototypeIntentions } from './prototypeGame'
import type { TableMeasurements } from '../presentation/tableScene'

const game = ref(prototypeGame())
const table = ref<InstanceType<typeof GameTable3D> | null>(null)
const history = ref<string[]>([])
const measurements = ref<TableMeasurements | null>(null)
function advance(message: string) {
  game.value.snapshot.state_version++
  game.value.snapshot.sequence++
  refreshPrototypeIntentions(game.value)
  history.value.unshift(message)
}
function playCard(id: string, targets: EffectTargetBinding[]) {
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
  advance(`${card.name} passou da Mão à área de jogo.`)
}
function attack(id: string, amount: number) {
  const villain = game.value.table.active_villains.find((item) => item.instance_id === id)
  if (!villain) return
  villain.health -= amount
  game.value.participant.resources.attack -= amount
  advance(`${villain.name} sofreu ${amount} de dano. Vida: ${villain.health}.`)
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
  advance(`${card.name} foi adquirida para ${destination === 'draw_pile' ? 'o topo do baralho' : 'o descarte'}. Mercado reposto.`)
}
function endTurn() {
  const current = game.value
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
  advance(`Descarte e compra de 5 cartas. Turno de ${current.participant.hero.name}; Petrificação reduz a Vida para ${current.participant.resources.health}.`)
}
function extensiveHand() {
  const sample = prototypeGame().table.hand[0]!
  game.value.table.hand = Array.from({ length: 20 }, (_, index) => ({ ...sample, instance_id: `long-${index}`, name: `Carta ${index + 1} — Encantamento de proteção`, description: 'Texto longo demonstrativo. '.repeat(45) }))
  advance('Cenário de leitura: 20 cartas e texto extenso.')
}
</script>

<template>
  <main class="prototype-shell">
    <header class="prototype-heading"><h1>Ensaio da mesa</h1><span>Dados fictícios · protótipo interativo · sem servidor</span><button type="button" @click="game = prototypeGame(); history = []">Reiniciar</button></header>
    <GameTable3D ref="table" :game="game" :commands-disabled="false" :pending-overlay="null" @play-card="playCard" @assign-attack="attack" @acquire-card="acquire" />
    <footer class="prototype-footer"><details><summary>Ensaio e histórico</summary><div class="prototype-menu-content"><button type="button" @click="extensiveHand">Mão extensa e texto longo</button><button type="button" @click="game = prototypeGame(); history = []">Reiniciar ensaio</button><ol><li v-for="(event, index) in history" :key="index">{{ event }}</li></ol></div></details><button type="button" @click="endTurn">Encerrar ações do Herói</button><button type="button" @click="measurements = table?.measurements() ?? null">Medir renderização</button></footer>
    <output v-if="measurements" class="prototype-metrics">Emulação local: {{ measurements.frames }} frames · mediana {{ measurements.medianFrameMs.toFixed(1) }} ms · p95 {{ measurements.p95FrameMs.toFixed(1) }} ms · {{ measurements.meshes }} meshes · {{ measurements.textures }} texturas. Não comprova desempenho físico.</output>
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
.prototype-metrics { display: block; font-size: 13px; padding: 8px 12px; }
@media (max-height: 500px) and (orientation: landscape) {
  .prototype-heading span { display: none; }
  .prototype-heading { min-height: 0; height: 24px; }
  .prototype-heading button { display: none; }
  .prototype-footer { font-size: 12px; }
}
@media (orientation: portrait) { .prototype-heading, .prototype-footer { flex-wrap: wrap; } }
</style>
