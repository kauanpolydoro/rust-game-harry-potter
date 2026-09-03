import { defineStore } from 'pinia'

import {
  isFindRoomResponse,
  isGameProjectionResponse,
  isLobbyResponse,
  type CreateRoomResponse,
  type FindRoomResponse,
  type GameProjectionResponse,
  type HeroId,
  type JoinRoomRequest,
  type LobbyResponse,
  type StartGameRequest,
} from '../contracts/identity-access.generated'

type RoomAccessStatus =
  | 'idle'
  | 'looking_up'
  | 'joining'
  | 'restoring'
  | 'selecting_hero'
  | 'setting_readiness'
  | 'starting_game'
  | 'ready'
  | 'failed'

function lobbyActionIsPending(status: RoomAccessStatus): boolean {
  return (
    status === 'selecting_hero' ||
    status === 'setting_readiness' ||
    status === 'starting_game' ||
    status === 'restoring'
  )
}

const sessionExpectedStorage = 'hogwarts.session.expected'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function apiErrorCode(value: unknown): string {
  return isRecord(value) && isRecord(value.error) && typeof value.error.code === 'string'
    ? value.error.code
    : 'UNEXPECTED_RESPONSE'
}

function markSessionExpected(expected: boolean): void {
  try {
    if (expected) {
      localStorage.setItem(sessionExpectedStorage, 'true')
    } else {
      localStorage.removeItem(sessionExpectedStorage)
    }
  } catch {
    // The HttpOnly cookie remains authoritative when storage is unavailable.
  }
}

function sessionIsExpected(): boolean {
  try {
    return localStorage.getItem(sessionExpectedStorage) === 'true'
  } catch {
    return false
  }
}

