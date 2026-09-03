import { defineStore } from 'pinia'

import { apiError, requestJson, transportErrorCode } from '../api/http'
import {
  isCreateRoomResponse,
  type CreateRoomRequest,
  type CreateRoomResponse,
} from '../contracts/identity-access.generated'

export type RoomCreationStatus = 'idle' | 'submitting' | 'succeeded' | 'failed'

interface PendingRoomIntent {
  commandType: 'create_room'
  createdAt: string
  idempotencyKey: string
}

const pendingIntentStorage = 'hogwarts.room-creation.pending-intent'
const idempotencyKeyPattern = /^[A-Za-z0-9_.:-]{8,128}$/

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function loadPendingIntent(): PendingRoomIntent | null {
  try {
    const serialized = sessionStorage.getItem(pendingIntentStorage)
    if (!serialized) {
      return null
    }
    const intent: unknown = JSON.parse(serialized)
    if (
      !isRecord(intent) ||
      typeof intent.idempotencyKey !== 'string' ||
      !idempotencyKeyPattern.test(intent.idempotencyKey) ||
      intent.commandType !== 'create_room' ||
      typeof intent.createdAt !== 'string' ||
      Number.isNaN(Date.parse(intent.createdAt)) ||
      Object.keys(intent).length !== 3
    ) {
      sessionStorage.removeItem(pendingIntentStorage)
      return null
    }
    return {
      commandType: intent.commandType,
      createdAt: intent.createdAt,
      idempotencyKey: intent.idempotencyKey,
    }
  } catch {
    return null
  }
}

function persistPendingIntent(intent: PendingRoomIntent): void {
  try {
    sessionStorage.setItem(pendingIntentStorage, JSON.stringify(intent))
  } catch {
    // Server-side idempotency still protects retries while this store remains alive.
  }
}

function removePendingIntent(): void {
  try {
    sessionStorage.removeItem(pendingIntentStorage)
  } catch {
    // Storage availability must not prevent a definitive response from being handled.
  }
}

export const useRoomCreationStore = defineStore('roomCreation', {
  state: (): {
    status: RoomCreationStatus
    roomCreation: CreateRoomResponse | null
    errorCode: string | null
    idempotencyKey: string | null
    pendingIntent: PendingRoomIntent | null
    pendingInput: CreateRoomRequest | null
    recoveringPendingIntent: boolean
  } => {
    const pendingIntent = loadPendingIntent()
    return {
      status: 'idle',
      roomCreation: null,
      errorCode: null,
      idempotencyKey: pendingIntent?.idempotencyKey ?? null,
      pendingIntent,
      pendingInput: null,
      recoveringPendingIntent: pendingIntent !== null,
    }
  },
  actions: {
    resetPendingRequest(): void {
      if (this.status === 'submitting') {
        return
      }
      if (this.recoveringPendingIntent && this.pendingIntent) {
        this.idempotencyKey = this.pendingIntent.idempotencyKey
        this.pendingInput = null
        this.errorCode = null
        this.status = 'idle'
        return
      }

      this.idempotencyKey = null
      this.pendingIntent = null
      this.pendingInput = null
      this.errorCode = null
      this.status = 'idle'
      removePendingIntent()
    },
    discardPendingRequest(): void {
      if (this.status === 'submitting') {
        return
      }
      this.idempotencyKey = null
      this.pendingIntent = null
      this.pendingInput = null
      this.recoveringPendingIntent = false
      this.errorCode = null
      this.status = 'idle'
      removePendingIntent()
    },
    async createRoom(input: CreateRoomRequest): Promise<void> {
      if (this.status === 'submitting') {
        return
      }

      this.status = 'submitting'
      this.errorCode = null
      this.idempotencyKey ??= crypto.randomUUID()
      this.pendingIntent ??= {
        commandType: 'create_room',
        createdAt: new Date().toISOString(),
        idempotencyKey: this.idempotencyKey,
      }
      this.pendingInput ??= { ...input }
      persistPendingIntent(this.pendingIntent)

      try {
        const { body: result, response } = await requestJson('/api/rooms', {
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
        if (response.ok && isCreateRoomResponse(result)) {
          this.roomCreation = result
          this.status = 'succeeded'
          this.idempotencyKey = null
          this.pendingIntent = null
          this.pendingInput = null
          this.recoveringPendingIntent = false
          removePendingIntent()
          return
        }

        const error = apiError(result)
        this.errorCode = error?.code ?? 'UNEXPECTED_RESPONSE'
        this.status = 'failed'
        if (this.recoveringPendingIntent && error) {
          this.pendingInput = null
        } else if (error && error.retry !== 'safe_to_retry') {
          this.idempotencyKey = null
          this.pendingIntent = null
          this.pendingInput = null
          removePendingIntent()
        }
      } catch (error) {
        this.errorCode = transportErrorCode(error)
        this.status = 'failed'
      }
    },
  },
})
