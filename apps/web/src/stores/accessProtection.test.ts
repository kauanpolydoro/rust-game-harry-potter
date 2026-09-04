import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useAccessProtectionStore } from './accessProtection'

const firstSession = {
  created_at: '2026-09-04T18:00:00Z',
  current: true,
  id: 'a8665180-17cb-4564-9174-14ef56f17851',
  label: 'Sessão 1',
} as const

const secondSession = {
  created_at: '2026-09-04T18:01:00Z',
  current: false,
  id: '736a975d-c348-491a-8c99-bf3459841d66',
  label: 'Sessão 2',
} as const

function revocationResponse() {
  return {
    revoked_session: { id: secondSession.id, label: secondSession.label },
    security_event: {
      actor_position: 1,
      cursor: 1,
      event_version: 1,
      occurred_at: '2026-09-04T18:02:00Z',
      session_label: secondSession.label,
      target_position: 1,
      type: 'session_revoked',
    },
    status: 'revoked',
  }
}

describe('access protection', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
    sessionStorage.clear()
    vi.unstubAllGlobals()
  })

  it('lists safe labels and retries one revocation with the same idempotency key', async () => {
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ sessions: [firstSession, secondSession] }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockRejectedValueOnce(new TypeError('response lost'))
      .mockResolvedValueOnce(
        new Response(JSON.stringify(revocationResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)
    const access = useAccessProtectionStore()

    await access.loadSessions()
    expect(access.sessions).toEqual([firstSession, secondSession])
    expect(await access.revokeSession(secondSession.id)).toBeNull()
    const firstHeaders = (request.mock.calls[1]![1] as RequestInit).headers
    expect(access.errorCode).toBe('NETWORK_UNAVAILABLE')

    expect(await access.revokeSession(secondSession.id)).toBe('session_retained')
    expect((request.mock.calls[2]![1] as RequestInit).headers).toEqual(firstHeaders)
    expect(access.sessions).toEqual([firstSession])
    expect(access.confirmation).toBe('session_revoked')
    expect(localStorage.length).toBe(0)
    expect(sessionStorage.length).toBe(0)
  })

  it('reports that protecting a participant invalidates the current session', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            participant: { display_name: 'Minerva', position: 1 },
            recovery_generation: 2,
            revoked_sessions: 2,
            security_event: {
              actor_position: 1,
              cursor: 2,
              event_version: 1,
              occurred_at: '2026-09-04T18:03:00Z',
              recovery_generation: 2,
              revoked_sessions: 2,
              target_position: 1,
              type: 'participant_protected',
            },
            status: 'protected',
          }),
          { headers: { 'Content-Type': 'application/json' }, status: 200 },
        ),
      ),
    )
    const access = useAccessProtectionStore()

    expect(await access.protectParticipant()).toBe('session_revoked')
    expect(access.confirmation).toBe('participant_protected')
  })

  it('preserves only the confirming host when room protection explicitly requests it', async () => {
    const request = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          current_session_preserved: true,
          password_generation: 2,
          recovery_epoch: 2,
          revoked_sessions: 3,
          security_event: {
            actor_position: 1,
            current_session_preserved: true,
            cursor: 3,
            event_version: 1,
            occurred_at: '2026-09-04T18:04:00Z',
            password_generation: 2,
            recovery_epoch: 2,
            revoked_sessions: 3,
            type: 'room_protected',
          },
          status: 'protected',
        }),
        { headers: { 'Content-Type': 'application/json' }, status: 200 },
      ),
    )
    vi.stubGlobal('fetch', request)
    const access = useAccessProtectionStore()

    expect(
      await access.protectRoom({
        current_recovery_password: 'a long uncommon passphrase',
        new_recovery_password: 'a newer uncommon recovery phrase',
        preserve_current_session: true,
        protection_confirmed: true,
      }),
    ).toBe('session_retained')
    expect(request).toHaveBeenCalledWith(
      '/api/rooms/current/protection',
      expect.objectContaining({
        body: JSON.stringify({
          current_recovery_password: 'a long uncommon passphrase',
          new_recovery_password: 'a newer uncommon recovery phrase',
          preserve_current_session: true,
          protection_confirmed: true,
        }),
        method: 'PUT',
      }),
    )
    expect(access.confirmation).toBe('room_protected')
  })

  it('fails closed when protection response fields disagree with its security event', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            current_session_preserved: false,
            password_generation: 2,
            recovery_epoch: 2,
            revoked_sessions: 1,
            security_event: {
              actor_position: 1,
              current_session_preserved: true,
              cursor: 3,
              event_version: 1,
              occurred_at: '2026-09-04T18:04:00Z',
              password_generation: 2,
              recovery_epoch: 2,
              revoked_sessions: 1,
              type: 'room_protected',
            },
            status: 'protected',
          }),
          { headers: { 'Content-Type': 'application/json' }, status: 200 },
        ),
      ),
    )
    const access = useAccessProtectionStore()

    expect(
      await access.protectRoom({
        current_recovery_password: 'a long uncommon passphrase',
        new_recovery_password: 'a newer uncommon recovery phrase',
        preserve_current_session: false,
        protection_confirmed: true,
      }),
    ).toBeNull()
    expect(access.errorCode).toBe('UNEXPECTED_RESPONSE')
    expect(access.confirmation).toBeNull()
  })
})
