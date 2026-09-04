import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { GameProjectionResponse } from '../contracts/identity-access.generated'
import { useGameCommandStore } from './gameCommand'

const gameId = 'dc8213d3-2941-4ef0-9ce8-b97cc6623410'

function projection(stateVersion = 1): GameProjectionResponse {
  const participant = {
    display_name: 'Minerva',
    hand_count: 1,
    hero: { id: 'harry' as const, name: 'Harry' },
    position: 1,
    resources: { attack: 2, health: 10, influence: 3 },
    role: 'host' as const,
  }

  return {
    choice: { status: 'none' },
    effects: { outcomes: [], status: 'idle' },
    game: {
      adventure: { id: 'adventure:fixture', name: 'Fixture' },
      expires_at: '2026-09-10T12:00:00Z',
      id: gameId,
      status: 'in_progress',
    },
    legal_actions: ['end_hero_actions', 'play_card', 'assign_attack', 'acquire_card'],
    legal_intentions: {
      acquire_cards: [{ card_id: 'card:market-one', cost: 3 }],
      assign_attack: [{ max_amount: 2, villain_id: 'villain:one' }],
      end_hero_actions: true,
      play_cards: [
        {
          card_id: 'card:starter-one',
          target_slots: [
            {
              max: 1,
              min: 1,
              options: [{ label: 'Luna', target_id: 'hero:2' }],
              selector_id: 'target:ally',
            },
          ],
        },
      ],
    },
    queued_effect_count: 0,
    queued_phases: ['end_turn'],
    participant,
    participants: [participant],
    snapshot: {
      cursor: stateVersion - 1,
      digest: `blake3:${'c'.repeat(64)}`,
      sequence: stateVersion - 1,
      snapshot_version: 1,
      state_version: stateVersion,
      versions: {
        content: 'fixture-v1',
        manifest: 1,
        manifest_digest: `blake3:${'b'.repeat(64)}`,
        prng: 'chacha20-v1',
        ruleset: 'fixture-rules-v1',
        sampling: 'rejection-sampling-v1',
        shuffle: 'fisher-yates-v1',
      },
    },
    table: {
      active_villains: [
        {
          attackable: true,
          catalog_id: 'villain:fixture',
          health: 4,
          instance_id: 'villain:one',
          max_attack: 2,
          name: 'Fixture Villain',
        },
      ],
      discard_pile_count: 0,
      draw_pile_count: 0,
      hand: [
        {
          catalog_id: 'starter:fixture',
          instance_id: 'card:starter-one',
          name: 'Fixture Starter',
        },
      ],
      hogwarts_deck_count: 1,
      market: [
        {
          affordable: true,
          catalog_id: 'hogwarts:fixture',
          cost: 3,
          instance_id: 'card:market-one',
          name: 'Fixture Market Card',
        },
      ],
      play_area: [],
      villain_deck_count: 0,
    },
    turn: { active_position: 1, number: 1, phase: 'hero_actions' },
  }
}

function errorResponse(code: string) {
  return {
    error: {
      category: 'request',
      code,
      correlation_id: gameId,
      details: {},
      message_key: `error.${code.toLowerCase()}`,
      retry: 'not_retryable',
    },
  }
}

function acceptedResponse(
  commandId: string,
  type: 'end_hero_actions' | 'play_card' | 'assign_attack' | 'acquire_card',
) {
  return {
    projection: projection(2),
    receipt: {
      accepted_sequence: 1,
      accepted_state_version: 2,
      command_id: commandId,
      expected_state_version: 1,
      expires_at: '2026-09-10T13:00:00Z',
      status: 'accepted',
      type,
    },
  }
}

