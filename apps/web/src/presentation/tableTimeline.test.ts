import { describe, expect, it } from 'vitest'
import type { RealtimeGameEvent } from '../contracts/identity-access.generated'
import { createTableTimeline } from './tableTimeline'

function played(sequence: number): RealtimeGameEvent {
  return {
    type: 'card_played', event_version: 6, sequence, state_version: sequence + 1,
    actor_position: 1, turn: 1, card_id: 'test-card', targets: [],
    effect_stop: 'stable', prng_counter: 0,
    effects: [
      { type: 'moved', rule_id: 'system:play-card', target_id: 'test-card', target_position: 1,
        from: 'hero_hand', to: 'hero_play_area' },
      { type: 'resource_changed', resource: 'health', before: 10, after: 8,
        target_id: 'hero:1', target_position: 1, rule_id: 'test-damage', cause: 'effect' },
      { type: 'resource_changed', resource: 'health', before: 8, after: 10,
        target_id: 'hero:1', target_position: 1, rule_id: 'test-heal', cause: 'effect' },
    ],
  }
}

describe('confirmed table presentation', () => {
  it('presents damage and healing separately in canonical order, once per game/sequence/index', () => {
    const timeline = createTableTimeline()
    timeline.reset('game-a', 0)
    timeline.append('game-a', [played(2)], 0)
    expect(timeline.take(0)).toBeNull()
    timeline.append('game-a', [played(1)], 10)
    timeline.append('game-a', [played(1), played(2)], 20)
    const groups = Array.from({ length: 6 }, () => timeline.take(30))
    expect(groups.map(group => group?.key)).toEqual([
      'game-a:1:0', 'game-a:1:1', 'game-a:1:2',
      'game-a:2:0', 'game-a:2:1', 'game-a:2:2',
    ])
    expect(groups.slice(0, 3).map(group => group?.cues[0]?.kind)).toEqual(['play', 'damage', 'heal'])
    expect(groups[1]?.cues[0]).toMatchObject({ before: 10, after: 8, amount: 2, position: 1 })
    expect(timeline.take(30)).toBeNull()
    timeline.reset('game-b', 0)
    timeline.append('game-a', [played(1)], 40)
    expect(timeline.take(40)).toBeNull()
    timeline.append('game-b', [played(1)], 40)
    expect(timeline.take(40)?.key).toBe('game-b:1:0')
  })

  it('bounds backlog to twenty groups and summarizes expired motion after two seconds without netting health', () => {
    const timeline = createTableTimeline()
    timeline.reset('game-a', 0)
    timeline.append('game-a', Array.from({ length: 10 }, (_, index) => played(index + 1)), 0)
    const groups = []
    for (let group = timeline.take(0); group; group = timeline.take(0)) groups.push(group)
    expect(groups.length).toBeLessThanOrEqual(20)
    expect(groups.reduce((sum, group) => sum + (group.summarized || 1), 0)).toBe(30)
    timeline.append('game-a', [played(11)], 10)
    const summary = timeline.take(2010)
    expect(summary?.summarized).toBe(3)
    expect(summary?.cues).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: 'damage', amount: 2, position: 1 }),
      expect.objectContaining({ kind: 'heal', amount: 2, position: 1 }),
    ]))
    expect(timeline.take(2010)).toBeNull()
    timeline.reset('game-a', 12)
    timeline.append('game-a', [played(11), played(12)], 2020)
    expect(timeline.take(2020)).toBeNull()
  })

  it('includes end-turn outcomes and every phase with distinct canonical indices', () => {
    const timeline = createTableTimeline()
    timeline.reset('game', 0)
    const event: RealtimeGameEvent = {
      type: 'turn_completed', event_version: 6, sequence: 1, state_version: 2, turn: 1,
      actor_position: 1, prng_counter: 0,
      end_turn: [
        { type: 'card_moved', card_id: 'old-card', from: 'hero_hand', to: 'hero_discard_pile' },
        { type: 'hero_recovered', position: 1, before: 0, after: 10 },
        { type: 'location_advanced', location_id: 'location-one', next_location_id: 'location-two' },
      ],
      steps: [
        { phase: 'end_turn', effects: [] },
        { phase: 'dark_arts', effects: [
          { type: 'resource_changed', rule_id: 'test', target_id: 'hero:2', target_position: 2,
            resource: 'health', before: 1, after: 0, cause: 'effect' },
        ] },
        { phase: 'villains', effects: [{ type: 'terminal', rule_id: 'test', outcome: 'lost' }] },
      ],
      control: { active_position: 2, turn: 2, phase: 'villains', status: 'lost', queued_phases: [],
        queued_effect_count: 0, decision_point: { type: 'none' } },
    }
    timeline.append('game', [event, event], 0)
    const groups = []
    for (let group = timeline.take(0); group; group = timeline.take(0)) groups.push(group)
    expect(groups.map(group => group.key)).toEqual(Array.from({ length: 8 }, (_, index) => `game:1:${index}`))
    expect(groups.flatMap(group => group.cues.map(cue => cue.kind))).toEqual([
      'discard', 'heal', 'location_lost', 'phase', 'phase', 'damage', 'stun', 'phase', 'lost',
    ])
  })

  it('bounds a visual gap even when HTTP has advanced beyond the missing WebSocket events', () => {
    const timeline = createTableTimeline()
    timeline.reset('game', 0)
    timeline.append('game', [played(2)], 0)
    expect(timeline.take(1999)).toBeNull()
    expect(timeline.take(2000)?.summarized).toBe(3)
    timeline.append('game', [played(1), played(2)], 2100)
    expect(timeline.take(2100)).toBeNull()
  })
})
