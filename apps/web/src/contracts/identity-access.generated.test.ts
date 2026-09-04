import { describe, expect, it } from 'vitest'

import {
  isAssistedRecoveryCredentialResponse,
  isDecisionPointSummary,
  isDirectRecoveryCredentialResponse,
  isEffectOutcomeSummary,
  isEffectPathSegmentSummary,
  isEndTurnOutcomeSummary,
  isErrorResponse,
  isExecuteGameCommandRequest,
  isGameProjectionResponse,
  isRegenerateAssistedRecoveryCredentialRequest,
  isRealtimeGameEvent,
  isRealtimePresenceMessage,
  isRotateRecoveryPasswordResponse,
  isSecurityEventsMessage,
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
    legal_actions: ['end_hero_actions'],
    legal_intentions: {
      acquire_cards: [],
      assign_attack: [],
      end_hero_actions: true,
      play_cards: [],
    },
    queued_effect_count: 0,
    queued_phases: ['end_turn'],
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
    turn: { active_position: 1, number: 1, phase: 'hero_actions' },
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
      event_version: 4,
      targets: [],
      type: 'card_played',
    }

    expect(isExecuteGameCommandRequest(command)).toBe(true)
    expect(isExecuteGameCommandRequest({ ...command, selected_options: [] })).toBe(true)
    expect(
      isExecuteGameCommandRequest({
        command_id: commandId,
        expected_state_version: 2,
        type: 'end_hero_actions',
      }),
    ).toBe(true)
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

    expect(isExecuteGameCommandRequest({ ...common, type: 'end_hero_actions' })).toBe(true)
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
        type: 'end_hero_actions',
      }),
    ).toBe(false)
  })

  it('accepts the command-specific realtime event shapes', () => {
    expect(
      isRealtimeGameEvent({
        actor_position: 1,
        amount: 1,
        effects: [],
        event_version: 4,
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

  it('keeps recovery management responses and security notices strictly secretless', () => {
    const regeneratedEvent = {
      actor_position: 1,
      cursor: 2,
      delivery: 'host_assisted',
      event_version: 1,
      occurred_at: '2026-09-04T18:00:00Z',
      recovery_generation: 2,
      target_position: 2,
      type: 'recovery_credential_regenerated',
    }
    const assisted = {
      delivery: 'host_assisted',
      participant: { display_name: 'Luna', position: 2 },
      recovery_generation: 2,
      recovery_token: 'a'.repeat(64),
      risk_message_key: 'participant.recovery.host_assisted_impersonation_risk',
      security_event: regeneratedEvent,
    }
    const direct = {
      delivery: 'direct',
      participant: { display_name: 'Minerva', position: 1 },
      recovery_generation: 2,
      recovery_token: 'b'.repeat(64),
      security_event: { ...regeneratedEvent, actor_position: 1, delivery: 'direct', target_position: 1 },
    }
    const passwordEvent = {
      actor_position: 1,
      cursor: 1,
      event_version: 1,
      occurred_at: '2026-09-04T17:59:00Z',
      password_generation: 2,
      type: 'recovery_password_rotated',
    }

    expect(isAssistedRecoveryCredentialResponse(assisted)).toBe(true)
    expect(isDirectRecoveryCredentialResponse(direct)).toBe(true)
    expect(
      isRotateRecoveryPasswordResponse({
        password_generation: 2,
        security_event: passwordEvent,
      }),
    ).toBe(true)
    expect(
      isSecurityEventsMessage({
        cursor: 2,
        events: [passwordEvent, regeneratedEvent],
        from_cursor: 0,
        protocol_version: 1,
        type: 'security_events',
      }),
    ).toBe(true)
    expect(
      isSecurityEventsMessage({
        cursor: 2,
        events: [{ ...passwordEvent, recovery_token: 'secret' }],
        from_cursor: 0,
        protocol_version: 1,
        type: 'security_events',
      }),
    ).toBe(false)
    expect(
      isRegenerateAssistedRecoveryCredentialRequest({
        host_assistance_risk_acknowledged: true,
      }),
    ).toBe(true)
    expect(
      isRegenerateAssistedRecoveryCredentialRequest({
        host_assistance_risk_acknowledged: false,
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

  it('validates each effect path segment as a closed discriminated variant', () => {
    for (const segment of [
      { index: 0, type: 'choice' },
      { type: 'otherwise' },
      { type: 'repeat_body' },
      { result: 8, type: 'roll_outcome' },
      { index: 1023, type: 'sequence' },
      { type: 'then' },
    ]) {
      expect(isEffectPathSegmentSummary(segment)).toBe(true)
    }

    expect(isEffectPathSegmentSummary({ type: 'choice' })).toBe(false)
    expect(isEffectPathSegmentSummary({ index: 0, type: 'otherwise' })).toBe(false)
    expect(isEffectPathSegmentSummary({ result: 6, type: 'sequence' })).toBe(false)
  })

  it('validates each effect outcome as a closed discriminated variant', () => {
    for (const outcome of [
      { die: 'd8', result: 8, rule_id: 'rule:roll', type: 'die_rolled' },
      {
        from: 'hero_hand',
        rule_id: 'rule:discard',
        target_id: 'card:alohomora',
        target_position: 1,
        to: 'hero_discard_pile',
        type: 'moved',
      },
      { reason: 'explicit', rule_id: 'rule:noop', type: 'no_op' },
      {
        after: 8,
        before: 10,
        cause: 'effect',
        resource: 'health',
        rule_id: 'rule:damage',
        target_id: 'hero:harry',
        target_position: 1,
        type: 'resource_changed',
      },
      { outcome: 'won', rule_id: 'rule:victory', type: 'terminal' },
    ]) {
      expect(isEffectOutcomeSummary(outcome)).toBe(true)
    }

    expect(
      isEffectOutcomeSummary({ die: 'd6', rule_id: 'rule:roll', type: 'die_rolled' }),
    ).toBe(false)
    expect(
      isEffectOutcomeSummary({
        die: 'd6',
        from: 'hero_hand',
        rule_id: 'rule:discard',
        target_id: 'card:alohomora',
        to: 'hero_discard_pile',
        type: 'moved',
      }),
    ).toBe(false)
    expect(isEffectOutcomeSummary({ rule_id: 'rule:noop', type: 'no_op' })).toBe(false)
    expect(
      isEffectOutcomeSummary({
        after: 8,
        before: 10,
        resource: 'health',
        rule_id: 'rule:damage',
        target_id: 'hero:harry',
        type: 'resource_changed',
      }),
    ).toBe(false)
    expect(
      isEffectOutcomeSummary({
        outcome: 'won',
        reason: 'explicit',
        rule_id: 'rule:victory',
        type: 'terminal',
      }),
    ).toBe(false)
  })

  it('validates each decision point as a closed discriminated variant', () => {
    const choice = {
      cause: 'rule:dark-arts',
      id: 'choice:dark-arts',
      kind: 'target',
      max: 1,
      min: 1,
      options: ['hero:harry', 'hero:ron'],
      responsible_position: 1,
      status: 'pending',
    }

    expect(isDecisionPointSummary({ type: 'none' })).toBe(true)
    expect(isDecisionPointSummary({ type: 'automatic' })).toBe(true)
    expect(
      isDecisionPointSummary({ responsible_position: 1, type: 'player_intent' }),
    ).toBe(true)
    expect(isDecisionPointSummary({ choice, type: 'effect_choice' })).toBe(true)

    expect(isDecisionPointSummary({ responsible_position: 1, type: 'none' })).toBe(false)
    expect(isDecisionPointSummary({ type: 'player_intent' })).toBe(false)
    expect(isDecisionPointSummary({ choice, type: 'automatic' })).toBe(false)
    expect(
      isDecisionPointSummary({
        choice: { ...choice, path: [{ index: 0, type: 'sequence' }] },
        type: 'effect_choice',
      }),
    ).toBe(false)
    expect(
      isDecisionPointSummary({
        choice: { ...choice, options: ['hero:harry', 'hero:harry'] },
        type: 'effect_choice',
      }),
    ).toBe(false)
  })

  it('validates each end-turn outcome as a closed discriminated variant', () => {
    expect(
      isEndTurnOutcomeSummary({
        card_id: 'card:alohomora',
        from: 'hero_hand',
        to: 'hero_discard_pile',
        type: 'card_moved',
      }),
    ).toBe(true)
    expect(
      isEndTurnOutcomeSummary({
        card_id: 'card:accio',
        from: 'hero_draw_pile',
        to: 'hero_hand',
        type: 'card_moved',
      }),
    ).toBe(true)
    expect(
      isEndTurnOutcomeSummary({
        bottom_to_top: ['card:one', 'card:two'],
        owner_position: 1,
        type: 'pile_shuffled',
        zone: 'hero_draw_pile',
      }),
    ).toBe(true)
    expect(
      isEndTurnOutcomeSummary({ before: 2, resource: 'attack', type: 'resource_reset' }),
    ).toBe(true)

    expect(
      isEndTurnOutcomeSummary({
        card_id: 'card:alohomora',
        from: 'hero_hand',
        owner_position: 1,
        to: 'hero_discard_pile',
        type: 'card_moved',
      }),
    ).toBe(false)
    expect(
      isEndTurnOutcomeSummary({
        card_id: 'card:alohomora',
        from: 'hero_hand',
        to: 'hero_hand',
        type: 'card_moved',
      }),
    ).toBe(false)
    expect(
      isEndTurnOutcomeSummary({
        card_id: 'card:accio',
        from: 'hero_draw_pile',
        to: 'hero_discard_pile',
        type: 'card_moved',
      }),
    ).toBe(false)
    expect(
      isEndTurnOutcomeSummary({
        bottom_to_top: ['card:one', 'card:one'],
        owner_position: 1,
        type: 'pile_shuffled',
        zone: 'hero_draw_pile',
      }),
    ).toBe(false)
    expect(isEndTurnOutcomeSummary({ resource: 'attack', type: 'resource_reset' })).toBe(false)
  })

  it('keeps realtime event versions valid and mutually exclusive', () => {
    const legacy = {
      actor_position: 1,
      effect_stop: 'stable',
      effects: [],
      prng_counter: 0,
      sequence: 1,
      state_version: 2,
      turn: 1,
      type: 'dark_arts_completed',
    }
    const version1 = { ...legacy, event_version: 1 }
    const version2Stable = { ...legacy, event_version: 2 }
    const version2Choice = {
      ...legacy,
      choice: {
        id: 'choice:legacy:target:0',
        kind: 'target',
        max: 1,
        min: 1,
        options: ['hero:harry', 'hero:ron'],
        responsible_position: 1,
      },
      effect_stop: 'choice',
      event_version: 2,
    }
    const version2Terminal = {
      ...legacy,
      effect_stop: 'terminal',
      effects: [{ outcome: 'won', rule_id: 'rule:terminal', type: 'terminal' }],
      event_version: 2,
    }
    const impossibleVersion3Turn = {
      actor_position: 1,
      control: {
        active_position: 2,
        decision_point: { responsible_position: 2, type: 'player_intent' },
        phase: 'hero_actions',
        queued_effects: [],
        queued_phases: ['end_turn'],
        status: 'in_progress',
        turn: 2,
      },
      end_turn: [
        { before: 0, resource: 'attack', type: 'resource_reset' },
        { before: 0, resource: 'influence', type: 'resource_reset' },
      ],
      event_version: 3,
      prng_counter: 0,
      sequence: 1,
      state_version: 2,
      steps: [
        { effects: [], phase: 'end_turn' },
        { effects: [], phase: 'dark_arts' },
        { effects: [], phase: 'villains' },
      ],
      turn: 1,
      type: 'turn_completed',
    }
    const publicControl = {
      active_position: 2,
      decision_point: { responsible_position: 2, type: 'player_intent' },
      phase: 'hero_actions',
      queued_effect_count: 0,
      queued_phases: ['end_turn'],
      status: 'in_progress',
      turn: 2,
    }
    const version4Turn = {
      ...impossibleVersion3Turn,
      control: publicControl,
      event_version: 4,
    }
    const nextChoice = {
      cause: 'rule:next',
      id: 'choice:next:target:0',
      kind: 'target',
      max: 1,
      min: 1,
      options: ['hero:harry', 'hero:ron'],
      responsible_position: 2,
      status: 'pending',
    }
    const version4Choice = {
      actor_position: 1,
      choice_cause: 'rule:dark-arts',
      choice_id: 'choice:dark-arts:target:0',
      control: {
        active_position: 1,
        decision_point: { choice: nextChoice, type: 'effect_choice' },
        phase: 'dark_arts',
        queued_effect_count: 1,
        queued_phases: ['villains', 'hero_actions', 'end_turn'],
        status: 'in_progress',
        turn: 1,
      },
      event_version: 4,
      prng_counter: 0,
      selected_options: ['hero:harry'],
      sequence: 2,
      state_version: 3,
      steps: [{ effects: [], phase: 'dark_arts' }],
      turn: 1,
      type: 'choice_resolved',
    }

    expect(isRealtimeGameEvent(version1)).toBe(true)
    expect(isRealtimeGameEvent(version2Stable)).toBe(true)
    expect(isRealtimeGameEvent(version2Choice)).toBe(true)
    expect(isRealtimeGameEvent(version2Terminal)).toBe(true)
    expect(isRealtimeGameEvent(impossibleVersion3Turn)).toBe(false)
    expect(isRealtimeGameEvent(version4Turn)).toBe(true)
    expect(isRealtimeGameEvent(version4Choice)).toBe(true)
    expect(
      isRealtimeGameEvent({
        ...version4Choice,
        steps: [
          ...version4Choice.steps,
          { effects: [], phase: 'villains' },
        ],
      }),
    ).toBe(true)
    expect(isRealtimeGameEvent({ ...version4Choice, steps: [] })).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...version4Choice,
        steps: [
          ...version4Choice.steps,
          { effects: [], phase: 'villains' },
          { effects: [], phase: 'dark_arts' },
        ],
      }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...version4Choice,
        steps: [{ effects: [], phase: 'end_turn' }],
      }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...version4Choice,
        control: { ...version4Choice.control, queued_effects: [] },
      }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({ ...version4Turn, steps: version4Turn.steps.slice(0, 2) }),
    ).toBe(true)
    expect(
      isRealtimeGameEvent({ ...version4Turn, steps: version4Turn.steps.slice(0, 1) }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...version4Turn,
        steps: [...version4Turn.steps, { effects: [], phase: 'hero_actions' }],
      }),
    ).toBe(false)

    expect(
      isRealtimeGameEvent({
        ...version2Choice,
        choice: {
          ...version2Choice.choice,
          options: ['hero:harry', 'hero:harry'],
        },
      }),
    ).toBe(false)

    expect(isRealtimeGameEvent({ ...version1, choice: version2Choice.choice })).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...version1,
        effects: [{ reason: 'explicit', rule_id: 'rule:legacy', type: 'no_op' }],
      }),
    ).toBe(false)
    expect(isRealtimeGameEvent({ ...version1, effect_stop: 'choice' })).toBe(false)
    expect(isRealtimeGameEvent({ ...version1, prng_counter: 1 })).toBe(false)
    expect(
      isRealtimeGameEvent({ ...version2Stable, choice: version2Choice.choice }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({ ...legacy, effect_stop: 'choice', event_version: 2 }),
    ).toBe(false)
    expect(
      isRealtimeGameEvent({ ...version2Terminal, choice: version2Choice.choice }),
    ).toBe(false)
    expect(isRealtimeGameEvent({ ...version2Choice, end_turn: [] })).toBe(false)
    expect(
      isRealtimeGameEvent({ ...version4Turn, effect_stop: 'stable', effects: [] }),
    ).toBe(false)
    expect(isRealtimeGameEvent({ ...version4Turn, event_version: 2 })).toBe(false)
    expect(isRealtimeGameEvent({ ...version2Choice, event_version: 3 })).toBe(false)
    expect(
      isRealtimeGameEvent({
        ...version4Turn,
        control: {
          ...version4Turn.control,
          queued_phases: ['end_turn', 'end_turn'],
        },
      }),
    ).toBe(false)
  })

  it('honors uniqueItems for every generated array guard', () => {
    expect(
      isGameProjectionResponse({
        ...projection(),
        legal_actions: ['end_hero_actions', 'end_hero_actions'],
      }),
    ).toBe(false)
    expect(
      isGameProjectionResponse({
        ...projection(),
        queued_phases: ['end_turn', 'end_turn'],
      }),
    ).toBe(false)
    expect(
      isGameProjectionResponse({
        ...projection(),
        queued_effect_count: 4097,
      }),
    ).toBe(false)
  })
})
