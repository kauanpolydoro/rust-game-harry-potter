import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useSecuritySyncStore } from './securitySync'

class FakeWebSocket {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  static instances: FakeWebSocket[] = []

  readonly url: string
  readonly requestedProtocol: string
  protocol = ''
  readyState = FakeWebSocket.CONNECTING
  onopen: (() => void) | null = null
  onmessage: ((event: { data: string }) => void) | null = null
  onerror: (() => void) | null = null
  onclose: ((event: { code: number }) => void) | null = null

  constructor(url: string | URL, protocol: string | string[]) {
    this.url = String(url)
    this.requestedProtocol = Array.isArray(protocol) ? (protocol[0] ?? '') : protocol
    FakeWebSocket.instances.push(this)
  }

  open(): void {
    this.readyState = FakeWebSocket.OPEN
    this.protocol = this.requestedProtocol
    this.onopen?.()
  }

  receive(message: unknown): void {
    this.onmessage?.({ data: JSON.stringify(message) })
  }

  close(): void {
    this.readyState = FakeWebSocket.CLOSED
    this.onclose?.({ code: 1000 })
  }

  serverClose(code: number): void {
    this.readyState = FakeWebSocket.CLOSED
    this.onclose?.({ code })
  }
}

const passwordEvent = {
  actor_position: 1,
  cursor: 1,
  event_version: 1,
  occurred_at: '2026-09-04T18:00:00Z',
  password_generation: 2,
  type: 'recovery_password_rotated',
}

