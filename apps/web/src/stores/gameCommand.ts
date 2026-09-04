import { defineStore } from 'pinia'

import { apiError, requestJson, transportErrorCode } from '../api/http'
import {
  isExecuteGameCommandResponse,
  type EffectTargetBinding,
  type ExecuteGameCommandRequest,
  type GameCommandReceipt,
  type GameProjectionResponse,
} from '../contracts/identity-access.generated'

type GameCommandStatus =
  | 'idle'
  | 'submitting'
  | 'uncertain'
  | 'recovering'
  | 'accepted'
  | 'not_committed'
  | 'stale'
  | 'resyncing'
  | 'resynced'
  | 'failed'

type GameCommandType = ExecuteGameCommandRequest['type']
type GameCommandIntent = ExecuteGameCommandRequest extends infer Command
  ? Command extends ExecuteGameCommandRequest
    ? Omit<Command, 'command_id' | 'expected_state_version'>
    : never
  : never

export type PendingGameCommandOverlay = GameCommandIntent

interface PendingGameIntent {
  commandId: string
  commandType: GameCommandType
  createdAt: string
  gameId: string
}

const pendingCommandStorage = 'hogwarts.game-command.pending-intent'
const gameCommandTypes = [
  'end_hero_actions',
  'resolve_choice',
  'play_card',
  'assign_attack',
  'acquire_card',
] as const satisfies readonly GameCommandType[]
const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isGameCommandType(value: unknown): value is GameCommandType {
  return gameCommandTypes.some((commandType) => commandType === value)
}

function cloneOverlay(intent: GameCommandIntent): PendingGameCommandOverlay {
  if (intent.type === 'play_card') {
    return {
      ...intent,
      targets: intent.targets.map((target) => ({
        selector_id: target.selector_id,
        target_ids: [...target.target_ids],
      })),
    }
  }
  if (intent.type === 'resolve_choice') {
    return { ...intent, selected_options: [...intent.selected_options] }
  }
  return { ...intent }
}

function createRequest(
  intent: GameCommandIntent,
  commandId: string,
  expectedStateVersion: number,
): ExecuteGameCommandRequest {
  const metadata = {
    command_id: commandId,
    expected_state_version: expectedStateVersion,
  }
  switch (intent.type) {
    case 'end_hero_actions':
      return { ...metadata, type: intent.type }
    case 'resolve_choice':
      return {
        ...metadata,
        choice_id: intent.choice_id,
        selected_options: intent.selected_options,
        type: intent.type,
      }
    case 'play_card':
      return {
        ...metadata,
        card_id: intent.card_id,
        targets: intent.targets,
        type: intent.type,
      }
    case 'assign_attack':
      return {
        ...metadata,
        amount: intent.amount,
        type: intent.type,
        villain_id: intent.villain_id,
      }
    case 'acquire_card':
      return { ...metadata, card_id: intent.card_id, type: intent.type }
  }
}

function loadPendingIntent(): PendingGameIntent | null {
  try {
    const serialized = sessionStorage.getItem(pendingCommandStorage)
    if (!serialized) {
      return null
    }
    const intent: unknown = JSON.parse(serialized)
    if (
      !isRecord(intent) ||
      typeof intent.commandId !== 'string' ||
      !uuidPattern.test(intent.commandId) ||
      !isGameCommandType(intent.commandType) ||
      typeof intent.createdAt !== 'string' ||
      Number.isNaN(Date.parse(intent.createdAt)) ||
      typeof intent.gameId !== 'string' ||
      !uuidPattern.test(intent.gameId) ||
      Object.keys(intent).length !== 4
    ) {
      sessionStorage.removeItem(pendingCommandStorage)
      return null
    }
    return {
      commandId: intent.commandId,
      commandType: intent.commandType,
      createdAt: intent.createdAt,
      gameId: intent.gameId,
    }
  } catch {
    return null
  }
}

function persistPendingIntent(intent: PendingGameIntent): void {
  try {
    sessionStorage.setItem(pendingCommandStorage, JSON.stringify(intent))
  } catch {
    // The persisted server receipt remains authoritative when storage is unavailable.
  }
}

function removePendingIntent(): void {
  try {
    sessionStorage.removeItem(pendingCommandStorage)
  } catch {
    // Storage availability must not prevent an official response from being applied.
  }
}

