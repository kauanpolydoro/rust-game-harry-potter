<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import type {
  EffectTargetBinding,
  GameProjectionResponse,
  LegalAttackSummary,
  LegalPlayCardSummary,
  LegalTargetSlotSummary,
} from '../contracts/identity-access.generated'
import type { PendingGameCommandOverlay } from '../stores/gameCommand'

const props = defineProps<{
  commandsDisabled: boolean
  game: GameProjectionResponse
  pendingOverlay: PendingGameCommandOverlay | null
}>()

const emit = defineEmits<{
  acquireCard: [cardId: string]
  assignAttack: [villainId: string, amount: number]
  playCard: [cardId: string, targets: EffectTargetBinding[]]
}>()

const cardTargets = ref<Record<string, Record<string, string[]>>>({})
const attackAmounts = ref<Record<string, number>>({})

const pendingCardId = computed(() =>
  props.pendingOverlay?.type === 'play_card' ? props.pendingOverlay.card_id : null,
)
const pendingVillainId = computed(() =>
  props.pendingOverlay?.type === 'assign_attack' ? props.pendingOverlay.villain_id : null,
)
const pendingMarketCardId = computed(() =>
  props.pendingOverlay?.type === 'acquire_card' ? props.pendingOverlay.card_id : null,
)
const pendingTargetIds = computed(() =>
  props.pendingOverlay?.type === 'play_card'
    ? new Set(props.pendingOverlay.targets.flatMap((binding) => binding.target_ids))
    : new Set<string>(),
)

watch(
  () => props.game.snapshot.state_version,
  () => {
    cardTargets.value = {}
    attackAmounts.value = {}
  },
)

function playableCard(cardId: string): LegalPlayCardSummary | undefined {
  return props.game.legal_intentions.play_cards.find((intent) => intent.card_id === cardId)
}

function attackIntent(villainId: string): LegalAttackSummary | undefined {
  return props.game.legal_intentions.assign_attack.find(
    (intent) => intent.villain_id === villainId,
  )
}

function selectedTargets(cardId: string, selectorId: string): string[] {
  return cardTargets.value[cardId]?.[selectorId] ?? []
}

function targetIsSelected(cardId: string, selectorId: string, targetId: string): boolean {
  return selectedTargets(cardId, selectorId).includes(targetId)
}

function targetIsDisabled(
  cardId: string,
  slot: LegalTargetSlotSummary,
  targetId: string,
): boolean {
  const selected = selectedTargets(cardId, slot.selector_id)
  return (
    props.commandsDisabled ||
    (slot.max > 1 && selected.length >= slot.max && !selected.includes(targetId))
  )
}

function updateTarget(
  cardId: string,
  slot: LegalTargetSlotSummary,
  targetId: string,
  event: Event,
): void {
  const checked = (event.target as HTMLInputElement).checked
  const selections = cardTargets.value[cardId] ?? {}
  const current = selections[slot.selector_id] ?? []
  let next: string[]
  if (slot.max === 1) {
    next = checked ? [targetId] : []
  } else if (checked && current.length < slot.max) {
    next = [...current, targetId]
  } else if (!checked) {
    next = current.filter((id) => id !== targetId)
  } else {
    next = current
  }
  cardTargets.value = {
    ...cardTargets.value,
    [cardId]: { ...selections, [slot.selector_id]: next },
  }
}

function cardIsReady(intent: LegalPlayCardSummary): boolean {
  return intent.target_slots.every((slot) => {
    const count = selectedTargets(intent.card_id, slot.selector_id).length
    return count >= slot.min && count <= slot.max
  })
}

function submitCard(intent: LegalPlayCardSummary): void {
  if (props.commandsDisabled || !cardIsReady(intent)) {
    return
  }
  emit(
    'playCard',
    intent.card_id,
    intent.target_slots.map((slot) => ({
      selector_id: slot.selector_id,
      target_ids: [...selectedTargets(intent.card_id, slot.selector_id)],
    })),
  )
}

