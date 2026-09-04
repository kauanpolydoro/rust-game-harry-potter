import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import {
  isRealtimeEventBatchMessage,
  isRealtimePresenceMessage,
  isRealtimeSnapshotMessage,
  isRealtimeSynchronizedMessage,
  type GameProjectionResponse,
  type ParticipantPresence,
  type RealtimeEventBatchMessage,
  type RealtimePresenceMessage,
} from '../contracts/identity-access.generated'
import { useRoomAccessStore } from './roomAccess'

const realtimeSubprotocol = 'hogwarts.realtime.v2'
const baseReconnectDelayMilliseconds = 500
const maximumReconnectDelayMilliseconds = 30_000
const hiddenReconnectFloorMilliseconds = 15_000
const stableConnectionMilliseconds = 5_000
const synchronizationTimeoutMilliseconds = 5_000

export type GameSyncStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'failed'

interface ConnectionRequest {
  cursor: number
  digest: string
  forceSnapshot: boolean
  gameId: string
  snapshotVersion: number
}

interface ConnectionCallbacks {
  currentRequest: () => ConnectionRequest | null
  discardAnimations: () => void
  invalidateSession: (expired: boolean) => void
  receive: (serialized: unknown) => void
  revalidateSession: () => void
  updateStatus: (status: GameSyncStatus) => void
}

function realtimeUrl(cursor: number, snapshotVersion: number, digest: string): string {
  const scheme = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const query = new URLSearchParams({
    cursor: String(cursor),
    snapshot_version: String(snapshotVersion),
    digest,
  })
  return `${scheme}//${window.location.host}/api/games/current/events?${query}`
}

function eventBatchContinuesFrom(message: RealtimeEventBatchMessage, cursor: number): boolean {
  const batchIsContiguous =
    message.events.length === message.cursor - message.from_cursor &&
    message.events.every(
      (event, index) =>
        event.sequence === message.from_cursor + index + 1 &&
        event.state_version === event.sequence + 1,
    )
  if (!batchIsContiguous) {
    return false
  }
  if (message.cursor <= cursor) {
    return true
  }
  const unseen = message.events.filter((event) => event.sequence > cursor)
  return (
    unseen.length === message.cursor - cursor &&
    unseen.every((event, index) => event.sequence === cursor + index + 1)
  )
}

function projectionHasCanonicalCursor(projection: GameProjectionResponse): boolean {
  return (
    projection.snapshot.cursor === projection.snapshot.sequence &&
    projection.snapshot.state_version === projection.snapshot.sequence + 1
  )
}

function presenceMatchesGame(
  message: RealtimePresenceMessage,
  game: GameProjectionResponse,
): boolean {
  if (message.game_id !== game.game.id) {
    return false
  }
  const gamePositions = game.participants.map((participant) => participant.position).sort()
  const presencePositions = message.participants.map((participant) => participant.position).sort()
  if (
    new Set(presencePositions).size !== presencePositions.length ||
    presencePositions.length !== gamePositions.length ||
    presencePositions.some((position, index) => position !== gamePositions[index])
  ) {
    return false
  }
  const requiredPosition = message.required_participant_position
  if (requiredPosition !== undefined && !presencePositions.includes(requiredPosition)) {
    return false
  }
  const requiredPresence = message.participants.find(
    (participant) => participant.position === requiredPosition,
  )
  return message.blocked === (requiredPosition !== undefined && requiredPresence?.status !== 'online')
}

class GameSyncConnection {
  private activeGameId: string | null = null
  private generation = 0
  private reconnectAttempt = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private socket: WebSocket | null = null
  private stabilityTimer: ReturnType<typeof setTimeout> | null = null
  private synchronizationTimer: ReturnType<typeof setTimeout> | null = null
  private forceSnapshotOnRetry = false
  private listeningForBrowserState = false

  constructor(private readonly callbacks: ConnectionCallbacks) {}

