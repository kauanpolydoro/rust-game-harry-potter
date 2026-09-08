import type { TableCue } from './tableTimeline'

export function cueLabel(cue: TableCue): string {
  switch (cue.kind) {
    case 'damage': return `−${cue.amount} Vida`
    case 'heal': return `+${cue.amount} Vida`
    case 'resource': {
      const resource = { health: 'Vida', attack: 'Ataque', influence: 'Influência', control: 'Controle' }[cue.resource ?? 'health']
      return `${cue.direction === 'loss' ? '−' : '+'}${cue.amount} ${resource}`
    }
    case 'play': return 'Carta jogada'
    case 'draw': return 'Carta comprada'
    case 'discard': return 'Carta descartada'
    case 'acquire': return 'Carta adquirida'
    case 'move': return cue.to === 'market' ? 'Mercado reposto' : cue.to === 'active_villains' ? 'Vilão revelado' : 'Carta movida'
    case 'stun': return 'Atordoado'
    case 'villain_defeated': return 'Vilão derrotado'
    case 'location_lost': return 'Local perdido'
    case 'won': return 'Vitória da equipe!'
    case 'lost': return 'Derrota da equipe'
    case 'shuffle': return 'Baralho embaralhado'
    case 'drawing_blocked': return 'Compra extra bloqueada'
    case 'drawing_restored': return 'Compra extra liberada'
    case 'phase': return ({ dark_arts: 'Artes das Trevas', villains: 'Vilões', hero_actions: 'Ações do Herói', end_turn: 'Fim do turno' } as Record<string, string>)[cue.phase ?? ''] ?? 'Turno atualizado'
  }
}
