import { defineStore } from 'pinia'

import {
  openRoomStatuses,
  participantRoles,
  type CreateRoomRequest,
  type CreateRoomResponse,
} from '../contracts/identity-access.generated'

export type RoomCreationStatus = 'idle' | 'submitting' | 'succeeded' | 'failed'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isCreateRoomResponse(value: unknown): value is CreateRoomResponse {
  if (!isRecord(value) || !isRecord(value.room) || !isRecord(value.participant)) {
    return false
  }
  const room = value.room
  const participant = value.participant

  return (
    typeof room.code === 'string' &&
    openRoomStatuses.some((status) => status === room.status) &&
    typeof participant.display_name === 'string' &&
    participantRoles.some((role) => role === participant.role)
  )
}

function apiError(value: unknown): { code: string; retry: string } | null {
  if (
    !isRecord(value) ||
    !isRecord(value.error) ||
    typeof value.error.code !== 'string' ||
    typeof value.error.retry !== 'string'
  ) {
    return null
  }

  return { code: value.error.code, retry: value.error.retry }
}

export const useRoomCreationStore = defineStore('roomCreation', {
  state: (): {
    status: RoomCreationStatus
    roomCreation: CreateRoomResponse | null
    errorCode: string | null
    idempotencyKey: string | null
  } => ({
    status: 'idle',
    roomCreation: null,
    errorCode: null,
    idempotencyKey: null,
  }),
  actions: {
    resetPendingRequest(): void {
      if (this.status !== 'submitting') {
        this.idempotencyKey = null
        this.errorCode = null
        this.status = 'idle'
      }
    },
    async createRoom(input: CreateRoomRequest): Promise<void> {
      if (this.status === 'submitting') {
        return
      }

      this.status = 'submitting'
      this.errorCode = null
      this.idempotencyKey ??= crypto.randomUUID()

      try {
        const response = await fetch('/api/rooms', {
          body: JSON.stringify(input),
          cache: 'no-store',
          credentials: 'same-origin',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
            'Idempotency-Key': this.idempotencyKey,
          },
          method: 'POST',
        })
        const result: unknown = await response.json()

        if (response.ok && isCreateRoomResponse(result)) {
          this.roomCreation = result
          this.status = 'succeeded'
          return
        }

        const error = apiError(result)
        this.errorCode = error?.code ?? 'UNEXPECTED_RESPONSE'
        this.status = 'failed'
        if (error?.retry !== 'safe_to_retry') {
          this.idempotencyKey = null
        }
      } catch {
        this.errorCode = 'NETWORK_UNAVAILABLE'
        this.status = 'failed'
      }
    },
  },
})
