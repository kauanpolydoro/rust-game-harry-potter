import type { EffectOutcomeSummary, EndTurnOutcomeSummary, RealtimeGameEvent } from '../contracts/identity-access.generated'

export interface TableCue {
  kind: 'play' | 'damage' | 'heal' | 'resource' | 'draw' | 'discard' | 'acquire' | 'move'
    | 'stun' | 'villain_defeated' | 'location_lost' | 'won' | 'lost' | 'phase' | 'shuffle' | 'drawing_blocked' | 'drawing_restored'
  target: string
  position?: number
  resource?: 'health' | 'attack' | 'influence' | 'control'
  before?: number
  after?: number
  amount?: number
  from?: string
  to?: string
  phase?: string
  source?: string
  direction?: 'gain' | 'loss'
}
export interface TableVisualGroup {
  key: string
  sequence: number
  index: number
  arrivedAt: number
  cues: TableCue[]
  summarized: number
}

function effectCues(effect: EffectOutcomeSummary): TableCue[] {
  if (effect.type === 'moved') return [movement(effect.target_id, effect.from, effect.to, effect.target_position)]
  if (effect.type === 'terminal') return [{ kind: effect.outcome, target: 'table' }]
  if (effect.type === 'drawing_blocked') return [{ kind: 'drawing_blocked', target: effect.target_id, position: effect.target_position }]
  if (effect.type !== 'resource_changed' || effect.before === effect.after) return []
  const cues: TableCue[] = [{ kind: effect.resource === 'health' ? effect.after < effect.before ? 'damage' : 'heal' : 'resource',
    target: effect.target_id, ...(effect.target_position === undefined ? {} : { position: effect.target_position }), resource: effect.resource,
    before: effect.before, after: effect.after, amount: Math.abs(effect.after - effect.before),
    direction: effect.after > effect.before ? 'gain' : 'loss' }]
  if (effect.resource === 'health' && effect.after === 0 && effect.target_position !== undefined) {
    cues.push({ kind: 'stun', target: effect.target_id, position: effect.target_position })
  }
  return cues
}

function movement(target: string, from: string, to: string, position?: number): TableCue {
  const kind = to === 'hero_play_area' ? 'play' : from === 'market' ? 'acquire'
    : to === 'hero_hand' ? 'draw' : to === 'hero_discard_pile' ? 'discard'
      : to === 'villain_discard' ? 'villain_defeated' : to === 'location_discard' ? 'location_lost' : 'move'
  return { kind, target, from, to, ...(position === undefined ? {} : { position }) }
}

function endTurnCues(outcome: EndTurnOutcomeSummary, actor: number): TableCue[] {
  switch (outcome.type) {
    case 'card_moved': return [movement(outcome.card_id, outcome.from, outcome.to, actor)]
    case 'hero_recovered': return [{ kind: 'heal', target: `hero:${outcome.position}`, position: outcome.position,
      resource: 'health', before: outcome.before, after: outcome.after, amount: 10 }]
    case 'resource_reset': return outcome.before ? [{ kind: 'resource', target: `hero:${actor}`, position: actor,
      resource: outcome.resource, before: outcome.before, after: 0, amount: outcome.before, direction: 'loss' }] : []
    case 'location_advanced': return [{ kind: 'location_lost', target: outcome.location_id, from: 'active_location', to: 'location_discard' }]
    case 'villain_revealed': return [movement(outcome.villain_id, 'villain_deck', 'active_villains')]
    case 'pile_shuffled': return [{ kind: 'shuffle', target: 'hero_draw_pile', position: outcome.owner_position }]
    case 'drawing_restored': return [{ kind: 'drawing_restored', target: `hero:${outcome.position}`, position: outcome.position }]
  }
}

function groupsFor(gameId: string, event: RealtimeGameEvent, arrivedAt: number): TableVisualGroup[] {
  const cues: TableCue[][] = []
  if ('end_turn' in event) cues.push(...event.end_turn.map(outcome => endTurnCues(outcome, event.actor_position)))
  if ('effects' in event) cues.push(...event.effects.map(effectCues))
  if ('steps' in event) {
    for (const step of event.steps) {
      cues.push([{ kind: 'phase', target: 'table', phase: step.phase }])
      cues.push(...step.effects.map(effectCues))
    }
  }
  const source = event.type === 'card_played' ? event.card_id : `hero:${event.actor_position}`
  return cues.map((items, index) => ({ key: `${gameId}:${event.sequence}:${index}`,
    sequence: event.sequence, index, arrivedAt, cues: items.map(cue => ({ ...cue, source })), summarized: 0 })).filter(group => group.cues.length)
}

function summarize(groups: TableVisualGroup[]): TableVisualGroup {
  const first = groups[0]!
  const combined = new Map<string, TableCue>()
  for (const group of groups) {
    for (const cue of group.cues) {
      // Keep signs separate, with bounded cardinality even for thousands of card movements.
      const key = `${cue.kind}:${cue.position ?? 'table'}:${cue.resource ?? ''}:${cue.direction ?? ''}`
      const previous = combined.get(key)
      const merged = { ...cue, target: cue.position ? `hero:${cue.position}` : 'table',
        amount: (previous?.amount ?? 0) + (cue.amount ?? 1) }
      delete merged.before
      delete merged.after
      combined.set(key, merged)
    }
  }
  return { ...first, key: `${first.key}:summary`, cues: [...combined.values()],
    summarized: groups.reduce((count, group) => count + (group.summarized || 1), 0) }
}

function bound(groups: TableVisualGroup[], now: number): void {
  let count = Math.max(0, groups.length - 19)
  while (count < groups.length && now - groups[count]!.arrivedAt >= 2000) count++
  if (count > 1 || (count === 1 && !groups[0]!.summarized)) {
    groups.splice(0, count, summarize(groups.slice(0, count)))
  }
}

/** Visual cursor is independent of the HTTP projection. Only confirmed events enter here. */
export function createTableTimeline() {
  let gameId = ''
  let cursor = 0
  const pending = new Map<number, { groups: TableVisualGroup[]; arrivedAt: number }>()
  const ready: TableVisualGroup[] = []
  function boundPending(now: number) {
    const entries = [...pending.entries()].sort(([left], [right]) => left - right)
    const count = entries.reduce((total, [, event]) => total + Math.max(1, event.groups.length), ready.length)
    if (count <= 20 && !entries.some(([, event]) => now - event.arrivedAt >= 2000)) return
    if (!entries.length) return
    // A visual gap cannot hold motion indefinitely. Summarize known consequences and
    // retire the obsolete range, without changing state or recovering the connection.
    const groups = [...ready, ...entries.flatMap(([, event]) => event.groups)]
    cursor = entries.at(-1)![0]
    pending.clear()
    ready.length = 0
    if (groups.length) ready.push(summarize(groups))
  }
  return {
    reset(id: string, through: number) {
      gameId = id
      cursor = through
      pending.clear()
      ready.length = 0
    },
    append(id: string, events: RealtimeGameEvent[], now: number) {
      if (id !== gameId) return
      for (const event of events) {
        if (event.sequence <= cursor || pending.has(event.sequence)) continue
        pending.set(event.sequence, { groups: groupsFor(id, event, now), arrivedAt: now })
        while (pending.has(cursor + 1)) {
          cursor++
          ready.push(...pending.get(cursor)!.groups)
          pending.delete(cursor)
        }
        bound(ready, now)
        boundPending(now)
      }
    },
    take(now: number): TableVisualGroup | null {
      boundPending(now)
      bound(ready, now)
      return ready.shift() ?? null
    },
  }
}
