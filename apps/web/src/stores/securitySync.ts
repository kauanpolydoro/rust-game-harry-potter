import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import {
  isSecurityEventsMessage,
  isSecuritySnapshotMessage,
  type SecurityEventsMessage,
  type SecuritySnapshotMessage,
} from '../contracts/identity-access.generated'

const securitySubprotocol = 'hogwarts.session.v1'
const baseReconnectDelayMilliseconds = 500
const maximumReconnectDelayMilliseconds = 30_000
const hiddenReconnectFloorMilliseconds = 15_000
const stableConnectionMilliseconds = 5_000
const synchronizationTimeoutMilliseconds = 5_000
const maximumNotices = 10

export type SecuritySyncStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'failed'

type SecurityNotice = SecuritySnapshotMessage['events'][number]

function securityEventsUrl(cursor: number): string {
  const scheme = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${scheme}//${window.location.host}/api/session/events?cursor=${cursor}`
}

function eventsFitEnvelope(
  events: SecurityNotice[],
  fromCursor: number,
  cursor: number,
): boolean {
  let previous = fromCursor
  for (const event of events) {
    const eventIsCoherent =
      event.type === 'recovery_credential_regenerated'
        ? event.delivery === 'direct'
          ? event.actor_position === event.target_position
          : event.actor_position !== event.target_position
        : event.type === 'session_revoked' || event.type === 'participant_protected'
          ? event.actor_position === event.target_position
          : true
    if (!eventIsCoherent || event.cursor <= previous || event.cursor > cursor) {
      return false
    }
    previous = event.cursor
  }
  return cursor >= fromCursor
}

