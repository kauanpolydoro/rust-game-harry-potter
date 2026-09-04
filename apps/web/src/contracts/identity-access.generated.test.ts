import { describe, expect, it } from 'vitest'

import {
  isErrorResponse,
  isExecuteGameCommandRequest,
  isGameProjectionResponse,
  isRealtimeGameEvent,
  isRealtimePresenceMessage,
  isRecoveredLobbyResponse,
  isRecoveryReplacementRequiredResponse,
} from './identity-access.generated'

function projection() {
  return {
    choice: { status: 'none' },
    effects: { outcomes: [], status: 'idle' },
    game: {
      adventure: { id: 'adventure:001', name: 'Game 1' },
      expires_at: '2026-09-10T12:00:00Z',
      id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
      status: 'in_progress',
    },
    legal_actions: ['complete_dark_arts'],
    legal_intentions: {
      acquire_cards: [],
      assign_attack: [],
      complete_dark_arts: true,
      play_cards: [],
    },
    participant: {
      display_name: 'Minerva',
      hero: { id: 'harry', name: 'Harry' },
      position: 1,
      resources: { attack: 0, health: 10, influence: 0 },
      role: 'host',
      hand_count: 0,
    },
    participants: [
      {
        display_name: 'Minerva',
        hero: { id: 'harry', name: 'Harry' },
        position: 1,
        resources: { attack: 0, health: 10, influence: 0 },
        role: 'host',
        hand_count: 0,
      },
    ],
    snapshot: {
      cursor: 0,
      digest: `blake3:${'c'.repeat(64)}`,
      sequence: 0,
      snapshot_version: 1,
      state_version: 1,
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
      active_villains: [],
      discard_pile_count: 0,
      draw_pile_count: 0,
      hand: [],
      hogwarts_deck_count: 0,
      market: [],
      play_area: [],
      villain_deck_count: 0,
    },
    turn: { active_position: 1, number: 1, phase: 'dark_arts' },
  }
}