export const useGameCommandStore = defineStore('gameCommand', {
  state: (): {
    status: GameCommandStatus
    pendingIntent: PendingGameIntent | null
    pendingOverlay: PendingGameCommandOverlay | null
    receipt: GameCommandReceipt | null
    errorCode: string | null
  } => {
    const pendingIntent = loadPendingIntent()
    return {
      status: pendingIntent ? 'uncertain' : 'idle',
      pendingIntent,
      pendingOverlay: null,
      receipt: null,
      errorCode: null,
    }
  },
  actions: {
    async endHeroActions(game: GameProjectionResponse): Promise<GameProjectionResponse | null> {
      return this.execute(game, { type: 'end_hero_actions' })
    },
    async resolveChoice(
      game: GameProjectionResponse,
      choiceId: string,
      selectedOptions: string[],
    ): Promise<GameProjectionResponse | null> {
      return this.execute(game, {
        choice_id: choiceId,
        selected_options: selectedOptions,
        type: 'resolve_choice',
      })
    },
    async playCard(
      game: GameProjectionResponse,
      cardId: string,
      targets: EffectTargetBinding[],
    ): Promise<GameProjectionResponse | null> {
      return this.execute(game, { card_id: cardId, targets, type: 'play_card' })
    },
    async assignAttack(
      game: GameProjectionResponse,
      villainId: string,
      amount: number,
    ): Promise<GameProjectionResponse | null> {
      return this.execute(game, { amount, type: 'assign_attack', villain_id: villainId })
    },
    async acquireCard(
      game: GameProjectionResponse,
      cardId: string,
    ): Promise<GameProjectionResponse | null> {
      return this.execute(game, { card_id: cardId, type: 'acquire_card' })
    },
    async execute(
      game: GameProjectionResponse,
      intent: GameCommandIntent,
    ): Promise<GameProjectionResponse | null> {
      if (this.status === 'submitting' || this.status === 'recovering' || this.pendingIntent) {
        return null
      }

      const request = createRequest(
        intent,
        crypto.randomUUID(),
        game.snapshot.state_version,
      )
      this.pendingIntent = {
        commandId: request.command_id,
        commandType: request.type,
        createdAt: new Date().toISOString(),
        gameId: game.game.id,
      }
      this.pendingOverlay = cloneOverlay(intent)
      this.status = 'submitting'
      this.errorCode = null
      this.receipt = null
      persistPendingIntent(this.pendingIntent)

      try {
        const { body: result, response } = await requestJson('/api/games/current/commands', {
          body: JSON.stringify(request),
          cache: 'no-store',
          credentials: 'same-origin',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
          },
          method: 'POST',
        })
        if (response.ok && isExecuteGameCommandResponse(result)) {
          return this.accept(result.receipt, result.projection)
        }

        this.errorCode = apiError(result)?.code ?? 'UNEXPECTED_RESPONSE'
        if (response.status >= 500 || this.errorCode === 'UNEXPECTED_RESPONSE') {
          this.status = 'uncertain'
        } else {
          this.status = this.errorCode === 'STALE_STATE_VERSION' ? 'stale' : 'failed'
          this.pendingIntent = null
          this.pendingOverlay = null
          removePendingIntent()
        }
        return null
      } catch (error) {
        this.errorCode = transportErrorCode(error)
        this.status = 'uncertain'
        return null
      }
    },
    async recoverPending(gameId: string): Promise<GameProjectionResponse | null> {
      if (!this.pendingIntent || this.status === 'submitting' || this.status === 'recovering') {
        return null
      }
      if (this.pendingIntent.gameId !== gameId) {
        this.pendingIntent = null
        this.pendingOverlay = null
        this.status = 'idle'
        this.errorCode = null
        removePendingIntent()
        return null
      }

      this.status = 'recovering'
      this.errorCode = null
      try {
        const { body: result, response } = await requestJson(
          `/api/games/current/commands/${encodeURIComponent(this.pendingIntent.commandId)}`,
          {
            cache: 'no-store',
            credentials: 'same-origin',
            headers: { Accept: 'application/json' },
          },
        )
        if (response.ok && isExecuteGameCommandResponse(result)) {
          return this.accept(result.receipt, result.projection)
        }

        this.errorCode = apiError(result)?.code ?? 'UNEXPECTED_RESPONSE'
        if (response.status === 404 && this.errorCode === 'COMMAND_NOT_FOUND') {
          this.pendingIntent = null
          this.pendingOverlay = null
          this.status = 'not_committed'
          removePendingIntent()
        } else {
          this.status = 'uncertain'
        }
        return null
      } catch (error) {
        this.errorCode = transportErrorCode(error)
        this.status = 'uncertain'
        return null
      }
    },
    clearFeedback(): void {
      if (this.pendingIntent || this.status === 'submitting' || this.status === 'recovering') {
        return
      }
      this.status = 'idle'
      this.errorCode = null
      this.receipt = null
    },
    beginStaleResync(): boolean {
      if (this.status !== 'stale' || this.errorCode !== 'STALE_STATE_VERSION') {
        return false
      }
      this.status = 'resyncing'
      return true
    },
    finishStaleResync(succeeded: boolean): void {
      if (this.status !== 'resyncing') {
        return
      }
      this.status = succeeded ? 'resynced' : 'stale'
    },
    accept(
      receipt: GameCommandReceipt,
      projection: GameProjectionResponse,
    ): GameProjectionResponse {
      this.receipt = receipt
      this.pendingIntent = null
      this.pendingOverlay = null
      this.status = 'accepted'
      this.errorCode = null
      removePendingIntent()
      return projection
    },
  },
})