export const useRoomAccessStore = defineStore('roomAccess', {
  state: (): {
    status: RoomAccessStatus
    roomLookup: FindRoomResponse | null
    lobby: LobbyResponse | null
    game: GameProjectionResponse | null
    errorCode: string | null
    idempotencyKey: string | null
    pendingInput: JoinRoomRequest | null
    startIdempotencyKey: string | null
    pendingStartInput: StartGameRequest | null
    sessionExpected: boolean
  } => ({
    status: 'idle',
    roomLookup: null,
    lobby: null,
    game: null,
    errorCode: null,
    idempotencyKey: null,
    pendingInput: null,
    startIdempotencyKey: null,
    pendingStartInput: null,
    sessionExpected: sessionIsExpected(),
  }),
  actions: {
    adoptCreatedRoom(room: CreateRoomResponse): void {
      this.lobby = room
      this.game = null
      this.status = 'ready'
      this.errorCode = null
      this.sessionExpected = true
      markSessionExpected(true)
    },
    clearLookup(): void {
      if (this.status === 'joining') {
        return
      }
      this.roomLookup = null
      this.errorCode = null
      this.idempotencyKey = null
      this.pendingInput = null
      this.status = 'idle'
    },
    async findRoom(roomCode: string): Promise<void> {
      if (this.status === 'looking_up' || this.status === 'joining') {
        return
      }
      const normalizedCode = roomCode.trim().toUpperCase()
      this.status = 'looking_up'
      this.errorCode = null

      try {
        const response = await fetch(`/api/rooms/${encodeURIComponent(normalizedCode)}`, {
          cache: 'no-store',
          credentials: 'same-origin',
          headers: { Accept: 'application/json' },
        })
        const result: unknown = await response.json()
        if (response.ok && isFindRoomResponse(result)) {
          this.roomLookup = result
          this.status = 'idle'
          return
        }

        this.roomLookup = null
        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
      } catch {
        this.roomLookup = null
        this.errorCode = 'NETWORK_UNAVAILABLE'
        this.status = 'failed'
      }
    },
    async joinRoom(input: JoinRoomRequest): Promise<void> {
      if (!this.roomLookup || this.status === 'joining') {
        return
      }

      this.status = 'joining'
      this.errorCode = null
      this.idempotencyKey ??= crypto.randomUUID()
      this.pendingInput ??= { ...input }

      try {
        const response = await fetch(
          `/api/rooms/${encodeURIComponent(this.roomLookup.room.code)}/participants`,
          {
            body: JSON.stringify(this.pendingInput),
            cache: 'no-store',
            credentials: 'same-origin',
            headers: {
              Accept: 'application/json',
              'Content-Type': 'application/json',
              'Idempotency-Key': this.idempotencyKey,
            },
            method: 'POST',
          },
        )
        const result: unknown = await response.json()
        if (response.ok && isLobbyResponse(result)) {
          this.lobby = result
          this.roomLookup = null
          this.status = 'ready'
          this.idempotencyKey = null
          this.pendingInput = null
          this.sessionExpected = true
          markSessionExpected(true)
          return
        }

        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
        if (this.errorCode === 'HERO_UNAVAILABLE') {
          const selectedHero = this.pendingInput.hero_id
          this.roomLookup.heroes = this.roomLookup.heroes.map((hero) =>
            hero.id === selectedHero ? { ...hero, available: false } : hero,
          )
          this.idempotencyKey = null
          this.pendingInput = null
        } else if (this.errorCode !== 'NETWORK_UNAVAILABLE') {
          this.idempotencyKey = null
          this.pendingInput = null
        }
      } catch {
        this.errorCode = 'NETWORK_UNAVAILABLE'
        this.status = 'failed'
      }
    },
    async restoreSession(): Promise<void> {
      if (!this.sessionExpected || this.lobby || this.game || this.status === 'restoring') {
        return
      }

      this.status = 'restoring'
      this.errorCode = null
      try {
        const response = await fetch('/api/session', {
          cache: 'no-store',
          credentials: 'same-origin',
          headers: { Accept: 'application/json' },
        })
        const result: unknown = await response.json()
        if (response.ok && isLobbyResponse(result)) {
          this.lobby = result
          this.game = null
          this.status = 'ready'
          return
        }
        if (response.ok && isGameProjectionResponse(result)) {
          this.game = result
          this.lobby = null
          this.status = 'ready'
          return
        }

        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
        if (response.status === 401) {
          this.sessionExpected = false
          markSessionExpected(false)
        }
      } catch {
        this.errorCode = 'NETWORK_UNAVAILABLE'
        this.status = 'failed'
      }
    },
    async selectHero(heroId: HeroId): Promise<void> {
      if (!this.lobby || lobbyActionIsPending(this.status)) {
        return
      }

      this.status = 'selecting_hero'
      this.errorCode = null
      try {
        const response = await fetch('/api/session/hero', {
          body: JSON.stringify({ hero_id: heroId }),
          cache: 'no-store',
          credentials: 'same-origin',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
          },
          method: 'PUT',
        })
        const result: unknown = await response.json()
        if (response.ok && isLobbyResponse(result)) {
          this.lobby = result
          this.status = 'ready'
          return
        }

        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
      } catch {
        this.errorCode = 'NETWORK_UNAVAILABLE'
        this.status = 'failed'
      }
    },
    async setReadiness(ready: boolean): Promise<void> {
      if (!this.lobby || lobbyActionIsPending(this.status)) {
        return
      }

      this.status = 'setting_readiness'
      this.errorCode = null
      try {
        const response = await fetch('/api/session/readiness', {
          body: JSON.stringify({ ready }),
          cache: 'no-store',
          credentials: 'same-origin',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
          },
          method: 'PUT',
        })
        const result: unknown = await response.json()
        if (response.ok && isLobbyResponse(result)) {
          this.lobby = result
          this.status = 'ready'
          return
        }

        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
      } catch {
        this.errorCode = 'NETWORK_UNAVAILABLE'
        this.status = 'failed'
      }
    },
    async startGame(input: StartGameRequest): Promise<void> {
      if (!this.lobby || lobbyActionIsPending(this.status)) {
        return
      }

      this.status = 'starting_game'
      this.errorCode = null
      this.startIdempotencyKey ??= crypto.randomUUID()
      this.pendingStartInput ??= { ...input }
      try {
        const response = await fetch('/api/games', {
          body: JSON.stringify(this.pendingStartInput),
          cache: 'no-store',
          credentials: 'same-origin',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
            'Idempotency-Key': this.startIdempotencyKey,
          },
          method: 'POST',
        })
        const result: unknown = await response.json()
        if (response.ok && isGameProjectionResponse(result)) {
          this.game = result
          this.lobby = null
          this.startIdempotencyKey = null
          this.pendingStartInput = null
          this.status = 'ready'
          return
        }

        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
        if (
          this.errorCode !== 'NETWORK_UNAVAILABLE' &&
          this.errorCode !== 'INTERNAL_ERROR' &&
          this.errorCode !== 'UNEXPECTED_RESPONSE'
        ) {
          this.startIdempotencyKey = null
          this.pendingStartInput = null
        }
      } catch {
        this.errorCode = 'NETWORK_UNAVAILABLE'
        this.status = 'failed'
      }
    },
    async refreshSession(): Promise<void> {
      if (!this.sessionExpected || lobbyActionIsPending(this.status)) {
        return
      }

      this.status = 'restoring'
      this.errorCode = null
      try {
        const response = await fetch('/api/session', {
          cache: 'no-store',
          credentials: 'same-origin',
          headers: { Accept: 'application/json' },
        })
        const result: unknown = await response.json()
        if (response.ok && isLobbyResponse(result)) {
          this.lobby = result
          this.game = null
          this.status = 'ready'
          return
        }
        if (response.ok && isGameProjectionResponse(result)) {
          this.game = result
          this.lobby = null
          this.status = 'ready'
          return
        }

        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
      } catch {
        this.errorCode = 'NETWORK_UNAVAILABLE'
        this.status = 'failed'
      }
    },
  },
})
