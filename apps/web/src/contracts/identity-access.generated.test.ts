import { describe, expect, it } from 'vitest'

import {
  isErrorResponse,
  isGameProjectionResponse,
} from './identity-access.generated'

function projection() {
  return {
    choice: { status: 'none' },
    game: {
      adventure: { id: 'adventure:001', name: 'Game 1' },
      expires_at: '2026-09-10T12:00:00Z',
      id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
      status: 'in_progress',
    },
    legal_actions: ['complete_dark_arts'],
    participant: {
      display_name: 'Minerva',
      hero: { id: 'harry', name: 'Harry' },
      position: 1,
      role: 'host',
    },
    participants: [
      {
        display_name: 'Minerva',
        hero: { id: 'harry', name: 'Harry' },
        position: 1,
        role: 'host',
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
    turn: { active_position: 1, number: 1, phase: 'dark_arts' },
  }
}

describe('generated identity contract guards', () => {
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
})
