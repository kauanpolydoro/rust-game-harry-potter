import { defineStore } from 'pinia'

import {
  openRoomStatuses,
  participantRoles,
  type CreateRoomRequest,
  type CreateRoomResponse,
} from '../contracts/identity-access.generated'

export type RoomCreationStatus = 'idle' | 'submitting' | 'succeeded' | 'failed'

const pendingKeyStorage = 'hogwarts.room-creation.idempotency-key'
const roomCodePattern = /^[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{8}$/

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
    Object.keys(value).length === 2 &&
    Object.keys(room).length === 2 &&
    Object.keys(participant).length === 2 &&
    typeof room.code === 'string' &&
    roomCodePattern.test(room.code) &&
    openRoomStatuses.some((status) => status === room.status) &&
    typeof participant.display_name === 'string' &&
    participant.display_name.length > 0 &&
    [...participant.display_name].length <= 40 &&
    participantRoles.some((role) => role === participant.role)
  )
}

function loadPendingKey(): string | null {
  try {
    const key = sessionStorage.getItem(pendingKeyStorage)
    return key && /^[A-Za-z0-9_.:-]{8,128}$/.test(key) ? key : null
  } catch {
    return null
  }
}

function persistPendingKey(key: string): void {
  try {
    sessionStorage.setItem(pendingKeyStorage, key)
  } catch {
    // Server-side idempotency still protects retries while this store remains alive.
  }
}

function removePendingKey(): void {
  try {
    sessionStorage.removeItem(pendingKeyStorage)
  } catch {
    // Storage availability must not prevent a definitive response from being handled.
  }
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
    pendingInput: CreateRoomRequest | null
  } => ({
    status: 'idle',
    roomCreation: null,
    errorCode: null,
    idempotencyKey: null,
    pendingInput: null,
  }),
  actions: {
    resetPendingRequest(): void {
      if (this.status !== 'submitting') {
        if (this.idempotencyKey !== null) {
          removePendingKey()
        }
        this.idempotencyKey = null
        this.pendingInput = null
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
      this.idempotencyKey ??= loadPendingKey() ?? crypto.randomUUID()
      this.pendingInput ??= { ...input }
      persistPendingKey(this.idempotencyKey)

      try {
        const response = await fetch('/api/rooms', {
          body: JSON.stringify(this.pendingInput),
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
          this.idempotencyKey = null
          this.pendingInput = null
          removePendingKey()
          return
        }

        const error = apiError(result)
        this.errorCode = error?.code ?? 'UNEXPECTED_RESPONSE'
        this.status = 'failed'
        if (error && error.retry !== 'safe_to_retry') {
          this.idempotencyKey = null
          this.pendingInput = null
          removePendingKey()
        }
      } catch {
        this.errorCode = 'NETWORK_UNAVAILABLE'
        this.status = 'failed'
      }
    },
  },
})
