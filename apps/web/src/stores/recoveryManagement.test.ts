import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useRecoveryManagementStore } from './recoveryManagement'
import { useRoomAccessStore } from './roomAccess'
import { useSecuritySyncStore } from './securitySync'

function regeneratedResponse(delivery: 'direct' | 'host_assisted') {
  return {
    delivery,
    participant: {
      display_name: delivery === 'direct' ? 'Minerva' : 'Luna',
      position: delivery === 'direct' ? 1 : 2,
    },
    recovery_generation: 2,
    recovery_token: 'a'.repeat(64),
    ...(delivery === 'host_assisted'
      ? {
          risk_message_key: 'participant.recovery.host_assisted_impersonation_risk',
        }
      : {}),
    security_event: {
      actor_position: 1,
      cursor: 1,
      delivery,
      event_version: 1,
      occurred_at: '2026-09-04T18:00:00Z',
      recovery_generation: 2,
      target_position: delivery === 'direct' ? 1 : 2,
      type: 'recovery_credential_regenerated',
    },
  }
}

describe('recovery credential management', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
    sessionStorage.clear()
  })

  afterEach(() => {
    useSecuritySyncStore().disconnect()
    vi.unstubAllGlobals()
  })

  it.each(['direct', 'host_assisted'] as const)(
    'discards a late %s credential after session invalidation',
    async (delivery) => {
      const response = Promise.withResolvers<Response>()
      vi.stubGlobal('fetch', vi.fn().mockReturnValue(response.promise))
      const recovery = useRecoveryManagementStore()
      const operation = delivery === 'direct'
        ? recovery.regenerateOwnCredential()
        : recovery.regenerateAssistedCredential(2)

      useRoomAccessStore().clearAuthenticatedSession()
      recovery.$reset()
      response.resolve(new Response(JSON.stringify(regeneratedResponse(delivery)), {
        headers: { 'Content-Type': 'application/json' },
        status: 200,
      }))

      expect(await operation).toBe(false)
      expect(recovery.issuedCredential).toBeNull()
      expect(recovery.confirmation).toBeNull()
      expect(recovery.pendingOperation).toBeNull()
      expect(recovery.status).toBe('idle')
      expect(localStorage.length).toBe(0)
      expect(sessionStorage.length).toBe(0)
    },
  )

  it('retries direct delivery with one idempotency key and keeps the token only in memory', async () => {
    const request = vi
      .fn()
      .mockRejectedValueOnce(new TypeError('response lost'))
      .mockResolvedValueOnce(
        new Response(JSON.stringify(regeneratedResponse('direct')), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)
    const recovery = useRecoveryManagementStore()

    expect(await recovery.regenerateOwnCredential()).toBe(false)
    const firstHeaders = (request.mock.calls[0]![1] as RequestInit).headers
    expect(recovery.errorCode).toBe('NETWORK_UNAVAILABLE')

    expect(await recovery.regenerateOwnCredential()).toBe(true)
    const retryHeaders = (request.mock.calls[1]![1] as RequestInit).headers
    expect(retryHeaders).toEqual(firstHeaders)
    expect(recovery.issuedCredential?.recovery_token).toBe('a'.repeat(64))
    expect(localStorage.length).toBe(0)
    expect(sessionStorage.length).toBe(0)
  })

  it('uses inline password confirmation and explicit assisted-delivery acknowledgement', async () => {
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            password_generation: 2,
            security_event: {
              actor_position: 1,
              cursor: 1,
              event_version: 1,
              occurred_at: '2026-09-04T18:00:00Z',
              password_generation: 2,
              type: 'recovery_password_rotated',
            },
          }),
          { headers: { 'Content-Type': 'application/json' }, status: 200 },
        ),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(regeneratedResponse('host_assisted')), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)
    const recovery = useRecoveryManagementStore()

    expect(
      await recovery.rotatePassword({
        current_recovery_password: 'a long uncommon passphrase',
        new_recovery_password: 'a newer uncommon recovery phrase',
      }),
    ).toBe(true)
    expect(request).toHaveBeenNthCalledWith(
      1,
      '/api/session/recovery-password',
      expect.objectContaining({
        body: JSON.stringify({
          current_recovery_password: 'a long uncommon passphrase',
          new_recovery_password: 'a newer uncommon recovery phrase',
        }),
        method: 'PUT',
      }),
    )

    expect(await recovery.regenerateAssistedCredential(2)).toBe(true)
    expect(request).toHaveBeenNthCalledWith(
      2,
      '/api/rooms/current/participants/2/recovery-credential',
      expect.objectContaining({
        body: JSON.stringify({ host_assistance_risk_acknowledged: true }),
        method: 'POST',
      }),
    )
    expect(recovery.issuedCredential?.delivery).toBe('host_assisted')
  })

  it('rejects a structurally valid response whose delivery metadata disagrees', async () => {
    const incoherent = regeneratedResponse('direct')
    incoherent.security_event.delivery = 'host_assisted'
    incoherent.security_event.target_position = 2
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify(incoherent), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      ),
    )
    const recovery = useRecoveryManagementStore()

    expect(await recovery.regenerateOwnCredential()).toBe(false)
    expect(recovery.errorCode).toBe('UNEXPECTED_RESPONSE')
    expect(recovery.issuedCredential).toBeNull()
  })

  it('does not expose a response superseded by a newer security event', async () => {
    const security = useSecuritySyncStore()
    security.receive(
      JSON.stringify({
        cursor: 3,
        events: [
          {
            actor_position: 1,
            cursor: 3,
            delivery: 'direct',
            event_version: 1,
            occurred_at: '2026-09-04T18:02:00Z',
            recovery_generation: 3,
            target_position: 1,
            type: 'recovery_credential_regenerated',
          },
        ],
        protocol_version: 1,
        type: 'security_snapshot',
      }),
    )
    const stale = regeneratedResponse('direct')
    stale.security_event.cursor = 2
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify(stale), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      ),
    )
    const recovery = useRecoveryManagementStore()

    expect(await recovery.regenerateOwnCredential()).toBe(false)
    expect(recovery.errorCode).toBe('RECOVERY_CREDENTIAL_SUPERSEDED')
    expect(recovery.issuedCredential).toBeNull()
  })
})