describe('game command intentions', () => {
  beforeEach(() => {
    sessionStorage.clear()
    setActivePinia(createPinia())
  })

  afterEach(() => {
    sessionStorage.clear()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('keeps a targeted card play as an overlay until the server answers', async () => {
    const officialProjection = projection()
    const unchangedProjection = structuredClone(officialProjection)
    let answerRequest = (_response: Response): void => undefined
    const response = new Promise<Response>((resolve) => {
      answerRequest = resolve
    })
    const request = vi.fn().mockReturnValue(response)
    vi.stubGlobal('fetch', request)
    const store = useGameCommandStore()
    const targets = [{ selector_id: 'target:ally', target_ids: ['hero:2'] }]

    const result = store.playCard(officialProjection, 'card:starter-one', targets)

    expect(store.status).toBe('submitting')
    expect(store.pendingOverlay).toEqual({
      card_id: 'card:starter-one',
      targets,
      type: 'play_card',
    })
    expect(officialProjection).toEqual(unchangedProjection)
    const persisted = JSON.parse(
      sessionStorage.getItem('hogwarts.game-command.pending-intent') ?? '{}',
    ) as Record<string, unknown>
    expect(Object.keys(persisted).sort()).toEqual([
      'commandId',
      'commandType',
      'createdAt',
      'gameId',
    ])
    expect(persisted.commandType).toBe('play_card')
    expect(JSON.stringify(persisted)).not.toContain('card:starter-one')
    expect(JSON.stringify(persisted)).not.toContain('hero:2')

    const body = JSON.parse(String(request.mock.calls[0]?.[1]?.body)) as Record<string, unknown>
    expect(body).toMatchObject({
      card_id: 'card:starter-one',
      expected_state_version: 1,
      targets,
      type: 'play_card',
    })
    expect(body.command_id).toEqual(expect.any(String))

    answerRequest(
      new Response(JSON.stringify(errorResponse('GAME_ACTION_NOT_ALLOWED')), {
        headers: { 'Content-Type': 'application/json' },
        status: 422,
      }),
    )

    await expect(result).resolves.toBeNull()
    expect(store.status).toBe('failed')
    expect(store.pendingIntent).toBeNull()
    expect(store.pendingOverlay).toBeNull()
    expect(sessionStorage.getItem('hogwarts.game-command.pending-intent')).toBeNull()
    expect(officialProjection).toEqual(unchangedProjection)
  })

  it('submits an attack assignment with its explicit villain and amount', async () => {
    const request = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(errorResponse('GAME_ACTION_NOT_ALLOWED')), {
        headers: { 'Content-Type': 'application/json' },
        status: 422,
      }),
    )
    vi.stubGlobal('fetch', request)
    const store = useGameCommandStore()

    await store.assignAttack(projection(), 'villain:one', 2)

    const body = JSON.parse(String(request.mock.calls[0]?.[1]?.body)) as Record<string, unknown>
    expect(body).toMatchObject({
      amount: 2,
      expected_state_version: 1,
      type: 'assign_attack',
      villain_id: 'villain:one',
    })
  })

  it('returns only the official projection after an acquisition is accepted', async () => {
    let submittedCommandId = ''
    const request = vi.fn().mockImplementation((_input: RequestInfo | URL, init?: RequestInit) => {
      const body = JSON.parse(String(init?.body)) as { command_id: string }
      submittedCommandId = body.command_id
      return Promise.resolve(
        new Response(JSON.stringify(acceptedResponse(body.command_id, 'acquire_card')), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    })
    vi.stubGlobal('fetch', request)
    const store = useGameCommandStore()

    const result = await store.acquireCard(projection(), 'card:market-one')

    const body = JSON.parse(String(request.mock.calls[0]?.[1]?.body)) as Record<string, unknown>
    expect(body).toEqual({
      card_id: 'card:market-one',
      command_id: submittedCommandId,
      expected_state_version: 1,
      type: 'acquire_card',
    })
    expect(result).toEqual(projection(2))
    expect(store.status).toBe('accepted')
    expect(store.receipt?.type).toBe('acquire_card')
    expect(store.pendingIntent).toBeNull()
    expect(store.pendingOverlay).toBeNull()
    expect(sessionStorage.getItem('hogwarts.game-command.pending-intent')).toBeNull()
  })

  it('recovers only generic receipt metadata after an uncertain result and reload', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('response lost after commit')))
    const originalStore = useGameCommandStore()

    await originalStore.playCard(projection(), 'card:starter-one', [
      { selector_id: 'target:ally', target_ids: ['hero:2'] },
    ])

    expect(originalStore.status).toBe('uncertain')
    expect(originalStore.pendingOverlay).toEqual({
      card_id: 'card:starter-one',
      targets: [{ selector_id: 'target:ally', target_ids: ['hero:2'] }],
      type: 'play_card',
    })

    setActivePinia(createPinia())
    const reloadedStore = useGameCommandStore()

    expect(reloadedStore.status).toBe('uncertain')
    expect(reloadedStore.pendingIntent?.commandType).toBe('play_card')
    expect(reloadedStore.pendingOverlay).toBeNull()
    const serialized = sessionStorage.getItem('hogwarts.game-command.pending-intent') ?? ''
    expect(serialized).not.toContain('card:starter-one')
    expect(serialized).not.toContain('hero:2')
  })
})