function attackAmount(intent: LegalAttackSummary): number {
  return attackAmounts.value[intent.villain_id] ?? intent.max_amount
}

function attackOptions(maximum: number): number[] {
  return Array.from({ length: maximum }, (_, index) => index + 1)
}

function updateAttackAmount(villainId: string, event: Event): void {
  attackAmounts.value = {
    ...attackAmounts.value,
    [villainId]: Number((event.target as HTMLSelectElement).value),
  }
}

function submitAttack(intent: LegalAttackSummary): void {
  if (props.commandsDisabled) {
    return
  }
  emit('assignAttack', intent.villain_id, attackAmount(intent))
}

function canAcquire(cardId: string): boolean {
  return props.game.legal_intentions.acquire_cards.some((intent) => intent.card_id === cardId)
}
</script>

<template>
  <section class="game-table" aria-labelledby="game-table-heading">
    <header class="game-table__heading">
      <div>
        <p>Mesa oficial v{{ game.snapshot.state_version }}</p>
        <h3 id="game-table-heading" tabindex="-1">Sua fase de ação</h3>
      </div>
      <div class="hero-resource-ledger" aria-label="Seus recursos oficiais">
        <span>Vida {{ game.participant.resources.health }}</span>
        <strong>
          Ataque {{ game.participant.resources.attack }} · Influência
          {{ game.participant.resources.influence }}
        </strong>
      </div>
    </header>

    <div class="pile-ledger" aria-label="Pilhas da mesa">
      <span>Compra {{ game.table.draw_pile_count }}</span>
      <span>Descarte {{ game.table.discard_pile_count }}</span>
      <span>Hogwarts {{ game.table.hogwarts_deck_count }}</span>
      <span>Vilões {{ game.table.villain_deck_count }}</span>
    </div>

    <section class="table-zone" aria-labelledby="villains-heading">
      <div class="table-zone__heading">
        <h4 id="villains-heading">Vilões ativos</h4>
        <span>{{ game.table.active_villains.length }}</span>
      </div>
      <ul v-if="game.table.active_villains.length" class="table-list">
        <li
          v-for="villain in game.table.active_villains"
          :key="villain.instance_id"
          class="table-row"
          :class="{ 'table-row--pending': pendingVillainId === villain.instance_id }"
        >
          <div class="table-row__identity">
            <strong>{{ villain.name }}</strong>
            <span>Vida {{ villain.health }}</span>
            <small v-if="pendingVillainId === villain.instance_id">Intenção enviada</small>
          </div>
          <div v-if="attackIntent(villain.instance_id)" class="table-row__action">
            <label :for="`attack-${villain.instance_id}`">Ataque</label>
            <select
              :id="`attack-${villain.instance_id}`"
              :disabled="commandsDisabled"
              :value="attackAmount(attackIntent(villain.instance_id)!)"
              @change="updateAttackAmount(villain.instance_id, $event)"
            >
              <option
                v-for="amount in attackOptions(attackIntent(villain.instance_id)!.max_amount)"
                :key="amount"
                :value="amount"
              >
                {{ amount }}
              </option>
            </select>
            <button
              class="table-action"
              type="button"
              :aria-label="`Atacar ${villain.name} com ${attackAmount(attackIntent(villain.instance_id)!)}`"
              :disabled="commandsDisabled"
              @click="submitAttack(attackIntent(villain.instance_id)!)"
            >
              Atacar com {{ attackAmount(attackIntent(villain.instance_id)!) }}
            </button>
          </div>
          <span v-else class="table-row__note">
            {{ villain.health === 0 ? 'Sem vida restante' : 'Ataque indisponível' }}
          </span>
        </li>
      </ul>
      <p v-else class="table-empty">Nenhum vilão ativo.</p>
    </section>

    <section class="table-zone" aria-labelledby="hand-heading">
      <div class="table-zone__heading">
        <h4 id="hand-heading">Sua mão</h4>
        <span>{{ game.table.hand.length }}</span>
      </div>
      <ul v-if="game.table.hand.length" class="table-list">
        <li
          v-for="card in game.table.hand"
          :key="card.instance_id"
          class="table-row table-row--card"
          :class="{ 'table-row--pending': pendingCardId === card.instance_id }"
        >
          <div class="table-row__identity">
            <strong>{{ card.name }}</strong>
            <span>Na mão</span>
            <small v-if="pendingCardId === card.instance_id">Intenção enviada</small>
          </div>
          <template v-if="playableCard(card.instance_id)">
            <fieldset
              v-for="slot in playableCard(card.instance_id)!.target_slots"
              :key="slot.selector_id"
              class="target-selector"
            >
              <legend>
                Alvo da carta
                <span v-if="slot.min === 0 && slot.max === 1">(opcional)</span>
                <span v-else-if="slot.min !== 1 || slot.max !== 1">
                  ({{ slot.min }} a {{ slot.max }})
                </span>
              </legend>
              <label
                v-for="option in slot.options"
                :key="option.target_id"
                class="target-option"
                :class="{ 'target-option--pending': pendingTargetIds.has(option.target_id) }"
              >
                <input
                  :checked="targetIsSelected(card.instance_id, slot.selector_id, option.target_id)"
                  :disabled="targetIsDisabled(card.instance_id, slot, option.target_id)"
                  :name="`target-${card.instance_id}-${slot.selector_id}`"
                  :type="slot.min === 1 && slot.max === 1 ? 'radio' : 'checkbox'"
                  @change="updateTarget(card.instance_id, slot, option.target_id, $event)"
                />
                <span>{{ option.label }}</span>
                <small v-if="pendingTargetIds.has(option.target_id)">Alvo enviado</small>
              </label>
            </fieldset>
            <button
              class="table-action table-action--wide"
              type="button"
              :disabled="commandsDisabled || !cardIsReady(playableCard(card.instance_id)!)"
              @click="submitCard(playableCard(card.instance_id)!)"
            >
              Jogar {{ card.name }}
            </button>
          </template>
          <span v-else class="table-row__note">Carta sem ação legal agora</span>
        </li>
      </ul>
      <p v-else class="table-empty">Sua mão está vazia.</p>
    </section>

    <section class="table-zone" aria-labelledby="play-area-heading">
      <div class="table-zone__heading">
        <h4 id="play-area-heading">Área de jogo</h4>
        <span>{{ game.table.play_area.length }}</span>
      </div>
      <ul v-if="game.table.play_area.length" class="table-list table-list--compact">
        <li v-for="card in game.table.play_area" :key="card.instance_id" class="table-row">
          <div class="table-row__identity">
            <strong>{{ card.name }}</strong>
            <span>Em jogo</span>
          </div>
        </li>
      </ul>
      <p v-else class="table-empty">Nenhuma carta jogada nesta fase.</p>
    </section>

    <section class="table-zone" aria-labelledby="market-heading">
      <div class="table-zone__heading">
        <h4 id="market-heading">Mercado de Hogwarts</h4>
        <span>{{ game.table.market.length }}</span>
      </div>
      <ul v-if="game.table.market.length" class="table-list">
        <li
          v-for="card in game.table.market"
          :key="card.instance_id"
          class="table-row"
          :class="{ 'table-row--pending': pendingMarketCardId === card.instance_id }"
        >
          <div class="table-row__identity">
            <strong>{{ card.name }}</strong>
            <span>Custo {{ card.cost }}</span>
            <small v-if="pendingMarketCardId === card.instance_id">Intenção enviada</small>
          </div>
          <button
            v-if="canAcquire(card.instance_id)"
            class="table-action"
            type="button"
            :aria-label="`Adquirir ${card.name} por ${card.cost} de Influência`"
            :disabled="commandsDisabled"
            @click="emit('acquireCard', card.instance_id)"
          >
            Adquirir por {{ card.cost }}
          </button>
          <span v-else class="table-row__note">Influência insuficiente</span>
        </li>
      </ul>
      <p v-else class="table-empty">O mercado está vazio.</p>
    </section>
  </section>
</template>
