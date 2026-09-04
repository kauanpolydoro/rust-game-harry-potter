import { defineStore } from 'pinia'

import {
  apiError,
  requestJson,
  transportErrorCode,
} from '../api/http'
import {
  isDeviceSessionsResponse,
  isProtectParticipantResponse,
  isProtectRoomResponse,
  isRevokeDeviceSessionResponse,
  type DeviceSessionSummary,
  type ProtectParticipantRequest,
  type ProtectRoomRequest,
  type ProtectRoomResponse,
  type RevokeDeviceSessionRequest,
} from '../contracts/identity-access.generated'
import { useRoomAccessStore } from './roomAccess'

type AccessProtectionStatus =
  | 'idle'
  | 'loading_sessions'
  | 'revoking_session'
  | 'protecting_participant'
  | 'protecting_room'
  | 'failed'

type AccessProtectionConfirmation =
  | 'session_revoked'
  | 'participant_protected'
  | 'room_protected'

export type AccessProtectionOutcome = 'session_retained' | 'session_revoked'

interface PendingAccessOperation {
  idempotencyKey: string
  kind: AccessProtectionConfirmation
  targetSessionId?: string
}

function operationMatches(
  operation: PendingAccessOperation | null,
  kind: AccessProtectionConfirmation,
  targetSessionId?: string,
): boolean {
  return operation?.kind === kind && operation.targetSessionId === targetSessionId
}

function roomProtectionIsCoherent(response: ProtectRoomResponse): boolean {
  const event = response.security_event
  return (
    response.password_generation === event.password_generation &&
    response.recovery_epoch === event.recovery_epoch &&
    response.revoked_sessions === event.revoked_sessions &&
    response.current_session_preserved === event.current_session_preserved
  )
}

