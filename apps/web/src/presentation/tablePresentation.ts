import type {
  CardAcquisitionDestination,
  EffectTargetBinding,
  LegalIntentionsSummary,
} from '../contracts/identity-access.generated'

export type TableZone = 'hand' | 'play' | 'market' | 'villain' | 'location' | 'dark_arts'
export interface TableCard {
  id: string
  catalogId?: string
  name: string
  description: string
  zone: TableZone
  cost?: number
  health?: number
  detail?: string
}
export interface TableState {
  version: number
  disabled: boolean
  cards: TableCard[]
  legal: LegalIntentionsSummary
}
export type TableIntent =
  | { type: 'play_card'; cardId: string; targets: EffectTargetBinding[] }
  | { type: 'assign_attack'; villainId: string; amount: number }
  | { type: 'acquire_card'; cardId: string; destination: CardAcquisitionDestination }

/** Local decisions only. The caller sends intentions through the command store. */
export function createTablePresentation(initial: TableState, send: (intent: TableIntent) => void) {
  let state = initial
  let selectedId: string | null = null
  let submitted = false
  let targets: Record<string, string[]> = {}
  let amount = 1
  let destination: CardAcquisitionDestination = 'discard_pile'
  let dragging = false

  function selected() {
    return state.cards.find((card) => card.id === selectedId) ?? null
  }
  function cancel() {
    selectedId = null
    targets = {}
    dragging = false
  }
  function playIntent() {
    return state.legal.play_cards.find((intent) => intent.card_id === selectedId)
  }
  function attackIntent() {
    return state.legal.assign_attack.find((intent) => intent.villain_id === selectedId)
  }
  function acquisitionIntent() {
    return state.legal.acquire_cards.find((intent) => intent.card_id === selectedId)
  }
  function canConfirm() {
    if (state.disabled || submitted) return false
    const attack = attackIntent()
    if (attack) return Number.isInteger(amount) && amount >= 1 && amount <= attack.max_amount
    const acquisition = acquisitionIntent()
    if (acquisition) return acquisition.destinations.includes(destination)
    const play = playIntent()
    return !state.disabled && !submitted && Boolean(play && play.target_slots.every((slot) => {
      const chosen = targets[slot.selector_id] ?? []
      return chosen.length >= slot.min && chosen.length <= slot.max &&
        chosen.every((id) => slot.options.some((option) => option.target_id === id))
    }))
  }
  return {
    view: () => ({ selected: selected(), blocked: state.disabled || submitted,
      canConfirm: canConfirm(), dragging, targets: { ...targets }, slots: playIntent()?.target_slots ?? [],
      amount, maxAmount: attackIntent()?.max_amount ?? 0,
      destination, destinations: acquisitionIntent()?.destinations ?? [] }),
    select(id: string) {
      targets = {}
      selectedId = state.cards.some((card) => card.id === id) ? id : null
      amount = attackIntent()?.max_amount ?? 1
      destination = acquisitionIntent()?.destinations[0] ?? 'discard_pile'
    },
    beginDrag(id: string) {
      if (state.disabled || submitted || !state.legal.play_cards.some((intent) => intent.card_id === id)) return
      this.select(id)
      dragging = true
    },
    drop(zone: TableZone | null) {
      if (!dragging) return
      dragging = false
      if (zone !== 'play') { cancel(); return }
      this.confirm()
    },
    setAmount(value: number) {
      if (Number.isFinite(value)) amount = Math.max(1, Math.min(Math.trunc(value), attackIntent()?.max_amount ?? 1))
    },
    setDestination(value: CardAcquisitionDestination) {
      if (acquisitionIntent()?.destinations.includes(value)) destination = value
    },
    toggleTarget(selectorId: string, targetId: string) {
      if (state.disabled || submitted) return
      const slot = playIntent()?.target_slots.find((candidate) => candidate.selector_id === selectorId)
      if (!slot?.options.some((option) => option.target_id === targetId)) return
      const previous = targets[selectorId] ?? []
      targets[selectorId] = previous.includes(targetId) ? previous.filter((id) => id !== targetId)
        : slot.max === 1 ? [targetId]
          : previous.length < slot.max ? [...previous, targetId] : previous
    },
    cancel,
    update(next: TableState) {
      if (state.disabled && !next.disabled) submitted = false
      if (next.version !== state.version) {
        cancel()
        submitted = false
      }
      state = next
    },
    confirm() {
      if (!canConfirm()) return
      const card = selected()
      if (!card) return
      if (attackIntent()) {
        submitted = true
        send({ type: 'assign_attack', villainId: card.id, amount })
        return
      }
      if (acquisitionIntent()) {
        submitted = true
        send({ type: 'acquire_card', cardId: card.id, destination })
        return
      }
      const play = playIntent()
      if (!card || !play) return
      submitted = true
      send({ type: 'play_card', cardId: card.id, targets: play.target_slots.map((slot) => ({
        selector_id: slot.selector_id, target_ids: [...(targets[slot.selector_id] ?? [])],
      })) })
    },
  }
}
