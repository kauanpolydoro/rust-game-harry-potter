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

  serverClose(code = 1006): void {
    this.readyState = FakeWebSocket.CLOSED
    this.onclose?.({ code })
  }
}

function projection(cursor = 0): GameProjectionResponse {
  const activePosition = (cursor % 2) + 1
  return {
    choice: { status: 'none' },
    effects: { outcomes: [], status: 'idle' },
    game: {
      adventure: { id: 'adventure:001', name: 'Game 1' },
      expires_at: '2026-09-10T12:00:00Z',
      id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
      status: 'in_progress',
    },
    legal_actions: activePosition === 1 ? ['end_hero_actions'] : [],
    queued_effect_count: 0,
    queued_phases: ['end_turn'],
    legal_intentions: {
      acquire_cards: [],
      assign_attack: [],
      end_hero_actions: activePosition === 1,
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
      {
        display_name: 'Luna',
        hero: { id: 'hermione', name: 'Hermione' },
        position: 2,
        resources: { attack: 0, health: 10, influence: 0 },
        role: 'guest',
        hand_count: 0,
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
    turn: {
      active_position: activePosition,
      number: cursor + 1,
      phase: 'hero_actions',
    },
  }
}

function eventBatch(fromCursor: number, cursor: number) {
  return {
    cursor,
    events: Array.from({ length: cursor - fromCursor }, (_, index) => {
      const sequence = fromCursor + index + 1
      const activePosition = (sequence % 2) + 1
      return {
        actor_position: activePosition === 1 ? 2 : 1,
        command_id: '8cbef381-3a98-4731-b16f-8b55db5e8f63',
        control: {
          active_position: activePosition,
          decision_point: { responsible_position: activePosition, type: 'player_intent' },
          phase: 'hero_actions',
          queued_effect_count: 0,
          queued_phases: ['end_turn'],
          status: 'in_progress',
          turn: sequence + 1,
        },
        end_turn: [
          { before: 0, resource: 'attack', type: 'resource_reset' },
          { before: 0, resource: 'influence', type: 'resource_reset' },
        ],
        event_version: 4,
        prng_counter: 0,
        sequence,
        state_version: sequence + 1,
        steps: [
          { effects: [], phase: 'end_turn' },
          { effects: [], phase: 'dark_arts' },
          { effects: [], phase: 'villains' },
        ],
        turn: sequence,
        type: 'turn_completed',
      }
    }),
    from_cursor: fromCursor,
    projection: projection(cursor),
    protocol_version: 2,
    type: 'events',
  }
}

function synchronizedMessage(game: GameProjectionResponse) {
  return {
    cursor: game.snapshot.cursor,
    digest: game.snapshot.digest,
    protocol_version: 2,
    snapshot_version: game.snapshot.snapshot_version,
    type: 'synchronized',
  }
}

function presenceMessage(
  statuses: Array<'online' | 'reconnecting' | 'offline'>,
  requiredParticipantPosition?: number,
) {
  return {
    blocked:
      requiredParticipantPosition !== undefined &&
      statuses[requiredParticipantPosition - 1] !== 'online',
    game_id: projection().game.id,
    participants: statuses.map((status, index) => ({ position: index + 1, status })),
    protocol_version: 2,
    ...(requiredParticipantPosition === undefined
      ? {}
      : { required_participant_position: requiredParticipantPosition }),
    type: 'presence',
  }
}

describe('official game synchronization', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.spyOn(Math, 'random').mockReturnValue(0)
    FakeWebSocket.instances = []
    vi.stubGlobal('WebSocket', FakeWebSocket)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    useGameSyncStore().disconnect()
    vi.useRealTimers()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('opens a versioned same-origin stream and replaces state from a snapshot', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()

    sync.connect(roomAccess.game)

    const socket = FakeWebSocket.instances[0]
    expect(socket?.requestedProtocol).toBe('hogwarts.realtime.v2')
    expect(socket?.url).toContain('/api/games/current/events?cursor=0&snapshot_version=1')
    expect(socket?.url).toContain(encodeURIComponent(projection().snapshot.digest))
    socket?.open()
    expect(sync.status).toBe('connecting')
    expect(sync.commandsFrozen).toBe(true)
    socket?.receive(synchronizedMessage(projection()))
    expect(sync.status).toBe('connected')
    expect(sync.commandsFrozen).toBe(false)

    const replacement = projection(1)
    socket?.receive({
      cursor: 1,
      projection: replacement,
      protocol_version: 2,
      type: 'snapshot',
    })

    expect(roomAccess.game).toEqual(replacement)
    expect(sync.cursor).toBe(1)
  })

  it('derives the coordination block only from the required participant presence', () => {
    const roomAccess = useRoomAccessStore()
    const officialProjection = projection()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()
    socket?.receive(synchronizedMessage(roomAccess.game))

    socket?.receive(presenceMessage(['offline', 'online'], 1))

    expect(sync.participantPresence).toEqual({ 1: 'offline', 2: 'online' })
    expect(sync.requiredParticipantPosition).toBe(1)
    expect(sync.gameBlocked).toBe(true)
    expect(roomAccess.game).toEqual(officialProjection)
    expect(sync.cursor).toBe(0)
    expect(sync.commandsFrozen).toBe(false)

    socket?.receive(presenceMessage(['online', 'offline'], 1))
    expect(sync.gameBlocked).toBe(false)

    socket?.receive(presenceMessage(['offline', 'offline']))
    expect(sync.gameBlocked).toBe(false)
    expect(sync.requiredParticipantPosition).toBeNull()
  })

  it('does not let malformed presence alter official synchronization', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()

    socket?.receive({
      ...presenceMessage(['online', 'offline'], 1),
      blocked: true,
    })

    expect(sync.participantPresence).toEqual({})
    expect(sync.commandsFrozen).toBe(true)
    vi.advanceTimersByTime(250)
    expect(FakeWebSocket.instances).toHaveLength(1)

    socket?.receive(synchronizedMessage(roomAccess.game))
    expect(sync.commandsFrozen).toBe(false)
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
    expect(roomAccess.game?.turn.phase).toBe('hero_actions')

    const duplicate = eventBatch(0, 1)
    duplicate.projection.game.expires_at = '2099-01-01T00:00:00Z'
    socket?.receive(duplicate)
    expect(roomAccess.game?.game.expires_at).toBe('2026-09-10T12:00:00Z')
  })

  it('schedules a full snapshot without an immediate loop when a batch has a cursor gap', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()

    socket?.receive(eventBatch(1, 2))

    expect(FakeWebSocket.instances).toHaveLength(1)
    expect(sync.status).toBe('reconnecting')
    vi.advanceTimersByTime(249)
    expect(FakeWebSocket.instances).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(FakeWebSocket.instances[1]?.url).toContain('snapshot_version=0')
    expect(roomAccess.game?.snapshot.cursor).toBe(0)
    expect(sync.commandsFrozen).toBe(true)
  })

  it('rejects event payload fields outside the public allowlist without a tight loop', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()
    const unsafe = eventBatch(0, 1)
    Object.assign(unsafe.events[0] ?? {}, { participant_id: 'private' })

    socket?.receive(unsafe)

    expect(FakeWebSocket.instances).toHaveLength(1)
    vi.advanceTimersByTime(250)
    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(roomAccess.game?.snapshot.cursor).toBe(0)
  })

  it('replaces a newer local cache from a full authoritative snapshot and cancels animations', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection(1)
    const sync = useGameSyncStore()
    const cancelAnimation = vi.fn()
    sync.registerAnimationCancellation(cancelAnimation)
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()

    socket?.receive({
      cursor: 0,
      projection: projection(0),
      protocol_version: 2,
      type: 'snapshot',
    })

    expect(roomAccess.game?.snapshot.cursor).toBe(0)
    expect(sync.cursor).toBe(0)
    expect(cancelAnimation).toHaveBeenCalledOnce()
    expect(sync.status).toBe('connected')
    expect(FakeWebSocket.instances).toHaveLength(1)
  })

  it('falls back to a full snapshot when a synchronization acknowledgement has another digest', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()
    const incompatible = synchronizedMessage(roomAccess.game)
    incompatible.digest = `blake3:${'f'.repeat(64)}`

    socket?.receive(incompatible)

    expect(sync.commandsFrozen).toBe(true)
    expect(sync.status).toBe('reconnecting')
    vi.advanceTimersByTime(250)
    expect(FakeWebSocket.instances[1]?.url).toContain('snapshot_version=0')
  })

  it('keeps commands frozen and requests a Snapshot when synchronization times out', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    FakeWebSocket.instances[0]?.open()

    vi.advanceTimersByTime(4_999)
    expect(FakeWebSocket.instances).toHaveLength(1)
    expect(sync.commandsFrozen).toBe(true)
    vi.advanceTimersByTime(1)
    expect(sync.status).toBe('reconnecting')
    vi.advanceTimersByTime(250)

    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(FakeWebSocket.instances[1]?.url).toContain('snapshot_version=0')
    expect(sync.commandsFrozen).toBe(true)
  })

  it('recovers instead of applying an event batch with a regressive state version', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection(1)
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()
    const regressive = eventBatch(1, 2)
    regressive.projection.snapshot.state_version = 1

    socket?.receive(regressive)

    expect(roomAccess.game?.snapshot.cursor).toBe(1)
    expect(sync.status).toBe('reconnecting')
    vi.advanceTimersByTime(250)
    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(FakeWebSocket.instances[1]?.url).toContain('snapshot_version=0')
  })

  it('resets synchronization coordinates when the active game changes', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection(5)
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)

    const replacement = projection(0)
    replacement.game.id = '9db43390-a2ea-42df-8d18-ce4d1e530302'
    roomAccess.game = replacement
    sync.connect(replacement)

    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(FakeWebSocket.instances[1]?.url).toContain('cursor=0')
    expect(sync.cursor).toBe(0)
    expect(sync.snapshotVersion).toBe(1)
  })

  it('uses bounded exponential backoff with jitter until a connection is stable', () => {
    vi.mocked(Math.random).mockReturnValue(1)
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)

    for (const delay of [750, 1_500, 3_000, 6_000, 12_000, 24_000, 30_000, 30_000]) {
      FakeWebSocket.instances.at(-1)?.serverClose()
      const count = FakeWebSocket.instances.length
      vi.advanceTimersByTime(delay - 1)
      expect(FakeWebSocket.instances).toHaveLength(count)
      vi.advanceTimersByTime(1)
      expect(FakeWebSocket.instances).toHaveLength(count + 1)
    }
  })

  it('marks the session invalid when the server closes the game stream for policy', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)
    const socket = FakeWebSocket.instances[0]
    socket?.open()
    socket?.receive(synchronizedMessage(roomAccess.game))

    socket?.serverClose(1008)

    expect(sync.status).toBe('failed')
    expect(sync.sessionInvalidated).toBe(true)
    vi.advanceTimersByTime(30_000)
    expect(FakeWebSocket.instances).toHaveLength(1)
  })

  it('resets the retry sequence only after the connection remains stable', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)

    FakeWebSocket.instances[0]?.serverClose()
    vi.advanceTimersByTime(250)
    const recovered = FakeWebSocket.instances[1]
    recovered?.open()
    recovered?.receive(synchronizedMessage(projection()))
    vi.advanceTimersByTime(5_000)
    recovered?.serverClose()

    vi.advanceTimersByTime(249)
    expect(FakeWebSocket.instances).toHaveLength(2)
    vi.advanceTimersByTime(1)
    expect(FakeWebSocket.instances).toHaveLength(3)
  })

  it('cancels a scheduled retry when the player reconnects manually', () => {
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)

    FakeWebSocket.instances[0]?.serverClose()
    sync.resynchronize()
    const manualSocket = FakeWebSocket.instances[1]
    manualSocket?.open()
    manualSocket?.receive({
      cursor: 0,
      projection: projection(),
      protocol_version: 2,
      type: 'snapshot',
    })
    vi.advanceTimersByTime(500)

    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(sync.status).toBe('connected')
  })

  it('uses a conservative retry floor while the page is in the background', () => {
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)

    FakeWebSocket.instances[0]?.serverClose()
    vi.advanceTimersByTime(14_999)
    expect(FakeWebSocket.instances).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(FakeWebSocket.instances).toHaveLength(2)
  })

  it('accelerates a background retry when the page becomes visible', () => {
    const visibility = vi
      .spyOn(document, 'visibilityState', 'get')
      .mockReturnValue('hidden')
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()
    sync.connect(roomAccess.game)

    FakeWebSocket.instances[0]?.serverClose()
    visibility.mockReturnValue('visible')
    document.dispatchEvent(new Event('visibilitychange'))

    vi.advanceTimersByTime(249)
    expect(FakeWebSocket.instances).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(FakeWebSocket.instances).toHaveLength(2)
  })

  it('pauses retries while offline and resumes after the browser is online', () => {
    const online = vi.spyOn(window.navigator, 'onLine', 'get').mockReturnValue(false)
    const roomAccess = useRoomAccessStore()
    roomAccess.game = projection()
    const sync = useGameSyncStore()

    sync.connect(roomAccess.game)
    expect(sync.status).toBe('failed')
    expect(FakeWebSocket.instances).toHaveLength(0)
    vi.advanceTimersByTime(30_000)
    expect(FakeWebSocket.instances).toHaveLength(0)

    online.mockReturnValue(true)
    window.dispatchEvent(new Event('online'))
    vi.advanceTimersByTime(250)
    expect(FakeWebSocket.instances).toHaveLength(1)
    expect(sync.status).toBe('reconnecting')
  })

  it('keeps sockets and timers isolated between Pinia instances', () => {
    const firstPinia = createPinia()
    setActivePinia(firstPinia)
    const firstRoom = useRoomAccessStore()
    firstRoom.game = projection()
    const firstSync = useGameSyncStore()
    firstSync.connect(firstRoom.game)
    const firstSocket = FakeWebSocket.instances[0]

    const secondPinia = createPinia()
    setActivePinia(secondPinia)
    const secondRoom = useRoomAccessStore()
    secondRoom.game = projection()
    const secondSync = useGameSyncStore()
    secondSync.connect(secondRoom.game)
    const secondSocket = FakeWebSocket.instances[1]

    firstSync.disconnect()
    expect(firstSocket?.readyState).toBe(FakeWebSocket.CLOSED)
    expect(secondSocket?.readyState).toBe(FakeWebSocket.CONNECTING)
    secondSync.disconnect()
  })
})