export const useAccessProtectionStore = defineStore('accessProtection', {
  state: (): {
    status: AccessProtectionStatus
    sessions: DeviceSessionSummary[]
    errorCode: string | null
    confirmation: AccessProtectionConfirmation | null
    pendingOperation: PendingAccessOperation | null
  } => ({
    status: 'idle',
    sessions: [],
    errorCode: null,
    confirmation: null,
    pendingOperation: null,
  }),
  actions: {
    beginOperation(
      kind: AccessProtectionConfirmation,
      targetSessionId?: string,
    ): PendingAccessOperation {
      if (operationMatches(this.pendingOperation, kind, targetSessionId)) {
        return this.pendingOperation as PendingAccessOperation
      }
      const operation: PendingAccessOperation = {
        idempotencyKey: crypto.randomUUID(),
        kind,
        ...(targetSessionId ? { targetSessionId } : {}),
      }
      this.pendingOperation = operation
      this.errorCode = null
      this.confirmation = null
      return operation
    },
    finishFailure(body: unknown): void {
      const error = apiError(body)
      this.errorCode = error?.code ?? 'UNEXPECTED_RESPONSE'
      this.status = 'failed'
      if (!error || error.retry !== 'safe_to_retry') {
        this.pendingOperation = null
      }
    },
    finishTransportFailure(error: unknown): void {
      this.errorCode = transportErrorCode(error)
      this.status = 'failed'
    },
    resetPendingOperation(): void {
      if (
        this.status === 'revoking_session' ||
        this.status === 'protecting_participant' ||
        this.status === 'protecting_room'
      ) {
        return
      }
      this.pendingOperation = null
      this.errorCode = null
      this.confirmation = null
    },
    clearPrivateState(): void {
      this.$reset()
    },
    async loadSessions(): Promise<boolean> {
      if (this.status === 'loading_sessions') {
        return false
      }
      this.status = 'loading_sessions'
      this.errorCode = null
      const roomAccess = useRoomAccessStore()
      const sessionGeneration = roomAccess.sessionGeneration
      try {
        const { body, response } = await requestJson('/api/session/device-sessions', {
          cache: 'no-store',
          credentials: 'same-origin',
          headers: { Accept: 'application/json' },
        })
        if (roomAccess.sessionGeneration !== sessionGeneration) {
          return false
        }
        if (response.ok && isDeviceSessionsResponse(body)) {
          this.sessions = body.sessions
          this.status = 'idle'
          return true
        }
        this.finishFailure(body)
      } catch (error) {
        if (roomAccess.sessionGeneration !== sessionGeneration) {
          return false
        }
        this.finishTransportFailure(error)
      }
      return false
    },
    async revokeSession(sessionId: string): Promise<AccessProtectionOutcome | null> {
      if (this.status === 'revoking_session') {
        return null
      }
      const session = this.sessions.find((candidate) => candidate.id === sessionId)
      if (!session) {
        this.errorCode = 'DEVICE_SESSION_NOT_FOUND'
        this.status = 'failed'
        return null
      }
      const operation = this.beginOperation('session_revoked', sessionId)
      const roomAccess = useRoomAccessStore()
      const sessionGeneration = roomAccess.sessionGeneration
      this.status = 'revoking_session'
      const input = {} satisfies RevokeDeviceSessionRequest
      try {
        const { body, response } = await requestJson(
          `/api/session/device-sessions/${encodeURIComponent(sessionId)}/revocation`,
          {
            body: JSON.stringify(input),
            cache: 'no-store',
            credentials: 'same-origin',
            headers: {
              Accept: 'application/json',
              'Content-Type': 'application/json',
              'Idempotency-Key': operation.idempotencyKey,
            },
            method: 'PUT',
          },
        )
        if (roomAccess.sessionGeneration !== sessionGeneration) {
          return null
        }
        if (
          response.ok &&
          isRevokeDeviceSessionResponse(body) &&
          body.revoked_session.id === session.id &&
          body.revoked_session.label === session.label &&
          body.security_event.session_label === session.label
        ) {
          this.sessions = this.sessions.filter((candidate) => candidate.id !== sessionId)
          this.status = 'idle'
          this.errorCode = null
          this.confirmation = 'session_revoked'
          this.pendingOperation = null
          return session.current ? 'session_revoked' : 'session_retained'
        }
        this.finishFailure(body)
      } catch (error) {
        if (roomAccess.sessionGeneration !== sessionGeneration) {
          return null
        }
        this.finishTransportFailure(error)
      }
      return null
    },
    async protectParticipant(): Promise<AccessProtectionOutcome | null> {
      if (this.status === 'protecting_participant') {
        return null
      }
      const operation = this.beginOperation('participant_protected')
      const roomAccess = useRoomAccessStore()
      const sessionGeneration = roomAccess.sessionGeneration
      this.status = 'protecting_participant'
      const input = { protection_confirmed: true } satisfies ProtectParticipantRequest
      try {
        const { body, response } = await requestJson('/api/session/protection', {
          body: JSON.stringify(input),
          cache: 'no-store',
          credentials: 'same-origin',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
            'Idempotency-Key': operation.idempotencyKey,
          },
          method: 'PUT',
        })
        if (roomAccess.sessionGeneration !== sessionGeneration) {
          return null
        }
        if (
          response.ok &&
          isProtectParticipantResponse(body) &&
          body.participant.position === body.security_event.target_position &&
          body.recovery_generation === body.security_event.recovery_generation &&
          body.revoked_sessions === body.security_event.revoked_sessions
        ) {
          this.status = 'idle'
          this.errorCode = null
          this.confirmation = 'participant_protected'
          this.pendingOperation = null
          return 'session_revoked'
        }
        this.finishFailure(body)
      } catch (error) {
        if (roomAccess.sessionGeneration !== sessionGeneration) {
          return null
        }
        this.finishTransportFailure(error)
      }
      return null
    },
    async protectRoom(input: ProtectRoomRequest): Promise<AccessProtectionOutcome | null> {
      if (this.status === 'protecting_room') {
        return null
      }
      const operation = this.beginOperation('room_protected')
      const roomAccess = useRoomAccessStore()
      const sessionGeneration = roomAccess.sessionGeneration
      this.status = 'protecting_room'
      try {
        const { body, response } = await requestJson('/api/rooms/current/protection', {
          body: JSON.stringify(input),
          cache: 'no-store',
          credentials: 'same-origin',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
            'Idempotency-Key': operation.idempotencyKey,
          },
          method: 'PUT',
        })
        if (roomAccess.sessionGeneration !== sessionGeneration) {
          return null
        }
        if (response.ok && isProtectRoomResponse(body) && roomProtectionIsCoherent(body)) {
          this.status = 'idle'
          this.errorCode = null
          this.confirmation = 'room_protected'
          this.pendingOperation = null
          return body.current_session_preserved ? 'session_retained' : 'session_revoked'
        }
        this.finishFailure(body)
      } catch (error) {
        if (roomAccess.sessionGeneration !== sessionGeneration) {
          return null
        }
        this.finishTransportFailure(error)
      }
      return null
    },
  },
})