export const useSecuritySyncStore = defineStore('securitySync', () => {
  const status = ref<SecuritySyncStatus>('disconnected')
  const cursor = ref(0)
  const notices = ref<SecurityNotice[]>([])
  const sessionInvalidated = ref(false)
  const gameExpired = ref(false)
  const latestPasswordRotationCursor = ref(0)
  const latestCredentialCursorByPosition = ref<Record<number, number>>({})
  const latestNotice = computed(() => notices.value.at(-1) ?? null)
  let socket: WebSocket | null = null
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null
  let stabilityTimer: ReturnType<typeof setTimeout> | null = null
  let synchronizationTimer: ReturnType<typeof setTimeout> | null = null
  let reconnectAttempt = 0
  let generation = 0
  let shouldConnect = false

  function closeSocket(): void {
    generation += 1
    clearStabilityTimer()
    clearSynchronizationTimer()
    const current = socket
    socket = null
    if (current) {
      current.onclose = null
      current.onerror = null
      current.onmessage = null
      current.onopen = null
      current.close()
    }
  }

  function clearReconnectTimer(): void {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
  }

  function clearStabilityTimer(): void {
    if (stabilityTimer) {
      clearTimeout(stabilityTimer)
      stabilityTimer = null
    }
  }

  function clearSynchronizationTimer(): void {
    if (synchronizationTimer) {
      clearTimeout(synchronizationTimer)
      synchronizationTimer = null
    }
  }

  function scheduleReconnect(): void {
    if (!shouldConnect || reconnectTimer) {
      return
    }
    const exponentialDelay = Math.min(
      maximumReconnectDelayMilliseconds,
      baseReconnectDelayMilliseconds * 2 ** Math.min(reconnectAttempt, 10),
    )
    const jitteredDelay = Math.min(
      maximumReconnectDelayMilliseconds,
      Math.round(exponentialDelay * (0.5 + Math.random())),
    )
    const delay =
      typeof document !== 'undefined' && document.visibilityState === 'hidden'
        ? Math.max(hiddenReconnectFloorMilliseconds, jitteredDelay)
        : jitteredDelay
    reconnectAttempt += 1
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null
      openSocket(true)
    }, delay)
  }

  function requestReconnect(): void {
    closeSocket()
    status.value = 'reconnecting'
    scheduleReconnect()
  }

  function appendNotices(events: SecurityNotice[]): void {
    const byCursor = new Map(notices.value.map((notice) => [notice.cursor, notice]))
    for (const event of events) {
      byCursor.set(event.cursor, event)
      if (event.type === 'recovery_password_rotated' || event.type === 'room_protected') {
        latestPasswordRotationCursor.value = Math.max(
          latestPasswordRotationCursor.value,
          event.cursor,
        )
      } else if (
        event.type === 'recovery_credential_regenerated' ||
        event.type === 'participant_protected'
      ) {
        latestCredentialCursorByPosition.value = {
          ...latestCredentialCursorByPosition.value,
          [event.target_position]: Math.max(
            latestCredentialCursorByPosition.value[event.target_position] ?? 0,
            event.cursor,
          ),
        }
      }
    }
    notices.value = [...byCursor.values()]
      .sort((left, right) => left.cursor - right.cursor)
      .slice(-maximumNotices)
  }

  function markSynchronized(): void {
    status.value = 'connected'
    clearSynchronizationTimer()
    clearStabilityTimer()
    const currentSocket = socket
    const currentGeneration = generation
    stabilityTimer = setTimeout(() => {
      if (
        socket === currentSocket &&
        generation === currentGeneration &&
        currentSocket?.readyState === WebSocket.OPEN
      ) {
        reconnectAttempt = 0
      }
    }, stableConnectionMilliseconds)
  }

  function acceptSnapshot(message: SecuritySnapshotMessage): void {
    if (!eventsFitEnvelope(message.events, 0, message.cursor)) {
      requestReconnect()
      return
    }
    const replacesLocalHistory = message.cursor < cursor.value
    if (replacesLocalHistory) {
      notices.value = []
      latestPasswordRotationCursor.value = 0
      latestCredentialCursorByPosition.value = {}
    }
    appendNotices(
      replacesLocalHistory
        ? message.events
        : message.events.filter((event) => event.cursor > cursor.value),
    )
    cursor.value = message.cursor
    markSynchronized()
  }

  function acceptEvents(message: SecurityEventsMessage): void {
    if (
      message.from_cursor !== cursor.value ||
      !eventsFitEnvelope(message.events, message.from_cursor, message.cursor)
    ) {
      requestReconnect()
      return
    }
    appendNotices(message.events)
    cursor.value = message.cursor
    markSynchronized()
  }

  function receive(serialized: unknown): void {
    if (typeof serialized !== 'string') {
      requestReconnect()
      return
    }
    let message: unknown
    try {
      message = JSON.parse(serialized)
    } catch {
      requestReconnect()
      return
    }
    if (isSecuritySnapshotMessage(message)) {
      acceptSnapshot(message)
      return
    }
    if (isSecurityEventsMessage(message)) {
      acceptEvents(message)
      return
    }
    requestReconnect()
  }

  function openSocket(reconnecting: boolean): void {
    if (!shouldConnect || socket) {
      return
    }
    const socketGeneration = generation
    let nextSocket: WebSocket
    try {
      nextSocket = new WebSocket(securityEventsUrl(cursor.value), securitySubprotocol)
    } catch {
      status.value = 'failed'
      scheduleReconnect()
      return
    }
    socket = nextSocket
    status.value = reconnecting ? 'reconnecting' : 'connecting'
    clearSynchronizationTimer()
    synchronizationTimer = setTimeout(() => {
      if (socket === nextSocket && generation === socketGeneration) {
        requestReconnect()
      }
    }, synchronizationTimeoutMilliseconds)
    nextSocket.onopen = () => {
      if (socket !== nextSocket || generation !== socketGeneration) {
        return
      }
      if (nextSocket.protocol !== securitySubprotocol) {
        requestReconnect()
      }
    }
    nextSocket.onmessage = (event) => {
      if (socket === nextSocket && generation === socketGeneration) {
        receive(event.data)
      }
    }
    nextSocket.onerror = () => {
      if (socket === nextSocket && generation === socketGeneration) {
        requestReconnect()
      }
    }
    nextSocket.onclose = (event) => {
      if (socket !== nextSocket || generation !== socketGeneration) {
        return
      }
      socket = null
      clearStabilityTimer()
      clearSynchronizationTimer()
      if (!shouldConnect) {
        status.value = 'disconnected'
        return
      }
      if (event.code === 1008 || event.code === 4001) {
        status.value = 'failed'
        gameExpired.value = event.code === 4001
        sessionInvalidated.value = true
        return
      }
      status.value = 'reconnecting'
      scheduleReconnect()
    }
  }

  function connect(): void {
    shouldConnect = true
    clearReconnectTimer()
    openSocket(status.value !== 'disconnected')
  }

  function retry(): void {
    if (!shouldConnect) {
      return
    }
    clearReconnectTimer()
    closeSocket()
    status.value = 'reconnecting'
    openSocket(true)
  }

  function disconnect(): void {
    shouldConnect = false
    clearReconnectTimer()
    closeSocket()
    status.value = 'disconnected'
    cursor.value = 0
    notices.value = []
    latestPasswordRotationCursor.value = 0
    latestCredentialCursorByPosition.value = {}
    sessionInvalidated.value = false
    gameExpired.value = false
    reconnectAttempt = 0
  }

  function dismissLatestNotice(): void {
    notices.value = notices.value.slice(0, -1)
  }

  function credentialWasSuperseded(targetPosition: number, eventCursor: number): boolean {
    return (
      latestPasswordRotationCursor.value > eventCursor ||
      (latestCredentialCursorByPosition.value[targetPosition] ?? 0) > eventCursor
    )
  }

  return {
    connect,
    credentialWasSuperseded,
    cursor,
    dismissLatestNotice,
    disconnect,
    latestNotice,
    notices,
    receive,
    retry,
    sessionInvalidated,
    gameExpired,
    status,
  }
})
