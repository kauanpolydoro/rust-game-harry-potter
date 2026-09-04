import { defineStore } from 'pinia'

import {
  apiError,
  isUncertainTransportFailure,
  requestJson,
  transportErrorCode,
} from '../api/http'
import {
  isFindRoomResponse,
  isGameProjectionResponse,
  isJoinRoomResponse,
  isLobbyResponse,
  isRecoveredGameResponse,
  isRecoveredLobbyResponse,
  isRecoveryReplacementRequiredResponse,
  type CreateRoomResponse,
  type FindRoomResponse,
  type GameProjectionResponse,
  type HeroId,
  type JoinRoomRequest,
  type LobbyResponse,
  type RecoverParticipationRequest,
  type RecoverySessionSummary,
  type StartGameRequest,
} from '../contracts/identity-access.generated'

type RoomAccessStatus =
  | 'idle'
  | 'looking_up'
  | 'joining'
  | 'recovering_participation'
  | 'choosing_recovery_session'
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
    status === 'recovering_participation' ||
    status === 'restoring'
  )
}

const sessionExpectedStorage = 'hogwarts.session.expected'
const pendingJoinStorage = 'hogwarts.room-join.pending-intent'
const recoveryAttemptStorage = 'hogwarts.participant-recovery.attempt-id'
const roomCodePattern = /^[23456789A-HJ-NP-Z]{8}$/
const uuidV4Pattern = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
const heroIds = new Set<HeroId>(['harry', 'hermione', 'neville', 'ron'])

interface PendingJoinIntent {
  commandType: 'join_room'
  createdAt: string
  idempotencyKey: string
  input: JoinRoomRequest
  roomCode: string
}

type ParticipationRecoveryInput = Omit<RecoverParticipationRequest, 'recovery_attempt_id'>

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function loadPendingJoinIntent(): PendingJoinIntent | null {
  try {
    const serialized = sessionStorage.getItem(pendingJoinStorage)
    if (!serialized) {
      return null
    }
    const intent: unknown = JSON.parse(serialized)
    const input = isRecord(intent) ? intent.input : null
    if (
      !isRecord(intent) ||
      Object.keys(intent).length !== 5 ||
      intent.commandType !== 'join_room' ||
      typeof intent.createdAt !== 'string' ||
      Number.isNaN(Date.parse(intent.createdAt)) ||
      typeof intent.idempotencyKey !== 'string' ||
      !uuidV4Pattern.test(intent.idempotencyKey) ||
      typeof intent.roomCode !== 'string' ||
      !roomCodePattern.test(intent.roomCode) ||
      !isRecord(input) ||
      Object.keys(input).length !== 2 ||
      typeof input.display_name !== 'string' ||
      typeof input.hero_id !== 'string' ||
      !heroIds.has(input.hero_id as HeroId)
    ) {
      sessionStorage.removeItem(pendingJoinStorage)
      return null
    }
    return {
      commandType: 'join_room',
      createdAt: intent.createdAt,
      idempotencyKey: intent.idempotencyKey,
      input: {
        display_name: input.display_name,
        hero_id: input.hero_id as HeroId,
      },
      roomCode: intent.roomCode,
    }
  } catch {
    try {
      sessionStorage.removeItem(pendingJoinStorage)
    } catch {
      // Unavailable storage must not prevent a fresh join attempt.
    }
    return null
  }
}

function persistPendingJoinIntent(intent: PendingJoinIntent): void {
  try {
    sessionStorage.setItem(pendingJoinStorage, JSON.stringify(intent))
  } catch {
    // Server-side idempotency still protects retries while this store remains alive.
  }
}

function removePendingJoinIntent(): void {
  try {
    sessionStorage.removeItem(pendingJoinStorage)
  } catch {
    // Storage availability must not prevent a definitive response from being handled.
  }
}

function loadRecoveryAttemptId(): string | null {
  try {
    const attemptId = sessionStorage.getItem(recoveryAttemptStorage)
    if (attemptId && uuidV4Pattern.test(attemptId)) {
      return attemptId
    }
    sessionStorage.removeItem(recoveryAttemptStorage)
  } catch {
    // In-memory idempotency remains available when session storage is unavailable.
  }
  return null
}

function persistRecoveryAttemptId(attemptId: string): void {
  try {
    sessionStorage.setItem(recoveryAttemptStorage, attemptId)
  } catch {
    // The active page can still repeat the same recovery attempt from memory.
  }
}

function removeRecoveryAttemptId(): void {
  try {
    sessionStorage.removeItem(recoveryAttemptStorage)
  } catch {
    // Storage availability must not prevent a confirmed recovery from completing.
  }
}