  connect(request: ConnectionRequest): void {
    const changingGame = this.activeGameId !== request.gameId
    if (
      !changingGame &&
      !request.forceSnapshot &&
      this.socket &&
      (this.socket.readyState === WebSocket.CONNECTING ||
        this.socket.readyState === WebSocket.OPEN)
    ) {
      return
    }

    this.clearReconnectTimer()
    this.closeCurrentSocket()
    if (changingGame) {
      this.reconnectAttempt = 0
      this.forceSnapshotOnRetry = false
    }
    this.activeGameId = request.gameId
    this.attachBrowserStateListeners()
    if (!this.isOnline()) {
      this.callbacks.updateStatus('failed')
      this.forceSnapshotOnRetry ||= request.forceSnapshot
      return
    }

    const generation = this.generation
    const requestedVersion = request.forceSnapshot ? 0 : request.snapshotVersion
    let socket: WebSocket
    try {
      socket = new WebSocket(
        realtimeUrl(request.cursor, requestedVersion, request.digest),
        realtimeSubprotocol,
      )
    } catch {
      this.callbacks.updateStatus('failed')
      this.forceSnapshotOnRetry ||= request.forceSnapshot
      this.scheduleReconnect(generation)
      return
    }
    this.socket = socket
    this.callbacks.updateStatus(
      request.forceSnapshot || this.reconnectAttempt > 0 ? 'reconnecting' : 'connecting',
    )
    this.clearSynchronizationTimer()
    this.synchronizationTimer = setTimeout(() => {
      if (this.isCurrent(socket, generation)) {
        this.requestRecovery(true)
      }
    }, synchronizationTimeoutMilliseconds)

    socket.onopen = () => {
      if (!this.isCurrent(socket, generation)) {
        return
      }
      if (socket.protocol !== realtimeSubprotocol) {
        this.requestRecovery(true)
      }
    }
    socket.onmessage = (event) => {
      if (this.isCurrent(socket, generation)) {
        this.callbacks.receive(event.data)
      }
    }
    socket.onerror = () => {
      if (this.isCurrent(socket, generation)) {
        this.callbacks.updateStatus('failed')
      }
    }
    socket.onclose = (event) => {
      if (!this.isCurrent(socket, generation)) {
        return
      }
      this.socket = null
      this.clearStabilityTimer()
      this.clearSynchronizationTimer()
      this.callbacks.discardAnimations()
      if (event.code === 1008 || event.code === 4001) {
        this.callbacks.updateStatus('failed')
        this.callbacks.invalidateSession(event.code === 4001)
        return
      }
      this.callbacks.revalidateSession()
      this.callbacks.updateStatus('reconnecting')
      this.scheduleReconnect(generation)
    }
  }

  requestRecovery(forceSnapshot: boolean): void {
    if (!this.activeGameId) {
      return
    }
    this.forceSnapshotOnRetry ||= forceSnapshot
    this.callbacks.discardAnimations()
    this.closeCurrentSocket()
    this.callbacks.updateStatus(this.isOnline() ? 'reconnecting' : 'failed')
    this.scheduleReconnect(this.generation)
  }

  disconnect(): void {
    this.closeCurrentSocket()
    this.clearReconnectTimer()
    this.clearStabilityTimer()
    this.clearSynchronizationTimer()
    this.activeGameId = null
    this.forceSnapshotOnRetry = false
    this.reconnectAttempt = 0
    this.detachBrowserStateListeners()
  }