describe('security event synchronization', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    FakeWebSocket.instances = []
    vi.stubGlobal('WebSocket', FakeWebSocket)
    setActivePinia(createPinia())
    localStorage.clear()
    sessionStorage.clear()
  })

  afterEach(() => {
    useSecuritySyncStore().disconnect()
    vi.useRealTimers()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('receives secretless notices with a memory-only cursor in lobby and game sessions', () => {
    const security = useSecuritySyncStore()
    security.connect()

    const socket = FakeWebSocket.instances[0]
    expect(socket?.requestedProtocol).toBe('hogwarts.session.v1')
    expect(socket?.url).toContain('/api/session/events?cursor=0')
    socket?.open()
    socket?.receive({
      cursor: 1,
      events: [passwordEvent],
      protocol_version: 1,
      type: 'security_snapshot',
    })

    expect(security.status).toBe('connected')
    expect(security.cursor).toBe(1)
    expect(security.notices).toEqual([passwordEvent])

    const assistedEvent = {
      actor_position: 1,
      cursor: 2,
      delivery: 'host_assisted',
      event_version: 1,
      occurred_at: '2026-09-04T18:01:00Z',
      recovery_generation: 2,
      target_position: 2,
      type: 'recovery_credential_regenerated',
    }
    socket?.receive({
      cursor: 2,
      events: [assistedEvent],
      from_cursor: 1,
      protocol_version: 1,
      type: 'security_events',
    })

    expect(security.cursor).toBe(2)
    expect(security.latestNotice).toEqual(assistedEvent)
    expect(localStorage.length).toBe(0)
    expect(sessionStorage.length).toBe(0)
  })

  it('marks a revoked session as terminal and accepts room protection as link supersession', () => {
    const security = useSecuritySyncStore()
    security.connect()
    const socket = FakeWebSocket.instances[0]
    socket?.open()
    const roomProtection = {
      actor_position: 1,
      current_session_preserved: true,
      cursor: 1,
      event_version: 1,
      occurred_at: '2026-09-04T18:00:00Z',
      password_generation: 2,
      recovery_epoch: 2,
      revoked_sessions: 3,
      type: 'room_protected',
    }
    socket?.receive({
      cursor: 1,
      events: [roomProtection],
      protocol_version: 1,
      type: 'security_snapshot',
    })

    expect(security.latestNotice).toEqual(roomProtection)
    expect(security.credentialWasSuperseded(1, 0)).toBe(true)
    socket?.serverClose(1008)
    expect(security.status).toBe('failed')
    expect(security.sessionInvalidated).toBe(true)
  })

  it('rejects a malformed or regressive event batch instead of displaying it', () => {
    const security = useSecuritySyncStore()
    security.connect()
    const socket = FakeWebSocket.instances[0]
    socket?.open()
    socket?.receive({
      cursor: 1,
      events: [passwordEvent],
      protocol_version: 1,
      type: 'security_snapshot',
    })

    socket?.receive({
      cursor: 2,
      events: [{ ...passwordEvent, cursor: 3 }],
      from_cursor: 1,
      protocol_version: 1,
      type: 'security_events',
    })

    expect(security.cursor).toBe(1)
    expect(security.notices).toEqual([passwordEvent])
    expect(security.status).toBe('reconnecting')
  })

  it('rejects incoherent direct-delivery actors', () => {
    const security = useSecuritySyncStore()
    security.connect()
    const socket = FakeWebSocket.instances[0]
    socket?.open()
    socket?.receive({
      cursor: 1,
      events: [
        {
          actor_position: 1,
          cursor: 1,
          delivery: 'direct',
          event_version: 1,
          occurred_at: '2026-09-04T18:00:00Z',
          recovery_generation: 2,
          target_position: 2,
          type: 'recovery_credential_regenerated',
        },
      ],
      protocol_version: 1,
      type: 'security_snapshot',
    })

    expect(security.cursor).toBe(0)
    expect(security.notices).toEqual([])
    expect(security.status).toBe('reconnecting')
  })

  it('reconnects with backoff and the memory cursor after an invalid batch', async () => {
    vi.spyOn(Math, 'random').mockReturnValue(0.5)
    const security = useSecuritySyncStore()
    security.connect()
    const socket = FakeWebSocket.instances[0]
    socket?.open()
    socket?.receive({
      cursor: 1,
      events: [passwordEvent],
      protocol_version: 1,
      type: 'security_snapshot',
    })
    socket?.receive({
      cursor: 2,
      events: [{ ...passwordEvent, cursor: 3 }],
      from_cursor: 1,
      protocol_version: 1,
      type: 'security_events',
    })

    expect(FakeWebSocket.instances).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(499)
    expect(FakeWebSocket.instances).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(FakeWebSocket.instances[1]?.url).toContain('/api/session/events?cursor=1')
  })

  it('replaces a future local cursor with the corrective server snapshot', async () => {
    vi.spyOn(Math, 'random').mockReturnValue(0.5)
    const security = useSecuritySyncStore()
    security.connect()
    const firstSocket = FakeWebSocket.instances[0]
    firstSocket?.open()
    const futureCredentialEvent = {
      actor_position: 2,
      cursor: 9,
      delivery: 'direct',
      event_version: 1,
      occurred_at: '2026-09-04T18:01:00Z',
      recovery_generation: 9,
      target_position: 2,
      type: 'recovery_credential_regenerated',
    }
    firstSocket?.receive({
      cursor: 9,
      events: [
        { ...passwordEvent, cursor: 8, password_generation: 9 },
        futureCredentialEvent,
      ],
      protocol_version: 1,
      type: 'security_snapshot',
    })

    expect(security.credentialWasSuperseded(1, 5)).toBe(true)
    expect(security.credentialWasSuperseded(2, 8)).toBe(true)
    firstSocket?.close()
    await vi.advanceTimersByTimeAsync(500)

    const secondSocket = FakeWebSocket.instances[1]
    expect(secondSocket?.url).toContain('/api/session/events?cursor=9')
    secondSocket?.open()
    secondSocket?.receive({
      cursor: 1,
      events: [passwordEvent],
      protocol_version: 1,
      type: 'security_snapshot',
    })

    expect(security.status).toBe('connected')
    expect(security.cursor).toBe(1)
    expect(security.notices).toEqual([passwordEvent])
    expect(security.credentialWasSuperseded(1, 5)).toBe(false)
    expect(security.credentialWasSuperseded(2, 8)).toBe(false)
  })

  it('abandons a connection that never sends its initial snapshot', async () => {
    vi.spyOn(Math, 'random').mockReturnValue(0.5)
    const security = useSecuritySyncStore()
    security.connect()
    FakeWebSocket.instances[0]?.open()

    await vi.advanceTimersByTimeAsync(5_000)
    expect(security.status).toBe('reconnecting')
    expect(FakeWebSocket.instances).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(500)
    expect(FakeWebSocket.instances).toHaveLength(2)
  })
})