function apiErrorCode(value: unknown): string {
  return apiError(value)?.code ?? 'UNEXPECTED_RESPONSE'
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
    pendingJoinIntent: PendingJoinIntent | null
    startIdempotencyKey: string | null
    pendingStartInput: StartGameRequest | null
    sessionExpected: boolean
    issuedRecoveryToken: string | null
    recoveryAttemptId: string | null
    replacementSessions: Array<RecoverySessionSummary>
  } => {
    const pendingJoinIntent = loadPendingJoinIntent()
    return {
      status: 'idle',
      roomLookup: null,
      lobby: null,
      game: null,
      errorCode: null,
      idempotencyKey: pendingJoinIntent?.idempotencyKey ?? null,
      pendingInput: pendingJoinIntent ? { ...pendingJoinIntent.input } : null,
      pendingJoinIntent,
      startIdempotencyKey: null,
      pendingStartInput: null,
      sessionExpected: sessionIsExpected(),
      issuedRecoveryToken: null,
      recoveryAttemptId: loadRecoveryAttemptId(),
      replacementSessions: [],
    }
  },
  actions: {
    clearPendingJoinIntent(): void {
      this.idempotencyKey = null
      this.pendingInput = null
      this.pendingJoinIntent = null
      removePendingJoinIntent()
    },
    replaceGameProjection(projection: GameProjectionResponse): void {
      if (this.game && this.game.game.id !== projection.game.id) {
        return
      }
      this.game = projection
      this.lobby = null
      this.status = 'ready'
      this.errorCode = null
    },
    adoptCreatedRoom(room: CreateRoomResponse): void {
      const { recovery_token: recoveryToken, ...lobby } = room
      this.lobby = lobby
      this.game = null
      this.status = 'ready'
      this.errorCode = null
      this.sessionExpected = true
      this.issuedRecoveryToken = recoveryToken
      markSessionExpected(true)
    },
    dismissRecoveryCredential(): void {
      this.issuedRecoveryToken = null
    },
    dismissParticipationRecovery(): void {
      if (this.status === 'recovering_participation') {
        return
      }
      this.errorCode = null
      this.replacementSessions = []
      this.status = 'idle'
      this.clearParticipationRecoveryAttempt()
    },
    clearParticipationRecoveryAttempt(): void {
      this.recoveryAttemptId = null
      removeRecoveryAttemptId()
    },
    clearLookup(): void {
      if (this.status === 'joining') {
        return
      }
      this.roomLookup = null
      this.errorCode = null
      this.clearPendingJoinIntent()
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
        const { body: result, response } = await requestJson(`/api/rooms/${encodeURIComponent(normalizedCode)}`, {
          cache: 'no-store',
          credentials: 'same-origin',
          headers: { Accept: 'application/json' },
        })
        if (response.ok && isFindRoomResponse(result)) {
          this.roomLookup = result
          this.status = 'idle'
          return
        }

        this.roomLookup = null
        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
      } catch (error) {
        this.roomLookup = null
        this.errorCode = transportErrorCode(error)
        this.status = 'failed'
      }
    },
    async joinRoom(input: JoinRoomRequest): Promise<void> {
      if (!this.roomLookup || this.status === 'joining') {
        return
      }

      this.idempotencyKey ??= crypto.randomUUID()
      this.pendingInput ??= { ...input }
      this.pendingJoinIntent ??= {
        commandType: 'join_room',
        createdAt: new Date().toISOString(),
        idempotencyKey: this.idempotencyKey,
        input: { ...this.pendingInput },
        roomCode: this.roomLookup.room.code,
      }
      persistPendingJoinIntent(this.pendingJoinIntent)
      await this.submitPendingJoin()
    },
    async recoverPendingJoin(): Promise<void> {
      if (!this.pendingJoinIntent || this.lobby || this.game || this.status === 'joining') {
        return
      }
      await this.submitPendingJoin()
    },
    async submitPendingJoin(): Promise<void> {
      const intent = this.pendingJoinIntent
      if (!intent) {
        return
      }

      this.status = 'joining'
      this.errorCode = null

      try {
        const { body: result, response } = await requestJson(
          `/api/rooms/${encodeURIComponent(intent.roomCode)}/participants`,
          {
            body: JSON.stringify(intent.input),
            cache: 'no-store',
            credentials: 'same-origin',
            headers: {
              Accept: 'application/json',
              'Content-Type': 'application/json',
              'Idempotency-Key': intent.idempotencyKey,
            },
            method: 'POST',
          },
        )
        if (response.ok && isJoinRoomResponse(result)) {
          const { recovery_token: recoveryToken, ...lobby } = result
          this.lobby = lobby
          this.roomLookup = null
          this.status = 'ready'
          this.clearPendingJoinIntent()
          this.sessionExpected = true
          this.issuedRecoveryToken = recoveryToken
          markSessionExpected(true)
          return
        }

        const error = apiError(result)
        this.errorCode = error?.code ?? 'UNEXPECTED_RESPONSE'
        this.status = 'failed'
        if (this.errorCode === 'HERO_UNAVAILABLE') {
          if (this.roomLookup) {
            const selectedHero = intent.input.hero_id
            this.roomLookup.heroes = this.roomLookup.heroes.map((hero) =>
              hero.id === selectedHero ? { ...hero, available: false } : hero,
            )
          }
          this.clearPendingJoinIntent()
        } else if (!isUncertainTransportFailure(this.errorCode) && error?.retry !== 'safe_to_retry') {
          this.clearPendingJoinIntent()
        }
      } catch (error) {
        this.errorCode = transportErrorCode(error)
        this.status = 'failed'
      }
    },
    async recoverParticipation(input: ParticipationRecoveryInput): Promise<boolean> {
      if (this.status === 'recovering_participation') {
        return false
      }

      this.status = 'recovering_participation'
      this.errorCode = null
      const recoveryAttemptId = this.recoveryAttemptId ?? crypto.randomUUID()
      this.recoveryAttemptId = recoveryAttemptId
      persistRecoveryAttemptId(recoveryAttemptId)
      try {
        const { body: result, response } = await requestJson('/api/session/recover', {
          body: JSON.stringify({
            ...input,
            recovery_attempt_id: recoveryAttemptId,
          } satisfies RecoverParticipationRequest),
          cache: 'no-store',
          credentials: 'same-origin',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
          },
          method: 'POST',
        })
        if (response.status === 409 && isRecoveryReplacementRequiredResponse(result)) {
          this.replacementSessions = result.sessions
          this.status = 'choosing_recovery_session'
          return false
        }
        if (
          response.ok &&
          (isRecoveredLobbyResponse(result) || isRecoveredGameResponse(result))
        ) {
          if (result.kind === 'lobby') {
            this.lobby = result.lobby
            this.game = null
          } else {
            this.game = result.game
            this.lobby = null
          }
          this.status = 'ready'
          this.issuedRecoveryToken = result.recovery_token
          this.replacementSessions = []
          this.clearPendingJoinIntent()
          this.clearParticipationRecoveryAttempt()
          this.sessionExpected = true
          markSessionExpected(true)
          return true
        }

        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
        return false
      } catch (error) {
        const recoveryError = transportErrorCode(error)
        this.errorCode = recoveryError
        this.status = 'failed'
        this.sessionExpected = false
        markSessionExpected(false)
        return false
      }
    },
    async restoreSession(): Promise<void> {
      if (
        (!this.sessionExpected && !this.pendingJoinIntent) ||
        this.lobby ||
        this.game ||
        this.status === 'restoring'
      ) {
        return
      }

      this.status = 'restoring'
      this.errorCode = null
      try {
        const { body: result, response } = await requestJson('/api/session', {
          cache: 'no-store',
          credentials: 'same-origin',
          headers: { Accept: 'application/json' },
        })
        if (response.ok && isLobbyResponse(result)) {
          this.lobby = result
          this.game = null
          this.status = 'ready'
          this.clearPendingJoinIntent()
          this.sessionExpected = true
          markSessionExpected(true)
          return
        }
        if (response.ok && isGameProjectionResponse(result)) {
          this.game = result
          this.lobby = null
          this.status = 'ready'
          this.clearPendingJoinIntent()
          this.sessionExpected = true
          markSessionExpected(true)
          return
        }

        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
        if (response.status === 401) {
          this.sessionExpected = false
          markSessionExpected(false)
        }
      } catch (error) {
        this.errorCode = transportErrorCode(error)
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
        const { body: result, response } = await requestJson('/api/session/hero', {
          body: JSON.stringify({ hero_id: heroId }),
          cache: 'no-store',
          credentials: 'same-origin',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
          },
          method: 'PUT',
        })
        if (response.ok && isLobbyResponse(result)) {
          this.lobby = result
          this.status = 'ready'
          return
        }

        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
      } catch (error) {
        this.errorCode = transportErrorCode(error)
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
        const { body: result, response } = await requestJson('/api/session/readiness', {
          body: JSON.stringify({ ready }),
          cache: 'no-store',
          credentials: 'same-origin',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
          },
          method: 'PUT',
        })
        if (response.ok && isLobbyResponse(result)) {
          this.lobby = result
          this.status = 'ready'
          return
        }

        this.errorCode = apiErrorCode(result)
        this.status = 'failed'
      } catch (error) {
        this.errorCode = transportErrorCode(error)
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
        const { body: result, response } = await requestJson('/api/games', {
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
        if (!isUncertainTransportFailure(this.errorCode) && this.errorCode !== 'INTERNAL_ERROR') {
          this.startIdempotencyKey = null
          this.pendingStartInput = null
        }
      } catch (error) {
        this.errorCode = transportErrorCode(error)
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
        const { body: result, response } = await requestJson('/api/session', {
          cache: 'no-store',
          credentials: 'same-origin',
          headers: { Accept: 'application/json' },
        })
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
      } catch (error) {
        this.errorCode = transportErrorCode(error)
        this.status = 'failed'
      }
    },
  },
})
