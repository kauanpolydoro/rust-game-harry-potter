import { defineStore } from 'pinia'

import {
  apiError,
  isUncertainTransportFailure,
  requestJson,
  transportErrorCode,
} from '../api/http'
import {
  isAssistedRecoveryCredentialResponse,
  isDirectRecoveryCredentialResponse,
  isRotateRecoveryPasswordResponse,
  type AssistedRecoveryCredentialResponse,
  type DirectRecoveryCredentialResponse,
  type RegenerateAssistedRecoveryCredentialRequest,
  type RegenerateOwnRecoveryCredentialRequest,
  type RotateRecoveryPasswordRequest,
  type RotateRecoveryPasswordResponse,
} from '../contracts/identity-access.generated'
import { useSecuritySyncStore } from './securitySync'

type RecoveryManagementStatus =
  | 'idle'
  | 'rotating_password'
  | 'regenerating_directly'
  | 'regenerating_with_assistance'
  | 'failed'

type IssuedRecoveryCredential =
  | DirectRecoveryCredentialResponse
  | AssistedRecoveryCredentialResponse

interface PendingRecoveryOperation {
  idempotencyKey: string
  kind: 'rotate_password' | 'regenerate_directly' | 'regenerate_with_assistance'
  targetPosition?: number
}

function pendingOperationMatches(
  pending: PendingRecoveryOperation | null,
  kind: PendingRecoveryOperation['kind'],
  targetPosition?: number,
): boolean {
  return (
    pending?.kind === kind &&
    (kind !== 'regenerate_with_assistance' || pending.targetPosition === targetPosition)
  )
}

function passwordRotationIsCoherent(response: RotateRecoveryPasswordResponse): boolean {
  return response.password_generation === response.security_event.password_generation
}

function directCredentialIsCoherent(response: DirectRecoveryCredentialResponse): boolean {
  const event = response.security_event
  return (
    event.delivery === 'direct' &&
    event.actor_position === event.target_position &&
    event.target_position === response.participant.position &&
    event.recovery_generation === response.recovery_generation
  )
}

function assistedCredentialIsCoherent(response: AssistedRecoveryCredentialResponse): boolean {
  const event = response.security_event
  return (
    event.delivery === 'host_assisted' &&
    event.actor_position !== event.target_position &&
    event.target_position === response.participant.position &&
    event.recovery_generation === response.recovery_generation
  )
}

export const useRecoveryManagementStore = defineStore('recoveryManagement', {
  state: (): {
    status: RecoveryManagementStatus
    errorCode: string | null
    confirmation: 'password_rotated' | 'credential_regenerated' | null
    issuedCredential: IssuedRecoveryCredential | null
    pendingOperation: PendingRecoveryOperation | null
  } => ({
    status: 'idle',
    errorCode: null,
    confirmation: null,
    issuedCredential: null,
    pendingOperation: null,
  }),
  actions: {
    beginOperation(
      kind: PendingRecoveryOperation['kind'],
      targetPosition?: number,
    ): PendingRecoveryOperation {
      const existingOperation = this.pendingOperation
      if (
        existingOperation &&
        pendingOperationMatches(existingOperation, kind, targetPosition)
      ) {
        return existingOperation
      }
      const pendingOperation: PendingRecoveryOperation = {
        idempotencyKey: crypto.randomUUID(),
        kind,
        ...(targetPosition === undefined ? {} : { targetPosition }),
      }
      this.pendingOperation = pendingOperation
      this.errorCode = null
      this.confirmation = null
      return pendingOperation
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
        this.status === 'rotating_password' ||
        this.status === 'regenerating_directly' ||
        this.status === 'regenerating_with_assistance'
      ) {
        return
      }
      this.pendingOperation = null
      this.errorCode = null
      this.confirmation = null
    },
    dismissIssuedCredential(): void {
      this.issuedCredential = null
    },
    async rotatePassword(input: RotateRecoveryPasswordRequest): Promise<boolean> {
      if (this.status === 'rotating_password') {
        return false
      }
      const pending = this.beginOperation('rotate_password')
      this.status = 'rotating_password'
      try {
        const { body, response } = await requestJson('/api/session/recovery-password', {
          body: JSON.stringify(input),
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
            'Idempotency-Key': pending.idempotencyKey,
          },
          method: 'PUT',
        })
        if (
          response.ok &&
          isRotateRecoveryPasswordResponse(body) &&
          passwordRotationIsCoherent(body)
        ) {
          this.status = 'idle'
          this.errorCode = null
          this.confirmation = 'password_rotated'
          this.issuedCredential = null
          this.pendingOperation = null
          return true
        }
        this.finishFailure(body)
      } catch (error) {
        this.finishTransportFailure(error)
      }
      return false
    },
    async regenerateOwnCredential(): Promise<boolean> {
      if (this.status === 'regenerating_directly') {
        return false
      }
      const pending = this.beginOperation('regenerate_directly')
      this.status = 'regenerating_directly'
      const input = {} satisfies RegenerateOwnRecoveryCredentialRequest
      try {
        const { body, response } = await requestJson('/api/session/recovery-credential', {
          body: JSON.stringify(input),
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
            'Idempotency-Key': pending.idempotencyKey,
          },
          method: 'POST',
        })
        if (
          response.ok &&
          isDirectRecoveryCredentialResponse(body) &&
          directCredentialIsCoherent(body)
        ) {
          if (
            useSecuritySyncStore().credentialWasSuperseded(
              body.participant.position,
              body.security_event.cursor,
            )
          ) {
            this.status = 'failed'
            this.errorCode = 'RECOVERY_CREDENTIAL_SUPERSEDED'
            this.issuedCredential = null
            this.pendingOperation = null
            return false
          }
          this.status = 'idle'
          this.errorCode = null
          this.confirmation = 'credential_regenerated'
          this.issuedCredential = body
          this.pendingOperation = null
          return true
        }
        this.finishFailure(body)
      } catch (error) {
        this.finishTransportFailure(error)
      }
      return false
    },
    async regenerateAssistedCredential(targetPosition: number): Promise<boolean> {
      if (this.status === 'regenerating_with_assistance') {
        return false
      }
      const pending = this.beginOperation('regenerate_with_assistance', targetPosition)
      this.status = 'regenerating_with_assistance'
      const input = {
        host_assistance_risk_acknowledged: true,
      } satisfies RegenerateAssistedRecoveryCredentialRequest
      try {
        const { body, response } = await requestJson(
          `/api/rooms/current/participants/${encodeURIComponent(targetPosition)}/recovery-credential`,
          {
            body: JSON.stringify(input),
            headers: {
              Accept: 'application/json',
              'Content-Type': 'application/json',
              'Idempotency-Key': pending.idempotencyKey,
            },
            method: 'POST',
          },
        )
        if (
          response.ok &&
          isAssistedRecoveryCredentialResponse(body) &&
          assistedCredentialIsCoherent(body)
        ) {
          if (
            useSecuritySyncStore().credentialWasSuperseded(
              body.participant.position,
              body.security_event.cursor,
            )
          ) {
            this.status = 'failed'
            this.errorCode = 'RECOVERY_CREDENTIAL_SUPERSEDED'
            this.issuedCredential = null
            this.pendingOperation = null
            return false
          }
          this.status = 'idle'
          this.errorCode = null
          this.confirmation = 'credential_regenerated'
          this.issuedCredential = body
          this.pendingOperation = null
          return true
        }
        this.finishFailure(body)
      } catch (error) {
        this.finishTransportFailure(error)
      }
      return false
    },
    retryIsUncertain(): boolean {
      return isUncertainTransportFailure(this.errorCode)
    },
  },
})