describe('generated identity contract guards', () => {
  it('discriminates choice commands, choice summaries and their official events', () => {
    const commandId = 'dc8213d3-2941-4ef0-9ce8-b97cc6623410'
    const choice = {
      cause: 'rule:functional',
      id: 'rule:functional:effect:0',
      kind: 'effect',
      max: 1,
      min: 1,
      options: ['option:1', 'option:2'],
      responsible_position: 2,
      status: 'pending',
    }
    const command = {
      choice_id: choice.id,
      command_id: commandId,
      expected_state_version: 2,
      selected_options: ['option:2'],
      type: 'resolve_choice',
    }
    const darkArtsEvent = {
      actor_position: 1,
      effect_stop: 'stable',
      effects: [],
      event_version: 3,
      prng_counter: 0,
      sequence: 1,
      state_version: 2,
      turn: 1,
      type: 'dark_arts_completed',
    }
    const event = {
      actor_position: 2,
      choice_cause: choice.cause,
      choice_id: choice.id,
      command_id: commandId,
      effect_stop: 'stable',
      effects: [],
      event_version: 3,
      prng_counter: 0,
      selected_options: ['option:2'],
      sequence: 2,
      state_version: 3,
      turn: 1,
      type: 'choice_resolved',
    }
    const cardEvent = {
      ...darkArtsEvent,
      card_id: 'instance:starter:1',
      targets: [],
      type: 'card_played',
    }

    expect(isExecuteGameCommandRequest(command)).toBe(true)
    expect(isExecuteGameCommandRequest({ ...command, selected_options: [] })).toBe(true)
    expect(
      isExecuteGameCommandRequest({
        ...command,
        type: 'complete_dark_arts',
      }),
    ).toBe(false)
    expect(isExecuteGameCommandRequest({ ...command, choice_id: undefined })).toBe(false)
    expect(isRealtimeGameEvent(event)).toBe(true)
    expect(isRealtimeGameEvent({ ...event, event_version: 2 })).toBe(false)
    expect(isRealtimeGameEvent(darkArtsEvent)).toBe(true)
    expect(
      isGameProjectionResponse({
        ...projection(),
        choice,
        legal_actions: ['resolve_choice'],
      }),
    ).toBe(true)
    expect(
      isGameProjectionResponse({
        ...projection(),
        choice: { ...choice, cause: undefined },
        legal_actions: ['resolve_choice'],
      }),
    ).toBe(false)
    expect(
      isGameProjectionResponse({
        ...projection(),
        choice: { ...choice, min: 2, max: 1 },
        legal_actions: ['resolve_choice'],
      }),
    ).toBe(false)
    expect(
      isGameProjectionResponse({
        ...projection(),
        choice: { ...choice, max: 3 },
        legal_actions: ['resolve_choice'],
      }),
    ).toBe(false)
    expect(
      isGameProjectionResponse({
        ...projection(),
        choice: { ...choice, min: 0, max: 0 },
        legal_actions: ['resolve_choice'],
      }),
    ).toBe(false)
    expect(
      isGameProjectionResponse({
        ...projection(),
        choice: { ...choice, max: 2 },
        legal_actions: ['resolve_choice'],
      }),
    ).toBe(false)
    expect(
      isGameProjectionResponse({
        ...projection(),
        choice: { ...choice, kind: 'target' },
        legal_actions: ['resolve_choice'],
      }),
    ).toBe(true)
    expect(
      isGameProjectionResponse({
        ...projection(),
        choice: { ...choice, kind: 'target', min: 0, max: 0 },
        legal_actions: ['resolve_choice'],
      }),
    ).toBe(false)
    expect(
      isGameProjectionResponse({
        ...projection(),
        choice: { ...choice, kind: 'target', max: choice.options.length },
        legal_actions: ['resolve_choice'],
      }),
    ).toBe(false)
    expect(
      isGameProjectionResponse({
        ...projection(),
        choice: { ...choice, options: ['o'.repeat(257), 'option:2'] },
        legal_actions: ['resolve_choice'],
      }),
    ).toBe(false)
    expect(isRealtimeGameEvent({ ...event, effect_stop: 'choice' })).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...event,
        choice,
        effect_stop: 'stable',
      }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...event,
        choice,
        effect_stop: 'terminal',
      }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...event,
        choice: { ...choice, kind: 'target' },
        effect_stop: 'choice',
      }),
    ).toBe(true)
    expect(
      isRealtimeGameEvent({ ...darkArtsEvent, effect_stop: 'choice' }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...darkArtsEvent,
        choice,
        effect_stop: 'stable',
      }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...darkArtsEvent,
        choice,
        effect_stop: 'terminal',
      }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...darkArtsEvent,
        choice,
        effect_stop: 'choice',
      }),
    ).toBe(true)
    expect(
      isRealtimeGameEvent({
        ...darkArtsEvent,
        choice,
        effect_stop: 'choice',
        event_version: 2,
      }),
    ).toBe(true)
    expect(isRealtimeGameEvent(cardEvent)).toBe(true)
    expect(isRealtimeGameEvent({ ...cardEvent, effect_stop: 'choice' })).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...cardEvent,
        choice,
        effect_stop: 'choice',
      }),
    ).toBe(true)
    expect(isRealtimeGameEvent({ ...cardEvent, choice })).toBe(false)
  })

  it('validates every field in the public error envelope', () => {
    const error = {
      error: {
        category: 'validation',
        code: 'INVALID_INPUT',
        correlation_id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
        details: {},
        message_key: 'error.invalid_input',
        retry: 'not_retryable',
      },
    }

    expect(isErrorResponse(error)).toBe(true)
    expect(isErrorResponse({ error: { ...error.error, category: undefined } })).toBe(false)
    expect(isErrorResponse({ error: { ...error.error, correlation_id: 'not-a-uuid' } })).toBe(false)
    expect(isErrorResponse({ error: { ...error.error, unexpected: true } })).toBe(false)
  })

  it('enforces UUID and RFC 3339 date-time formats', () => {
    expect(isGameProjectionResponse(projection())).toBe(true)
    expect(
      isGameProjectionResponse({
        ...projection(),
        game: { ...projection().game, id: 'not-a-uuid' },
      }),
    ).toBe(false)
    expect(
      isGameProjectionResponse({
        ...projection(),
        game: { ...projection().game, expires_at: 'September someday' },
      }),
    ).toBe(false)
  })

  it('validates each command shape without accepting fields from another command', () => {
    const common = {
      command_id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
      expected_state_version: 2,
    }

    expect(isExecuteGameCommandRequest({ ...common, type: 'complete_dark_arts' })).toBe(true)
    expect(
      isExecuteGameCommandRequest({
        ...common,
        card_id: 'instance:00000001',
        targets: [{ selector_id: 'target:hero', target_ids: ['hero:1'] }],
        type: 'play_card',
      }),
    ).toBe(true)
    expect(
      isExecuteGameCommandRequest({
        ...common,
        amount: 1,
        type: 'assign_attack',
        villain_id: 'instance:00000002',
      }),
    ).toBe(true)
    expect(
      isExecuteGameCommandRequest({
        ...common,
        card_id: 'instance:00000003',
        type: 'acquire_card',
      }),
    ).toBe(true)
    expect(isExecuteGameCommandRequest({ ...common, type: 'play_card' })).toBe(false)
    expect(
      isExecuteGameCommandRequest({
        ...common,
        card_id: 'instance:00000003',
        type: 'complete_dark_arts',
      }),
    ).toBe(false)
  })

  it('accepts the command-specific realtime event shapes', () => {
    expect(
      isRealtimeGameEvent({
        actor_position: 1,
        amount: 1,
        effects: [],
        event_version: 3,
        sequence: 2,
        state_version: 3,
        turn: 1,
        type: 'attack_assigned',
        villain_id: 'instance:00000002',
      }),
    ).toBe(true)
  })

  it('validates ephemeral presence without accepting official state fields', () => {
    const presence = {
      blocked: true,
      game_id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
      participants: [
        { position: 1, status: 'reconnecting' },
        { position: 2, status: 'online' },
      ],
      protocol_version: 2,
      required_participant_position: 1,
      type: 'presence',
    }

    expect(isRealtimePresenceMessage(presence)).toBe(true)
    expect(isRealtimePresenceMessage({ ...presence, state_version: 1 })).toBe(false)
    expect(
      isRealtimePresenceMessage({
        ...presence,
        participants: [{ position: 1, status: 'unknown' }],
      }),
    ).toBe(false)
  })

  it('accepts only the safe device replacement metadata and a complete recovery result', () => {
    const replacement = {
      sessions: [
        {
          created_at: '2026-09-01T14:20:00Z',
          id: '2fe6c1be-50fc-42ac-8c4f-6ef270099c24',
          label: 'Sessão 1',
        },
        {
          created_at: '2026-09-03T10:05:00Z',
          id: '8aa543d4-9d6f-4a8c-bd7b-6c6605be48fc',
          label: 'Sessão 2',
        },
      ],
      status: 'replacement_required',
    }

    expect(isRecoveryReplacementRequiredResponse(replacement)).toBe(true)
    expect(
      isRecoveryReplacementRequiredResponse({
        ...replacement,
        sessions: [{ ...replacement.sessions[0], user_agent: 'browser fingerprint' }],
      }),
    ).toBe(false)
    expect(
      isRecoveredLobbyResponse({
        kind: 'lobby',
        lobby: {
          content_options: [],
          heroes: [],
          participant: {
            display_name: 'Minerva',
            position: 1,
            ready: false,
            role: 'host',
          },
          participants: [],
          room: { code: '9HKGW4RT', status: 'open' },
        },
        recovery_token: 'a'.repeat(64),
      }),
    ).toBe(true)
    expect(
      isRecoveredLobbyResponse({ kind: 'lobby', recovery_token: 'a'.repeat(64) }),
    ).toBe(false)
  })
})