  markSynchronized(): void {
    const socket = this.socket
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return
    }
    this.callbacks.updateStatus('connected')
    this.clearSynchronizationTimer()
    this.clearStabilityTimer()
    const generation = this.generation
    this.stabilityTimer = setTimeout(() => {
      if (this.isCurrent(socket, generation) && socket.readyState === WebSocket.OPEN) {
        this.reconnectAttempt = 0
      }
    }, stableConnectionMilliseconds)
  }

  private scheduleReconnect(generation: number): void {
    if (this.reconnectTimer || generation !== this.generation || !this.activeGameId) {
      return
    }
    if (!this.isOnline()) {
      this.callbacks.updateStatus('failed')
      return
    }

    const exponentialDelay = Math.min(
      maximumReconnectDelayMilliseconds,
      baseReconnectDelayMilliseconds * 2 ** Math.min(this.reconnectAttempt, 10),
    )
    const jitteredDelay = Math.min(
      maximumReconnectDelayMilliseconds,
      Math.round(exponentialDelay * (0.5 + Math.random())),
    )
    const delay = this.isHidden()
      ? Math.max(hiddenReconnectFloorMilliseconds, jitteredDelay)
      : jitteredDelay
    this.reconnectAttempt += 1
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      if (generation !== this.generation || !this.isOnline()) {
        this.callbacks.updateStatus('failed')
        return
      }
      const current = this.callbacks.currentRequest()
      if (!current || current.gameId !== this.activeGameId) {
        return
      }
      const forceSnapshot = this.forceSnapshotOnRetry
      this.forceSnapshotOnRetry = false
      this.connect({ ...current, forceSnapshot })
    }, delay)
  }

  private closeCurrentSocket(): void {
    this.generation += 1
    const socket = this.socket
    this.socket = null
    this.clearStabilityTimer()
    this.clearSynchronizationTimer()
    if (socket) {
      socket.onclose = null
      socket.onerror = null
      socket.onmessage = null
      socket.onopen = null
      socket.close()
    }
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
  }

  private clearStabilityTimer(): void {
    if (this.stabilityTimer) {
      clearTimeout(this.stabilityTimer)
      this.stabilityTimer = null
    }
  }

  private clearSynchronizationTimer(): void {
    if (this.synchronizationTimer) {
      clearTimeout(this.synchronizationTimer)
      this.synchronizationTimer = null
    }
  }

  private isCurrent(socket: WebSocket, generation: number): boolean {
    return generation === this.generation && socket === this.socket
  }

  private isOnline(): boolean {
    return typeof navigator === 'undefined' || navigator.onLine
  }

  private isHidden(): boolean {
    return typeof document !== 'undefined' && document.visibilityState === 'hidden'
  }

  private attachBrowserStateListeners(): void {
    if (this.listeningForBrowserState || typeof window === 'undefined') {
      return
    }
    window.addEventListener('offline', this.handleOffline)
    window.addEventListener('online', this.handleOnline)
    document.addEventListener('visibilitychange', this.handleVisibilityChange)
    this.listeningForBrowserState = true
  }

  private detachBrowserStateListeners(): void {
    if (!this.listeningForBrowserState || typeof window === 'undefined') {
      return
    }
    window.removeEventListener('offline', this.handleOffline)
    window.removeEventListener('online', this.handleOnline)
    document.removeEventListener('visibilitychange', this.handleVisibilityChange)
    this.listeningForBrowserState = false
  }

  private readonly handleOffline = (): void => {
    if (!this.activeGameId || this.isOnline()) {
      return
    }
    this.clearReconnectTimer()
    this.callbacks.discardAnimations()
    this.closeCurrentSocket()
    this.callbacks.updateStatus('failed')
  }

  private readonly handleOnline = (): void => {
    if (!this.socket && !this.reconnectTimer && this.activeGameId) {
      this.callbacks.updateStatus('reconnecting')
      this.scheduleReconnect(this.generation)
    }
  }

  private readonly handleVisibilityChange = (): void => {
    if (this.isHidden()) {
      this.callbacks.discardAnimations()
      return
    }
    if (!this.isHidden() && !this.socket && this.activeGameId) {
      if (this.reconnectTimer) {
        this.reconnectAttempt = Math.max(0, this.reconnectAttempt - 1)
      }
      this.clearReconnectTimer()
      this.callbacks.updateStatus('reconnecting')
      this.scheduleReconnect(this.generation)
    }
  }
}

