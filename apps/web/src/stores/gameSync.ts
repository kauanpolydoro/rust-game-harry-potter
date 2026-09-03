import { defineStore } from 'pinia'

import {
  isRealtimeEventBatchMessage,
  isRealtimeSnapshotMessage,
  type GameProjectionResponse,
  type RealtimeEventBatchMessage,
} from '../contracts/identity-access.generated'
import { useRoomAccessStore } from './roomAccess'

const realtimeSubprotocol = 'hogwarts.realtime.v1'
const reconnectDelayMilliseconds = 500

type GameSyncStatus = 'disconnected' | 'connecting' | 'connected' | 'reconnecting' | 'failed'

let activeSocket: WebSocket | null = null
let activeGameId: string | null = null
let reconnectTimer: ReturnType<typeof setTimeout> | null = null
let connectionGeneration = 0

function realtimeUrl(cursor: number, snapshotVersion: number): string {
  const scheme = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const query = new URLSearchParams({
    cursor: String(cursor),
    snapshot_version: String(snapshotVersion),
  })
  return `${scheme}//${window.location.host}/api/games/current/events?${query}`
}

function eventBatchContinuesFrom(message: RealtimeEventBatchMessage, cursor: number): boolean {
  if (message.cursor <= cursor) {
    return true
  }
  const batchIsContiguous =
    message.events.length === message.cursor - message.from_cursor &&
    message.events.every(
      (event, index) => event.sequence === message.from_cursor + index + 1,
    )
  const unseen = message.events.filter((event) => event.sequence > cursor)
  return (
    batchIsContiguous &&
    unseen.length === message.cursor - cursor &&
    unseen.every((event, index) => event.sequence === cursor + index + 1)
  )
}

export const useGameSyncStore = defineStore('gameSync', {
  state: (): {
    status: GameSyncStatus
    cursor: number
    snapshotVersion: number
  } => ({
    status: 'disconnected',
    cursor: 0,
    snapshotVersion: 1,
  }),
  actions: {
    connect(game: GameProjectionResponse, forceSnapshot = false): void {
      this.cursor = Math.max(this.cursor, game.snapshot.cursor)
      this.snapshotVersion = game.snapshot.snapshot_version
      if (
        !forceSnapshot &&
        activeGameId === game.game.id &&
        activeSocket &&
        (activeSocket.readyState === WebSocket.CONNECTING ||
          activeSocket.readyState === WebSocket.OPEN)
      ) {
        return
      }

      this.closeSocket()
      activeGameId = game.game.id
      if (typeof WebSocket === 'undefined') {
        this.status = 'failed'
        return
      }

      const generation = connectionGeneration
      const requestedSnapshotVersion = forceSnapshot ? 0 : this.snapshotVersion
      let socket: WebSocket
      try {
        socket = new WebSocket(
          realtimeUrl(this.cursor, requestedSnapshotVersion),
          realtimeSubprotocol,
        )
      } catch {
        this.status = 'failed'
        this.scheduleReconnect(generation)
        return
      }
      activeSocket = socket
      this.status = forceSnapshot ? 'reconnecting' : 'connecting'

      socket.onopen = () => {
        if (generation !== connectionGeneration || socket !== activeSocket) {
          return
        }
        if (socket.protocol !== realtimeSubprotocol) {
          this.forceSnapshot()
          return
        }
        this.status = 'connected'
      }
      socket.onmessage = (event) => {
        if (generation !== connectionGeneration || socket !== activeSocket) {
          return
        }
        this.receive(event.data)
      }
      socket.onerror = () => {
        if (generation === connectionGeneration && socket === activeSocket) {
          this.status = 'failed'
        }
      }
      socket.onclose = () => {
        if (generation !== connectionGeneration || socket !== activeSocket) {
          return
        }
        activeSocket = null
        this.status = 'reconnecting'
        this.scheduleReconnect(generation)
      }
    },
    receive(serialized: unknown): void {
      if (typeof serialized !== 'string') {
        this.forceSnapshot()
        return
      }
      let message: unknown
      try {
        message = JSON.parse(serialized)
      } catch {
        this.forceSnapshot()
        return
      }

      const roomAccess = useRoomAccessStore()
      const current = roomAccess.game
      if (!current || current.game.id !== activeGameId) {
        return
      }
      if (isRealtimeSnapshotMessage(message)) {
        if (
          message.cursor !== message.projection.snapshot.cursor ||
          message.cursor !== message.projection.snapshot.sequence ||
          message.projection.game.id !== current.game.id
        ) {
          this.forceSnapshot()
          return
        }
        this.cursor = message.cursor
        this.snapshotVersion = message.projection.snapshot.snapshot_version
        roomAccess.replaceGameProjection(message.projection)
        return
      }
      if (isRealtimeEventBatchMessage(message)) {
        if (
          message.projection.game.id !== current.game.id ||
          message.cursor !== message.projection.snapshot.cursor ||
          message.cursor !== message.projection.snapshot.sequence ||
          message.from_cursor > this.cursor ||
          !eventBatchContinuesFrom(message, this.cursor)
        ) {
          this.forceSnapshot()
          return
        }
        if (message.cursor <= this.cursor) {
          return
        }
        this.cursor = message.cursor
        this.snapshotVersion = message.projection.snapshot.snapshot_version
        roomAccess.replaceGameProjection(message.projection)
        return
      }
      this.forceSnapshot()
    },
    forceSnapshot(): void {
      const game = useRoomAccessStore().game
      if (game) {
        this.connect(game, true)
      }
    },
    scheduleReconnect(generation: number): void {
      if (reconnectTimer || generation !== connectionGeneration) {
        return
      }
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null
        if (generation !== connectionGeneration) {
          return
        }
        const game = useRoomAccessStore().game
        if (game) {
          this.connect(game)
        }
      }, reconnectDelayMilliseconds)
    },
    closeSocket(): void {
      connectionGeneration += 1
      if (reconnectTimer) {
        clearTimeout(reconnectTimer)
        reconnectTimer = null
      }
      const socket = activeSocket
      activeSocket = null
      if (socket) {
        socket.onclose = null
        socket.close()
      }
    },
    disconnect(): void {
      this.closeSocket()
      activeGameId = null
      this.status = 'disconnected'
      this.cursor = 0
      this.snapshotVersion = 1
    },
  },
})
