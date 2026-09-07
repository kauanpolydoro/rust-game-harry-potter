import type { GameProjectionResponse } from '../contracts/identity-access.generated'
import type { TableCard, TableState } from './tablePresentation'

export function tableState(game: GameProjectionResponse, disabled: boolean): TableState {
  const cards: TableCard[] = []
  for (const [zone, collection] of [
    ['hand', game.table.hand], ['play', game.table.play_area],
    ['market', game.table.market], ['villain', game.table.active_villains],
  ] as const) {
    for (const card of collection) {
      cards.push({ id: card.instance_id, catalogId: card.catalog_id, name: card.name, zone,
        description: [card.description, 'reward_description' in card ? card.reward_description : ''].filter(Boolean).join('\n\n'),
        ...('cost' in card ? { cost: card.cost } : {}),
        ...('health' in card ? { health: card.health } : {}),
      })
    }
  }
  if (game.table.revealed_dark_arts) {
    const card = game.table.revealed_dark_arts
    cards.push({ id: card.instance_id, catalogId: card.catalog_id, name: card.name, description: card.description ?? '', zone: 'dark_arts' })
  }
  if (game.table.current_location) {
    const location = game.table.current_location
    cards.push({ id: 'current-location', catalogId: location.catalog_id, name: location.name, zone: 'location',
      detail: `Controle ${location.control}/${location.control_limit}`,
      description: `Controle ${location.control} de ${location.control_limit}. Locais restantes: ${game.table.location_deck_count}.`,
    })
  }
  return { version: game.snapshot.state_version, disabled, cards, legal: game.legal_intentions }
}
