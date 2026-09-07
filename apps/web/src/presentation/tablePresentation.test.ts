import { describe, expect, it } from 'vitest'

import { createTablePresentation, type TableIntent, type TableState } from './tablePresentation'

const state = (): TableState => ({
  version: 3,
  disabled: false,
  cards: [{ id: 'wand-1', name: 'Varinha', zone: 'hand', description: 'Ganhe 1 de Ataque.' }],
  legal: {
    end_hero_actions: true,
    play_cards: [{ card_id: 'wand-1', target_slots: [] }],
    assign_attack: [],
    acquire_cards: [],
  },
})

describe('table presentation', () => {
  it('inspects locally and only sends a legal play after explicit confirmation', () => {
    const intentions: TableIntent[] = []
    const table = createTablePresentation(state(), (intent) => intentions.push(intent))
    table.select('wand-1')
    expect(table.view().selected?.description).toBe('Ganhe 1 de Ataque.')
    expect(intentions).toEqual([])
    table.confirm()
    expect(intentions).toEqual([{ type: 'play_card', cardId: 'wand-1', targets: [] }])
    table.confirm()
    expect(intentions).toHaveLength(1)
  })

  it('requires the official target cardinality and invalidates a decision after another device acts', () => {
    const intentions: TableIntent[] = []
    const projection = state()
    projection.legal.play_cards[0]!.target_slots = [{
      selector_id: 'ally', min: 1, max: 1,
      options: [{ target_id: 'hero:2', label: 'Hermione' }],
    }]
    const table = createTablePresentation(projection, (intent) => intentions.push(intent))
    table.select('wand-1')
    table.confirm()
    expect(intentions).toEqual([])
    table.toggleTarget('ally', 'hero:3')
    table.confirm()
    expect(intentions).toEqual([])
    table.toggleTarget('ally', 'hero:2')
    expect(table.view().canConfirm).toBe(true)
    table.update({ ...projection, version: 4 })
    table.confirm()
    expect(intentions).toEqual([])
    expect(table.view().selected).toBeNull()
    table.select('wand-1')
    table.toggleTarget('ally', 'hero:2')
    table.confirm()
    expect(intentions).toEqual([{
      type: 'play_card', cardId: 'wand-1',
      targets: [{ selector_id: 'ally', target_ids: ['hero:2'] }],
    }])
  })

  it('clamps attack to the legal amount and only acquires to an offered destination', () => {
    const intentions: TableIntent[] = []
    const projection = state()
    projection.cards.push(
      { id: 'draco', name: 'Draco', description: 'Vilão', zone: 'villain', health: 6 },
      { id: 'owl', name: 'Coruja', description: 'Aliado', zone: 'market', cost: 2 },
    )
    projection.legal.assign_attack = [{ villain_id: 'draco', max_amount: 3 }]
    projection.legal.acquire_cards = [{ card_id: 'owl', cost: 2, destinations: ['draw_pile'] }]
    const table = createTablePresentation(projection, (intent) => intentions.push(intent))
    table.select('draco')
    table.setAmount(99)
    table.confirm()
    expect(intentions).toEqual([{ type: 'assign_attack', villainId: 'draco', amount: 3 }])
    table.update({ ...projection, disabled: true })
    table.update({ ...projection, disabled: false })
    table.select('owl')
    table.setDestination('discard_pile')
    table.confirm()
    expect(intentions[1]).toEqual({ type: 'acquire_card', cardId: 'owl', destination: 'draw_pile' })
  })

  it('uses the same intention for a valid drop and cancels invalid, interrupted or obsolete drags', () => {
    const intentions: TableIntent[] = []
    const table = createTablePresentation(state(), (intent) => intentions.push(intent))
    table.beginDrag('wand-1')
    table.drop('market')
    expect(table.view().selected).toBeNull()
    table.beginDrag('wand-1')
    table.cancel()
    table.drop('play')
    table.beginDrag('wand-1')
    table.update({ ...state(), version: 4 })
    table.drop('play')
    expect(intentions).toEqual([])
    table.beginDrag('wand-1')
    table.drop('play')
    expect(intentions).toEqual([{ type: 'play_card', cardId: 'wand-1', targets: [] }])
  })
})
