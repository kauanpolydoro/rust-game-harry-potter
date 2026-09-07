import type { GameProjectionResponse } from '../contracts/identity-access.generated'

/** Fictional demonstration data. Never imported by the live game's entry point. */
export function prototypeGame(): GameProjectionResponse {
  const participants: GameProjectionResponse['participants'] = ['Harry', 'Hermione', 'Ron', 'Neville'].map((name, index) => ({
    position: index + 1, display_name: ['Minerva', 'Luna', 'Cedrico', 'Ginny'][index]!,
    hero: { id: (['harry', 'hermione', 'ron', 'neville'] as const)[index]!, name },
    resources: { attack: 0, influence: 0, health: 10 - index },
    role: index === 0 ? 'host' : 'guest', hand_count: 5, stunned: false,
  }))
  const game: GameProjectionResponse = {
    game: { id: 'prototype-only', adventure: { id: 'adventure:001', name: 'Protótipo do Jogo 1' }, status: 'in_progress', expires_at: '2099-01-01T00:00:00Z' },
    snapshot: { state_version: 1, sequence: 0, cursor: 0, snapshot_version: 1, digest: 'prototype-only', versions: {
      content: 'prototype', manifest: 1, manifest_digest: 'prototype', prng: 'chacha20-v1', ruleset: 'prototype', sampling: 'rejection-sampling-v1', shuffle: 'fisher-yates-v1',
    } },
    participant: participants[0]!, participants,
    turn: { active_position: 1, number: 1, phase: 'hero_actions' },
    choice: { status: 'none' }, queued_effect_count: 0, queued_phases: [], effects: { status: 'idle', outcomes: [] },
    legal_actions: ['play_card', 'end_hero_actions'],
    legal_intentions: { end_hero_actions: true, play_cards: [], assign_attack: [], acquire_cards: [] },
    table: {
      hand: [
        { instance_id: 'spell-1', catalog_id: 'prototype-spell', name: 'Alohomora!', description: 'Ganhe 1 de Influência.\n\nCarta demonstrativa para avaliar seleção, leitura e passagem da Mão à mesa.' },
        { instance_id: 'spell-2', catalog_id: 'prototype-spell', name: 'Alohomora!', description: 'Ganhe 1 de Influência.' },
        { instance_id: 'wand', catalog_id: 'prototype-wand', name: 'Varinha', description: 'Ganhe 3 de Ataque. Depois, selecione um Vilão e a quantidade que deseja atribuir.' },
        { instance_id: 'owl', catalog_id: 'prototype-owl', name: 'Edwiges', description: 'Escolha um Herói para recuperar 1 de Vida. O alvo é uma seleção local até confirmar a jogada.' },
        { instance_id: 'book', catalog_id: 'prototype-book', name: 'História de Hogwarts', description: 'Ganhe 2 de Influência.' },
      ],
      play_area: [], draw_pile_count: 5, discard_pile_count: 0, hogwarts_deck_count: 25,
      market: ['Essência de Ditamno', 'Incendio', 'Hagrid', 'Reparo', 'Lumos', 'Poção Polissuco'].map((name, index) => ({
        instance_id: `market-${index}`, catalog_id: `prototype-market-${index}`, name, cost: index % 3 + 2, affordable: false,
        description: 'Carta demonstrativa do mercado. A aquisição vai ao destino escolhido e o espaço é reposto pela próxima carta.',
      })),
      active_villains: [{ instance_id: 'draco', catalog_id: 'prototype-draco', name: 'Draco Malfoy', health: 6, attackable: false, max_attack: 0,
        description: 'Um adversário ocupa esta zona da mesa. A Vida muda apenas quando a ação é confirmada.', reward_description: 'Recompensa demonstrativa: a equipe remove 1 de Controle.' }],
      current_location: { instance_id: 'location', catalog_id: 'prototype-location', name: 'Beco Diagonal', control: 2, control_limit: 4, dark_arts_count: 1 },
      location_deck_count: 1, location_discard_count: 0, villain_deck_count: 2, villain_discard_count: 0,
      revealed_dark_arts: { instance_id: 'dark-arts', catalog_id: 'prototype-dark-arts', name: 'Petrificação', description: 'O Herói ativo perde 1 de Vida. Resolução demonstrativa no início do próximo turno.' },
    },
  }
  refreshPrototypeIntentions(game)
  return game
}

export function refreshPrototypeIntentions(game: GameProjectionResponse) {
  game.legal_intentions = {
    end_hero_actions: true,
    play_cards: game.table.hand.map((card) => ({ card_id: card.instance_id, target_slots: card.instance_id === 'owl'
      ? [{ selector_id: 'hero', min: 1, max: 1, options: game.participants.map((hero) => ({ target_id: `hero:${hero.position}`, label: hero.hero.name })) }] : [] })),
    assign_attack: game.participant.resources.attack > 0 ? game.table.active_villains.filter((v) => v.health > 0).map((v) => ({ villain_id: v.instance_id, max_amount: Math.min(v.health, game.participant.resources.attack) })) : [],
    acquire_cards: game.table.market.filter((card) => card.cost <= game.participant.resources.influence).map((card) => ({ card_id: card.instance_id, cost: card.cost, destinations: ['discard_pile', 'draw_pile'] })),
  }
}
