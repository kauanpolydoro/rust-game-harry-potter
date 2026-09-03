import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { GameProjectionResponse } from '../contracts/identity-access.generated'
import { useGameSyncStore } from './gameSync'
import { useRoomAccessStore } from './roomAccess'

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
  onclose: (() => void) | null = null

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
    this.onclose?.()
  }
}

function projection(cursor = 0): GameProjectionResponse {
  return {
    choice: { status: 'none' },
    game: {
      adventure: { id: 'adventure:001', name: 'Game 1' },
      expires_at: '2026-09-10T12:00:00Z',
      id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
      status: 'in_progress',
    },
    legal_actions: cursor === 0 ? ['complete_dark_arts'] : [],
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
      {
        display_name: 'Luna',
        hero: { id: 'hermione', name: 'Hermione' },
        position: 2,
        role: 'guest',
      },
    ],
    snapshot: {
      cursor,
      digest: `blake3:${(cursor === 0 ? 'c' : 'd').repeat(64)}`,
      sequence: cursor,
      snapshot_version: 1,
      state_version: cursor + 1,
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
    turn: {
      active_position: 1,
      number: 1,
      phase: cursor === 0 ? 'dark_arts' : 'hero_action',
    },
  }
}

function eventBatch(fromCursor: number, cursor: number) {
  return {
    cursor,
    events: Array.from({ length: cursor - fromCursor }, (_, index) => ({
      actor_position: 1,
      command_id: '8cbef381-3a98-4731-b16f-8b55db5e8f63',
      event_version: 1,
      sequence: fromCursor + index + 1,
      state_version: fromCursor + index + 2,
      turn: 1,
      type: 'dark_arts_completed',
    })),
    from_cursor: fromCursor,
    projection: projection(cursor),
    protocol_version: 1,
    type: 'events',
  }
}

describe('official game synchronization', () => {
  beforeEach(() => {
    FakeWebSocket.instances = []
    vi.stubGlobal('WebSocket', FakeWebSocket)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    useGameSyncStore().disconnect()
    vi.unstubAllGlobals()
  })

  it('opens a versioned same-origin stream and replaces state from a snapshot', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()

    sync.connect(roomAccess.game)

    const socket = FakeWebSocket.instances[0]
    expect(socket?.requestedProtocol).toBe('hogwarts.realtime.v1')
    expect(socket?.url).toContain('/api/games/current/events?cursor=0&snapshot_version=1')
    socket?.open()
    expect(sync.status).toBe('connected')

    const replacement = projection(1)
    socket?.receive({
      cursor: 1,
      projection: replacement,
      protocol_version: 1,
      type: 'snapshot',
    })

    expect(roomAccess.game).toEqual(replacement)
    expect(sync.cursor).toBe(1)
  })

  it('applies a contiguous event batch once and ignores its redelivery', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()

    socket?.receive(eventBatch(0, 1))
    expect(roomAccess.game?.snapshot.cursor).toBe(1)
    expect(roomAccess.game?.turn.phase).toBe('hero_action')

    const duplicate = eventBatch(0, 1)
    duplicate.projection.game.expires_at = '2099-01-01T00:00:00Z'
    socket?.receive(duplicate)
    expect(roomAccess.game?.game.expires_at).toBe('2026-09-10T12:00:00Z')
  })

  it('requests a full snapshot when a batch has a cursor gap', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()

    socket?.receive(eventBatch(1, 2))

    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(FakeWebSocket.instances[1]?.url).toContain('snapshot_version=0')
    expect(roomAccess.game?.snapshot.cursor).toBe(0)
  })

  it('rejects event payload fields outside the public allowlist', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()
    const unsafe = eventBatch(0, 1)
    Object.assign(unsafe.events[0] ?? {}, { participant_id: 'private' })

    socket?.receive(unsafe)

    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(roomAccess.game?.snapshot.cursor).toBe(0)
  })
})