export const useGameSyncStore = defineStore('gameSync', () => {
  const roomAccess = useRoomAccessStore()
  const status = ref<GameSyncStatus>('disconnected')
  const sessionInvalidated = ref(false)
  const gameExpired = ref(false)
  const cursor = ref(0)
  const digest = ref('')
  const snapshotVersion = ref(1)
  const currentGameId = ref<string | null>(null)
  const participantPresence = ref<Record<number, ParticipantPresence['status']>>({})
  const requiredParticipantPosition = ref<number | null>(null)
  const gameBlocked = ref(false)
  const animationCancellations = new Set<() => void>()
  const commandsFrozen = computed(() => status.value !== 'connected')

  function discardAnimations(): void {
    const cancellations = [...animationCancellations]
    animationCancellations.clear()
    for (const cancel of cancellations) {
      try {
        cancel()
      } catch {
        // A broken visual effect must not prevent authoritative convergence.
      }
    }
  }

  const connection = new GameSyncConnection({
    currentRequest: () => {
      const game = roomAccess.game
      if (!game || game.game.id !== currentGameId.value) {
        return null
      }
      return {
        cursor: cursor.value,
        digest: digest.value,
        forceSnapshot: false,
        gameId: game.game.id,
        snapshotVersion: snapshotVersion.value,
      }
    },
    discardAnimations,
    invalidateSession: (expired) => {
      gameExpired.value = expired
      sessionInvalidated.value = true
    },
    receive,
    revalidateSession: () => {
      void roomAccess.revalidateSession()
    },
    updateStatus: (nextStatus) => {
      status.value = nextStatus
    },
  })

  function connect(game: GameProjectionResponse, forceSnapshot = false): void {
    if (currentGameId.value !== game.game.id) {
      cursor.value = 0
      snapshotVersion.value = 1
      currentGameId.value = game.game.id
      cursor.value = game.snapshot.cursor
      digest.value = game.snapshot.digest
      snapshotVersion.value = game.snapshot.snapshot_version
      participantPresence.value = {}
      requiredParticipantPosition.value = null
      gameBlocked.value = false
    } else {
      cursor.value = Math.max(cursor.value, game.snapshot.cursor)
      if (cursor.value === game.snapshot.cursor) {
        digest.value = game.snapshot.digest
      }
      snapshotVersion.value = game.snapshot.snapshot_version
    }
    connection.connect({
      cursor: cursor.value,
      digest: digest.value,
      forceSnapshot,
      gameId: game.game.id,
      snapshotVersion: snapshotVersion.value,
    })
  }

  function receive(serialized: unknown): void {
    if (typeof serialized !== 'string') {
      connection.requestRecovery(true)
      return
    }
    let message: unknown
    try {
      message = JSON.parse(serialized)
    } catch {
      connection.requestRecovery(true)
      return
    }

    const current = roomAccess.game
    if (!current || current.game.id !== currentGameId.value) {
      return
    }
    if (isRealtimePresenceMessage(message)) {
      if (!presenceMatchesGame(message, current)) {
        return
      }
      participantPresence.value = Object.fromEntries(
        message.participants.map((participant) => [participant.position, participant.status]),
      )
      requiredParticipantPosition.value = message.required_participant_position ?? null
      gameBlocked.value = message.blocked
      return
    }
    if (isRealtimeSnapshotMessage(message)) {
      if (
        message.cursor !== message.projection.snapshot.cursor ||
        message.cursor !== message.projection.snapshot.sequence ||
        message.projection.game.id !== current.game.id ||
        !projectionHasCanonicalCursor(message.projection)
      ) {
        connection.requestRecovery(true)
        return
      }
      discardAnimations()
      cursor.value = message.cursor
      digest.value = message.projection.snapshot.digest
      snapshotVersion.value = message.projection.snapshot.snapshot_version
      roomAccess.replaceGameProjection(message.projection)
      connection.markSynchronized()
      return
    }
    if (isRealtimeSynchronizedMessage(message)) {
      if (
        message.cursor !== cursor.value ||
        message.cursor !== current.snapshot.cursor ||
        message.snapshot_version !== snapshotVersion.value ||
        message.snapshot_version !== current.snapshot.snapshot_version ||
        message.digest !== digest.value ||
        message.digest !== current.snapshot.digest
      ) {
        connection.requestRecovery(true)
        return
      }
      connection.markSynchronized()
      return
    }
    if (isRealtimeEventBatchMessage(message)) {
      if (
        message.projection.game.id !== current.game.id ||
        message.cursor !== message.projection.snapshot.cursor ||
        message.cursor !== message.projection.snapshot.sequence ||
        message.projection.snapshot.snapshot_version !== snapshotVersion.value ||
        message.projection.snapshot.state_version < current.snapshot.state_version ||
        !projectionHasCanonicalCursor(message.projection) ||
        message.from_cursor > cursor.value ||
        !eventBatchContinuesFrom(message, cursor.value)
      ) {
        connection.requestRecovery(true)
        return
      }
      if (message.cursor <= cursor.value) {
        return
      }
      cursor.value = message.cursor
      digest.value = message.projection.snapshot.digest
      snapshotVersion.value = message.projection.snapshot.snapshot_version
      roomAccess.advanceGameProjection(message.projection)
      connection.markSynchronized()
      return
    }
    connection.requestRecovery(true)
  }

  function resynchronize(): void {
    const game = roomAccess.game
    if (game) {
      connect(game, true)
    }
  }

  function disconnect(): void {
    connection.disconnect()
    currentGameId.value = null
    status.value = 'disconnected'
    cursor.value = 0
    digest.value = ''
    snapshotVersion.value = 1
    participantPresence.value = {}
    requiredParticipantPosition.value = null
    gameBlocked.value = false
    sessionInvalidated.value = false
    gameExpired.value = false
    discardAnimations()
  }

  function presenceFor(position: number): ParticipantPresence['status'] | null {
    return participantPresence.value[position] ?? null
  }

  function registerAnimationCancellation(cancel: () => void): () => void {
    animationCancellations.add(cancel)
    return () => animationCancellations.delete(cancel)
  }

  return {
    commandsFrozen,
    connect,
    cursor,
    disconnect,
    gameBlocked,
    participantPresence,
    presenceFor,
    registerAnimationCancellation,
    receive,
    requiredParticipantPosition,
    resynchronize,
    sessionInvalidated,
    gameExpired,
    snapshotVersion,
    status,
  }
})
