<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { isUncertainTransportFailure, requestJson } from './api/http'
import GameStage from './components/GameStage.vue'
import RecoveryManagement from './components/RecoveryManagement.vue'
import type {
  HeroId,
  PendingChoiceSummary,
  StartGameRequest,
} from './contracts/identity-access.generated'
import { takeRecoveryToken } from './recoveryCredential'
import { useAccessProtectionStore } from './stores/accessProtection'
import { type Availability, useHealthStore } from './stores/health'
import { useGameCommandStore } from './stores/gameCommand'
import { useGameSyncStore } from './stores/gameSync'
import { useRoomAccessStore } from './stores/roomAccess'
import { useRoomCreationStore } from './stores/roomCreation'
import { useRecoveryManagementStore } from './stores/recoveryManagement'
import { useSecuritySyncStore } from './stores/securitySync'

type AccessInvalidationReason =
  | 'session_revoked'
  | 'participant_protected'
  | 'room_protected'
  | 'game_expired'

const accessProtection = useAccessProtectionStore()
const health = useHealthStore()
const gameCommand = useGameCommandStore()
const gameSync = useGameSyncStore()
const roomAccess = useRoomAccessStore()
const roomCreation = useRoomCreationStore()
const recoveryManagement = useRecoveryManagementStore()
const securitySync = useSecuritySyncStore()
const accessInvalidationReason = ref<AccessInvalidationReason | null>(null)
const recoveryToken = ref(takeRecoveryToken())
const entryMode = ref<'create' | 'join' | 'recover'>(
  recoveryToken.value ? 'recover' : 'create',
)
const displayName = ref('')
const recoveryPassword = ref('')
const roomCode = ref('')
const selectedHero = ref<HeroId | ''>('')
const selectedChoiceOptions = ref<string[]>([])
const passwordVisible = ref(false)
const copyResult = ref<'idle' | 'copied' | 'failed'>('idle')
const selectedContentKey = ref('')
const selectedReplacementSessionId = ref('')

if (!recoveryToken.value && roomAccess.pendingJoinIntent) {
  entryMode.value = 'join'
  displayName.value = roomAccess.pendingJoinIntent.input.display_name
  roomCode.value = roomAccess.pendingJoinIntent.roomCode
  selectedHero.value = roomAccess.pendingJoinIntent.input.hero_id
}

const statusPresentation = {
  checking: {
    description: 'Confirmando acesso ao serviço autoritativo.',
    label: 'Verificando servidor',
  },
  ready: {
    description: 'O serviço autoritativo está pronto para receber o grupo.',
    label: 'Servidor pronto',
  },
  unavailable: {
    description: 'Não foi possível confirmar o serviço autoritativo. Tente novamente.',
    label: 'Servidor indisponível',
  },
} satisfies Record<Availability, { description: string; label: string }>

const currentStatus = computed(() => statusPresentation[health.availability])
const lobby = computed(() => roomAccess.lobby)
const game = computed(() => roomAccess.game)
const pendingChoice = computed<PendingChoiceSummary | null>(() => {
  const choice = game.value?.choice
  return choice?.status === 'pending' ? choice : null
})
const pendingChoiceResponsibleName = computed(() => {
  const choice = pendingChoice.value
  if (!choice) {
    return ''
  }
  return (
    game.value?.participants.find(
      (participant) => participant.position === choice.responsible_position,
    )?.display_name ?? `a posição ${choice.responsible_position}`
  )
})
const isResponsibleForPendingChoice = computed(() => {
  const choice = pendingChoice.value
  const participant = game.value?.participant
  return Boolean(choice && participant && choice.responsible_position === participant.position)
})
const isSelectableChoiceForParticipant = computed(() => {
  const choice = pendingChoice.value
  return Boolean(
    isResponsibleForPendingChoice.value &&
      choice &&
      choice.min >= 0 &&
      choice.max >= choice.min &&
      choice.max <= choice.options.length,
  )
})
const commandSubmissionBlocked = computed(
  () =>
    gameSync.commandsFrozen ||
    Boolean(gameCommand.pendingIntent) ||
    ['submitting', 'recovering', 'stale', 'resyncing'].includes(gameCommand.status),
)
const choiceInputDisabled = computed(
  () =>
    commandSubmissionBlocked.value ||
    game.value?.legal_actions.includes('resolve_choice') !== true,
)
const orderedSelectedChoiceOptions = computed(() => {
  const choice = pendingChoice.value
  if (!choice) {
    return []
  }
  const selectedOptions = new Set(selectedChoiceOptions.value)
  return choice.options.filter((option) => selectedOptions.has(option))
})
const canResolvePendingChoice = computed(
  () => {
    const choice = pendingChoice.value
    const selectedCount = selectedChoiceOptions.value.length
    return Boolean(
      isSelectableChoiceForParticipant.value &&
        !choiceInputDisabled.value &&
        choice &&
        orderedSelectedChoiceOptions.value.length === selectedCount &&
        selectedCount >= choice.min &&
        selectedCount <= choice.max,
    )
  },
)
const pendingChoiceFocusKey = computed(() => {
  const choice = pendingChoice.value
  return gameSync.status === 'connected' &&
    isSelectableChoiceForParticipant.value &&
    !choiceInputDisabled.value &&
    choice
    ? choice.id
    : null
})
const isHost = computed(() => lobby.value?.participant.role === 'host')
const isRestoringSession = computed(
  () => roomAccess.status === 'restoring' && !lobby.value && !game.value,
)
const sessionNeedsRecovery = computed(
  () => roomAccess.sessionExpected && !lobby.value && !game.value,
)
const recoveryNeedsReplacement = computed(() => roomAccess.replacementSessions.length === 2)
const selectedReplacementSession = computed(() =>
  roomAccess.replacementSessions.find(
    (session) => session.id === selectedReplacementSessionId.value,
  ),
)
const lookupCode = computed(() => roomAccess.roomLookup?.room.code ?? '')
const lookupHeroes = computed(() => roomAccess.roomLookup?.heroes ?? [])
const adventureChoices = computed(() =>
  (lobby.value?.content_options ?? []).flatMap((manifest) =>
    manifest.adventures.map((adventure) => ({
      key: `${manifest.manifest_digest}:${adventure.id}`,
      adventure,
      manifest,
      playable: manifest.playable && adventure.playable,
    })),
  ),
)
const selectedContent = computed(() =>
  adventureChoices.value.find((choice) => choice.key === selectedContentKey.value),
)
const lobbyIsReadyToSeal = computed(
  () =>
    Boolean(lobby.value) &&
    (lobby.value?.participants.length ?? 0) >= 2 &&
    lobby.value?.participants.every((participant) => participant.ready && participant.hero),
)
const canStartGame = computed(
  () => Boolean(isHost.value && lobbyIsReadyToSeal.value && selectedContent.value?.playable),
)
const endHeroActionsIsLegal = computed(() => {
  const currentGame = game.value
  return (
    currentGame?.turn.phase === 'hero_actions' &&
    currentGame.participant.position === currentGame.turn.active_position &&
    currentGame.legal_actions.includes('end_hero_actions')
  )
})
const canEndHeroActions = computed(
  () =>
    endHeroActionsIsLegal.value &&
    !commandSubmissionBlocked.value,
)
const serviceHeading = computed(() => {
  if (accessInvalidationReason.value === 'game_expired') {
    return 'Partida expirada'
  }
  if (accessInvalidationReason.value) {
    return 'Acesso protegido'
  }
  if (isRestoringSession.value) {
    return 'Retomando sua sessão'
  }
  if (health.availability === 'ready' && sessionNeedsRecovery.value) {
    return 'Não foi possível retomar'
  }
  return currentStatus.value.label
})
const serviceDescription = computed(() => {
  if (accessInvalidationReason.value === 'game_expired') {
    return 'A partida encerrou após sete dias sem uma ação oficial aceita. Os dados privados e as ações pendentes deste navegador foram removidos. Você pode criar uma nova sala.'
  }
  if (accessInvalidationReason.value === 'participant_protected') {
    return 'Todas as sessões e links desta participação foram revogados. Use um novo link de recuperação para retornar.'
  }
  if (accessInvalidationReason.value === 'room_protected') {
    return 'A senha, os links e as sessões da sala foram renovados. Solicite ao Anfitrião as novas credenciais para retornar.'
  }
  if (accessInvalidationReason.value === 'session_revoked') {
    return 'Esta sessão foi revogada. O estado privado e as ações pendentes deste navegador foram removidos.'
  }
  if (isRestoringSession.value) {
    return 'Confirmando sua posição durável nesta mesa.'
  }
  if (health.availability === 'ready' && sessionNeedsRecovery.value) {
    return 'Sua posição continua vinculada a este navegador. Tente novamente quando a conexão voltar.'
  }
  return currentStatus.value.description
})
const displayNameError = computed(() =>
  roomCreation.errorCode === 'INVALID_DISPLAY_NAME' ||
  roomAccess.errorCode === 'INVALID_DISPLAY_NAME'
    ? 'Informe um nome entre 1 e 40 caracteres.'
    : null,
)
const passwordError = computed(() =>
  roomCreation.errorCode === 'WEAK_RECOVERY_PASSWORD'
    ? 'Escolha uma senha mais longa e menos previsível.'
    : null,
)
const participationRecoveryError = computed(() => {
  if (isUncertainTransportFailure(roomAccess.errorCode)) {
    return 'A confirmação não chegou. Tente novamente nesta tela ou reabra o link nesta mesma aba.'
  }
  if (roomAccess.errorCode === 'RECOVERY_FAILED') {
    return 'Não foi possível recuperar a participação. Confira o link e a senha da sala.'
  }
  return roomAccess.errorCode
    ? 'O serviço não conseguiu confirmar a recuperação. Tente novamente com o mesmo link.'
    : null
})
const createFormError = computed(() => {
  if (isUncertainTransportFailure(roomCreation.errorCode)) {
    return 'A confirmação não chegou. Tente novamente para consultar a mesma criação.'
  }
  switch (roomCreation.errorCode) {
    case 'INVALID_DISPLAY_NAME':
    case 'WEAK_RECOVERY_PASSWORD':
    case null:
      return null
    case 'IDEMPOTENCY_KEY_REUSED':
      return roomCreation.recoveringPendingIntent
        ? 'O nome ou a senha não correspondem à criação pendente. Reinsira os mesmos dados ou descarte a tentativa.'
        : 'Não foi possível retomar a criação. Descarte a tentativa pendente para começar outra.'
    default:
      return 'Não foi possível criar a sala. Revise os dados e tente novamente.'
  }
})
const joinFormError = computed(() => {
  if (isUncertainTransportFailure(roomAccess.errorCode)) {
    if (roomAccess.pendingJoinIntent && !roomAccess.roomLookup) {
      return 'A confirmação da entrada não chegou. Tente retomar a mesma solicitação.'
    }
    return roomAccess.roomLookup
      ? 'A confirmação não chegou. Tente entrar novamente com os mesmos dados.'
      : 'Não foi possível localizar a sala. Confira sua conexão e tente novamente.'
  }
  switch (roomAccess.errorCode) {
    case null:
      return null
    case 'ROOM_NOT_FOUND':
    case 'ROOM_UNAVAILABLE':
      return 'Não foi possível encontrar uma sala aberta com esse código.'
    case 'ROOM_FULL':
      return 'A sala já tem quatro participantes.'
    case 'HERO_UNAVAILABLE':
      return 'Outro participante escolheu esse Herói primeiro. Escolha um dos disponíveis.'
    case 'INVALID_HERO':
      return 'Escolha um Herói disponível.'
    default:
      return 'Não foi possível entrar na sala. Revise os dados e tente novamente.'
  }
})
const lobbyError = computed(() => {
  if (isUncertainTransportFailure(roomAccess.errorCode)) {
    return 'A confirmação não chegou. Repita a mesma ação para consultar o resultado.'
  }
  switch (roomAccess.errorCode) {
    case 'HERO_UNAVAILABLE':
      return 'Outro participante escolheu esse Herói primeiro. Atualize sua escolha.'
    case 'INTERNAL_ERROR':
    case 'UNEXPECTED_RESPONSE':
      return 'A confirmação da partida falhou. Tente novamente com a mesma solicitação.'
    case 'ROOM_SEALED':
      return 'A sala já foi selada. Atualize para receber sua projeção inicial.'
    case 'ROOM_PARTICIPANT_COUNT_INVALID':
      return 'A sala precisa ter entre dois e quatro participantes.'
    case 'PARTICIPANT_HEROES_INVALID':
      return 'Cada participante precisa confirmar um Herói único.'
    case 'PARTICIPANTS_NOT_READY':
      return 'Todos os participantes precisam confirmar que estão prontos.'
    case 'CONTENT_NOT_PLAYABLE':
      return 'O conteúdo selecionado ainda possui lacunas funcionais e não pode iniciar uma partida.'
    case null:
      return null
    default:
      return 'Não foi possível atualizar seu Herói.'
  }
})
const createSubmitLabel = computed(() => {
  if (roomCreation.status === 'submitting') {
    return 'Criando sala'
  }
  if (roomCreation.recoveringPendingIntent) {
    return 'Retomar criação pendente'
  }
  return roomCreation.status === 'failed' ? 'Tentar criar novamente' : 'Criar sala privada'
})
const joinSubmitLabel = computed(() => {
  if (!roomAccess.roomLookup) {
    if (roomAccess.pendingJoinIntent) {
      return roomAccess.status === 'joining' ? 'Retomando entrada' : 'Retomar entrada pendente'
    }
    return roomAccess.status === 'looking_up' ? 'Localizando sala' : 'Localizar sala'
  }
  if (roomAccess.status === 'joining') {
    return 'Entrando na sala'
  }
  return isUncertainTransportFailure(roomAccess.errorCode)
    ? 'Tentar entrar novamente'
    : 'Entrar na sala'
})

function lobbyIsBusy(): boolean {
  return ['selecting_hero', 'setting_readiness', 'starting_game', 'restoring'].includes(
    roomAccess.status,
  )
}

function retry(): void {
  if (health.availability !== 'checking') {
    void health.check()
  }
}

function retrySession(): void {
  void roomAccess.restoreSession()
}

function handleAccessInvalidation(reason: AccessInvalidationReason): void {
  if (accessInvalidationReason.value === reason) {
    return
  }
  if (reason === 'game_expired') {
    // A WebSocket cannot clear the HttpOnly cookie. The expired session endpoint can.
    void requestJson('/api/session').catch(() => undefined)
  }
  accessInvalidationReason.value = reason
  selectedChoiceOptions.value = []
  selectedReplacementSessionId.value = ''
  recoveryToken.value = null
  displayName.value = ''
  recoveryPassword.value = ''
  roomCode.value = ''
  selectedHero.value = ''
  selectedContentKey.value = ''
  passwordVisible.value = false
  copyResult.value = 'idle'
  roomCreation.discardPendingRequest()
  roomCreation.$reset()
  gameSync.disconnect()
  securitySync.disconnect()
  gameCommand.clearPrivateState()
  recoveryManagement.$reset()
  accessProtection.clearPrivateState()
  roomAccess.clearAuthenticatedSession()
}

async function returnToEntry(): Promise<void> {
  accessInvalidationReason.value = null
  entryMode.value = 'create'
  displayName.value = ''
  recoveryPassword.value = ''
  roomCode.value = ''
  selectedHero.value = ''
  await nextTick()
  document.getElementById('display-name')?.focus()
}

async function createRoom(): Promise<void> {
  await roomCreation.createRoom({
    display_name: displayName.value,
    recovery_password: recoveryPassword.value,
  })

  const createdRoom = roomCreation.takeRoomCreation()
  if (createdRoom) {
    roomAccess.adoptCreatedRoom(createdRoom)
    recoveryPassword.value = ''
    passwordVisible.value = false
  }
  await focusAfterAction(roomCreation.errorCode)
}

async function recoverParticipation(): Promise<void> {
  const token = recoveryToken.value
  if (!token) {
    return
  }
  const recovered = await roomAccess.recoverParticipation({
    recovery_password: recoveryPassword.value,
    recovery_token: token,
    ...(selectedReplacementSessionId.value
      ? { replace_session_id: selectedReplacementSessionId.value }
      : {}),
  })
  if (recovered) {
    recoveryToken.value = null
    recoveryPassword.value = ''
    passwordVisible.value = false
  }
  await nextTick()
  if (recoveryNeedsReplacement.value) {
    document.getElementById('participation-recovery-heading')?.focus()
    return
  }
  document
    .getElementById(
      recovered ? (game.value ? 'game-heading' : 'room-success-heading') : 'recovery-room-password',
    )
    ?.focus()
}

async function leaveParticipationRecovery(): Promise<void> {
  roomAccess.dismissParticipationRecovery()
  recoveryToken.value = null
  recoveryPassword.value = ''
  selectedReplacementSessionId.value = ''
  passwordVisible.value = false
  entryMode.value = 'create'
  await nextTick()
  document.getElementById('display-name')?.focus()
}

function formatSessionCreatedAt(createdAt: string): string {
  return new Intl.DateTimeFormat('pt-BR', {
    dateStyle: 'short',
    timeStyle: 'short',
  }).format(new Date(createdAt))
}

async function findRoom(): Promise<void> {
  await roomAccess.findRoom(roomCode.value)
  if (roomAccess.roomLookup) {
    roomCode.value = roomAccess.roomLookup.room.code
    await nextTick()
    document.getElementById('join-display-name')?.focus()
  } else {
    await nextTick()
    document.getElementById('room-code')?.focus()
  }
}

async function findOrRecoverRoom(): Promise<void> {
  if (roomAccess.pendingJoinIntent) {
    await roomAccess.recoverPendingJoin()
    await focusAfterAction(roomAccess.errorCode)
    return
  }
  await findRoom()
}

async function joinRoom(): Promise<void> {
  if (!selectedHero.value) {
    return
  }
  await roomAccess.joinRoom({
    display_name: displayName.value,
    hero_id: selectedHero.value,
  })
  if (roomAccess.errorCode === 'HERO_UNAVAILABLE') {
    selectedHero.value = ''
  }
  await focusAfterAction(roomAccess.errorCode)
}

async function confirmHero(): Promise<void> {
  if (!selectedHero.value) {
    return
  }
  await roomAccess.selectHero(selectedHero.value)
  await focusAfterAction(roomAccess.errorCode)
}

async function toggleReadiness(): Promise<void> {
  if (!lobby.value?.participant.hero) {
    return
  }
  await roomAccess.setReadiness(!lobby.value.participant.ready)
  await focusAfterAction(roomAccess.errorCode)
}

async function startGame(): Promise<void> {
  const content = selectedContent.value
  if (!content || !canStartGame.value) {
    return
  }
  const input: StartGameRequest = {
    adventure_id: content.adventure.id,
    manifest_digest: content.manifest.manifest_digest,
    ruleset_version: content.manifest.ruleset_version,
  }
  await roomAccess.startGame(input)
  await nextTick()
  document.getElementById(game.value ? 'game-heading' : 'room-success-heading')?.focus()
}

async function refreshLobby(): Promise<void> {
  await roomAccess.refreshSession()
  await nextTick()
  document.getElementById(game.value ? 'game-heading' : 'room-success-heading')?.focus()
}

async function endHeroActions(): Promise<void> {
  if (!game.value || !canEndHeroActions.value) {
    return
  }
  const projection = await gameCommand.endHeroActions(game.value)
  if (projection) {
    roomAccess.advanceGameProjection(projection)
    await nextTick()
    document.getElementById('game-heading')?.focus()
  } else if (gameCommand.status === 'stale') {
    await resyncStaleGame()
  }
}

async function resolvePendingChoice(): Promise<void> {
  const currentGame = game.value
  const choice = pendingChoice.value
  if (!currentGame || !choice || !canResolvePendingChoice.value) {
    return
  }

  const projection = await gameCommand.resolveChoice(
    currentGame,
    choice.id,
    orderedSelectedChoiceOptions.value,
  )
  if (projection) {
    roomAccess.advanceGameProjection(projection)
    await nextTick()
    document.getElementById('game-heading')?.focus()
  } else if (gameCommand.status === 'stale') {
    await resyncStaleGame()
  } else if (gameCommand.errorCode === 'CHOICE_NOT_ASSIGNED') {
    gameSync.resynchronize()
  }
}

async function resyncStaleGame(): Promise<void> {
  const staleGameId = game.value?.game.id
  if (!staleGameId || !gameCommand.beginStaleResync()) {
    return
  }

  await roomAccess.refreshSession()
  gameCommand.finishStaleResync(
    roomAccess.status === 'ready' && roomAccess.game?.game.id === staleGameId,
  )
  await nextTick()
  document.getElementById('game-heading')?.focus()
}

async function recoverGameCommand(): Promise<void> {
  if (!game.value) {
    return
  }
  const projection = await gameCommand.recoverPending(game.value.game.id)
  if (projection) {
    roomAccess.advanceGameProjection(projection)
    await nextTick()
    document.getElementById('game-heading')?.focus()
  }
}

async function focusAfterAction(errorCode: string | null): Promise<void> {
  await nextTick()
  if (lobby.value) {
    const nextAction =
      errorCode === null
        ? document.querySelector<HTMLButtonElement>('.action-dock .primary-button')
        : null
    const focusTarget = nextAction ?? document.getElementById('room-success-heading')
    focusTarget?.focus()
  } else if (errorCode === 'INVALID_DISPLAY_NAME') {
    document.getElementById(entryMode.value === 'join' ? 'join-display-name' : 'display-name')?.focus()
  } else if (errorCode === 'WEAK_RECOVERY_PASSWORD') {
    document.getElementById('recovery-password')?.focus()
  }
}

function togglePassword(): void {
  passwordVisible.value = !passwordVisible.value
}

function discardPendingRequest(): void {
  roomCreation.discardPendingRequest()
  displayName.value = ''
  recoveryPassword.value = ''
  passwordVisible.value = false
}

function showJoin(): void {
  entryMode.value = 'join'
  displayName.value = ''
  roomCreation.resetPendingRequest()
}

function showCreate(): void {
  entryMode.value = 'create'
  displayName.value = ''
  roomCode.value = ''
  selectedHero.value = ''
  roomAccess.clearLookup()
}

async function discardPendingJoin(): Promise<void> {
  if (roomAccess.status === 'joining') {
    return
  }
  roomAccess.clearLookup()
  displayName.value = ''
  roomCode.value = ''
  selectedHero.value = ''
  await nextTick()
  document.getElementById('room-code')?.focus()
}

function heroIsSelectable(heroId: HeroId, available: boolean): boolean {
  return available || lobby.value?.participant.hero?.id === heroId
}

async function copyRoomCode(): Promise<void> {
  const code = lobby.value?.room.code
  if (!code) {
    return
  }

  try {
    await navigator.clipboard.writeText(code)
    copyResult.value = 'copied'
  } catch {
    copyResult.value = 'failed'
  }
}

watch([displayName, recoveryPassword], () => roomCreation.resetPendingRequest())
watch(
  adventureChoices,
  (choices) => {
    if (!choices.some((choice) => choice.key === selectedContentKey.value)) {
      selectedContentKey.value = choices.find((choice) => choice.playable)?.key ?? ''
    }
  },
  { immediate: true },
)
watch(
  () => pendingChoice.value?.id,
  () => {
    selectedChoiceOptions.value = []
  },
)
watch(pendingChoiceFocusKey, async (choiceId) => {
  if (!choiceId) {
    return
  }
  await nextTick()
  document.getElementById('pending-choice-option-0')?.focus()
})
watch(
  game,
  (current) => {
    if (current) {
      gameSync.connect(current)
    } else {
      gameSync.disconnect()
    }
  },
  { immediate: true },
)
watch(
  () => Boolean(lobby.value || game.value),
  (hasAuthenticatedSession) => {
    if (hasAuthenticatedSession) {
      securitySync.connect()
    } else {
      securitySync.disconnect()
    }
  },
  { immediate: true },
)
watch(
  [() => securitySync.sessionInvalidated, () => gameSync.sessionInvalidated],
  ([securitySessionInvalidated, gameSessionInvalidated]) => {
    if (securitySessionInvalidated || gameSessionInvalidated) {
      handleAccessInvalidation(
        securitySync.gameExpired || gameSync.gameExpired ? 'game_expired' : 'session_revoked',
      )
    }
  },
)

watch(
  [
    () => roomAccess.errorCode,
    () => gameCommand.errorCode,
    () => recoveryManagement.errorCode,
    () => accessProtection.errorCode,
  ],
  (errors) => {
    if (errors.includes('GAME_EXPIRED')) {
      handleAccessInvalidation('game_expired')
    } else if (
      errors.includes('SESSION_INVALID') &&
      (roomAccess.game || roomAccess.lobby || gameCommand.pendingIntent)
    ) {
      // Another tab may have already removed the shared cookie after expiry.
      handleAccessInvalidation('session_revoked')
    }
  },
)

function revalidateVisibleSession(): void {
  if (document.visibilityState === 'visible' && (roomAccess.game || roomAccess.lobby)) {
    void roomAccess.revalidateSession()
  }
}

onBeforeUnmount(() => {
  document.removeEventListener('visibilitychange', revalidateVisibleSession)
  window.removeEventListener('pageshow', revalidateVisibleSession)
  window.removeEventListener('online', revalidateVisibleSession)
  gameSync.disconnect()
  securitySync.disconnect()
})

onMounted(async () => {
  document.addEventListener('visibilitychange', revalidateVisibleSession)
  window.addEventListener('pageshow', revalidateVisibleSession)
  window.addEventListener('online', revalidateVisibleSession)
  await Promise.all([
    health.check(),
    recoveryToken.value ? Promise.resolve() : roomAccess.restoreSession(),
  ])
  if (!recoveryToken.value && !lobby.value && !game.value && roomAccess.pendingJoinIntent) {
    await roomAccess.recoverPendingJoin()
  }
  if (game.value && gameCommand.pendingIntent) {
    await recoverGameCommand()
  }
  if (lobby.value || game.value) {
    await nextTick()
    document.getElementById(game.value ? 'game-heading' : 'room-success-heading')?.focus()
  }
})
</script>

<template>
  <main class="shell">
    <header class="masthead">
      <span class="cue-mark" aria-hidden="true"></span>
      <h1>Batalha de Hogwarts</h1>
      <span class="edition">Mesa cooperativa</span>
    </header>

    <section
      v-if="health.availability !== 'ready' || isRestoringSession || sessionNeedsRecovery || accessInvalidationReason"
      class="service-check"
      :class="`service-check--${accessInvalidationReason ? 'ended' : health.availability}`"
      aria-labelledby="service-heading"
      :aria-busy="health.availability === 'checking' || roomAccess.status === 'restoring'"
    >
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">1</span>
        <span class="cue-line"></span>
        <span class="cue-label">Estado oficial</span>
      </div>

      <div class="service-state" role="status" aria-live="polite" aria-atomic="true">
        <div class="state-heading">
          <span class="state-signal" aria-hidden="true"></span>
          <h2 id="service-heading">{{ serviceHeading }}</h2>
        </div>
        <p class="state-description">
          {{ serviceDescription }}
        </p>
      </div>
    </section>

    <GameStage
      v-else-if="game"
      v-model:selected-choice-options="selectedChoiceOptions"
      :choice-input-disabled="choiceInputDisabled"
      :is-choice-responsible="isResponsibleForPendingChoice"
      @access-invalidated="handleAccessInvalidation"
    />

    <section
      v-else-if="lobby"
      class="room-success"
      aria-labelledby="room-success-heading"
      aria-live="polite"
      :aria-busy="lobbyIsBusy()"
    >
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">3</span>
        <span class="cue-line"></span>
        <span class="cue-label">Sala aberta</span>
      </div>

      <div class="room-stage room-stage--success">
        <p class="service-confirmation" role="status">
          <span class="state-signal" aria-hidden="true"></span>
          Posição confirmada
        </p>
        <h2 id="room-success-heading" tabindex="-1">
          {{ isHost ? 'Sala pronta' : 'Sala aberta' }}
        </h2>
        <p class="stage-description">
          Sua participação está vinculada a esta sessão. O código apenas localiza a sala.
        </p>

        <div class="room-code-block">
          <span id="room-code-label">Código da sala</span>
          <output aria-labelledby="room-code-label">{{ lobby.room.code }}</output>
        </div>

        <RecoveryManagement
          :participant="lobby.participant"
          :participants="lobby.participants"
          @access-invalidated="handleAccessInvalidation"
        />

        <dl class="room-details">
          <div>
            <dt>{{ isHost ? 'Anfitrião da sala' : 'Sua participação' }}</dt>
            <dd>{{ lobby.participant.display_name }}</dd>
          </div>
          <div>
            <dt>Posição durável</dt>
            <dd>Posição {{ lobby.participant.position }}</dd>
          </div>
          <div>
            <dt>Herói</dt>
            <dd>{{ lobby.participant.hero?.name ?? 'Ainda não escolhido' }}</dd>
          </div>
          <div>
            <dt>Sessão</dt>
            <dd>Protegida neste navegador</dd>
          </div>
          <div>
            <dt>Prontidão</dt>
            <dd>{{ lobby.participant.ready ? 'Confirmada' : 'Pendente' }}</dd>
          </div>
        </dl>

        <form
          v-if="!lobby.participant.hero"
          class="hero-selection"
          :aria-busy="roomAccess.status === 'selecting_hero'"
          @submit.prevent="confirmHero()"
        >
          <fieldset>
            <legend>Escolha seu Herói</legend>
            <div class="hero-options">
              <template v-for="hero in lobby.heroes" :key="hero.id">
                <label v-if="heroIsSelectable(hero.id, hero.available)" class="hero-option">
                  <input
                    v-model="selectedHero"
                    :value="hero.id"
                    name="lobby-hero"
                    type="radio"
                  />
                  <span>{{ hero.name }}</span>
                  <small aria-hidden="true">Disponível</small>
                </label>
                <label v-else class="hero-option hero-option--unavailable">
                  <input :disabled="true" :value="hero.id" name="lobby-hero" type="radio" />
                  <span>{{ hero.name }}</span>
                  <small aria-hidden="true">Indisponível</small>
                </label>
              </template>
            </div>
          </fieldset>
          <button
            class="secondary-button"
            :disabled="!selectedHero || lobbyIsBusy()"
            type="submit"
          >
            {{ roomAccess.status === 'selecting_hero' ? 'Confirmando Herói' : 'Confirmar Herói' }}
          </button>
          <p v-if="lobbyError" class="form-error" role="alert">{{ lobbyError }}</p>
        </form>

        <div class="participant-lineup">
          <h3>Participantes</h3>
          <ol>
            <li v-for="participant in lobby.participants" :key="participant.position">
              <span>Posição {{ participant.position }}</span>
              <strong>{{ participant.display_name }}</strong>
              <span>{{ participant.hero?.name ?? 'Herói pendente' }}</span>
              <span :class="participant.ready ? 'ready-label' : 'pending-label'">
                {{ participant.ready ? 'Pronto' : 'Preparando' }}
              </span>
            </li>
          </ol>
        </div>

        <div v-if="isHost" class="content-selection">
          <label for="adventure-selection">Aventura e conteúdo da partida</label>
          <select
            id="adventure-selection"
            v-model="selectedContentKey"
            :disabled="lobbyIsBusy() || Boolean(roomAccess.pendingStartInput)"
          >
            <option value="" disabled>Selecione conteúdo jogável</option>
            <template v-for="choice in adventureChoices" :key="choice.key">
              <option v-if="choice.playable" :value="choice.key">
                {{ choice.adventure.name }} · {{ choice.manifest.ruleset_version }}
              </option>
              <option v-else disabled :value="choice.key">
                {{ choice.adventure.name }} · {{ choice.manifest.ruleset_version }} · não jogável
              </option>
            </template>
          </select>
          <p v-if="selectedContent">
            Manifesto v{{ selectedContent.manifest.manifest_version }} ·
            {{ selectedContent.manifest.content_version }}
          </p>
          <p v-if="roomAccess.pendingStartInput" class="pending-selection-note">
            Escolha preservada para repetir a mesma solicitação com segurança.
          </p>
          <p v-if="!selectedContent" class="content-warning" role="status">
            Nenhum Manifesto jogável está publicado. Lacunas funcionais impedem o selo da sala.
          </p>
        </div>

        <div class="lobby-utilities">
          <button class="text-button" type="button" @click="copyRoomCode()">
            {{ copyResult === 'copied' ? 'Copiar código novamente' : 'Copiar código da sala' }}
          </button>
          <button
            v-if="lobby.participant.ready"
            class="text-button"
            type="button"
            :disabled="lobbyIsBusy()"
            @click="toggleReadiness()"
          >
            Reabrir minha preparação
          </button>
        </div>

        <p v-if="copyResult === 'copied'" class="copy-feedback" role="status">Código copiado.</p>
        <p v-else-if="copyResult === 'failed'" class="copy-feedback copy-feedback--error" role="alert">
          Não foi possível copiar. Selecione o código e copie manualmente.
        </p>
        <p v-if="lobbyError" class="form-error lobby-error" role="alert">{{ lobbyError }}</p>
      </div>
    </section>

    <section
      v-else-if="entryMode === 'recover'"
      class="room-setup"
      aria-labelledby="participation-recovery-heading"
    >
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">2</span>
        <span class="cue-line"></span>
        <span class="cue-label">Retomar posição</span>
      </div>

      <div class="room-stage">
        <p class="service-confirmation" role="status">
          <span class="state-signal" aria-hidden="true"></span>
          Link protegido
        </p>
        <h2 id="participation-recovery-heading" tabindex="-1">
          {{
            recoveryNeedsReplacement
              ? 'Escolha uma sessão para substituir'
              : 'Recupere sua participação'
          }}
        </h2>
        <p class="stage-description">
          {{
            recoveryNeedsReplacement
              ? 'Você já usa duas sessões. Escolha explicitamente qual delas perderá o acesso.'
              : 'O link identifica uma posição sem revelá-la. Confirme a senha da sala para criar a sessão deste dispositivo.'
          }}
        </p>

        <form
          id="recover-participation"
          class="room-form"
          :aria-busy="roomAccess.status === 'recovering_participation'"
          @submit.prevent="recoverParticipation()"
        >
          <fieldset v-if="recoveryNeedsReplacement" class="recovery-session-options">
            <legend>Sessões disponíveis</legend>
            <p class="field-guidance">
              Nada será desconectado até você confirmar a substituição.
            </p>
            <label
              v-for="session in roomAccess.replacementSessions"
              :key="session.id"
              class="recovery-session-option"
            >
              <input
                v-model="selectedReplacementSessionId"
                :aria-label="session.label"
                :value="session.id"
                name="replacement-session"
                type="radio"
              />
              <span>
                <strong>{{ session.label }}</strong>
                <small>Criada em {{ formatSessionCreatedAt(session.created_at) }}</small>
              </span>
            </label>
          </fieldset>
          <div v-else class="field">
            <label for="recovery-room-password">Senha de recuperação da sala</label>
            <div class="password-control">
              <input
                id="recovery-room-password"
                v-model="recoveryPassword"
                :aria-invalid="roomAccess.errorCode === 'RECOVERY_FAILED'"
                :type="passwordVisible ? 'text' : 'password'"
                aria-describedby="participation-recovery-guidance participation-recovery-error"
                autocomplete="current-password"
                maxlength="128"
                name="recovery-room-password"
                required
              />
              <button
                class="password-toggle"
                type="button"
                aria-controls="recovery-room-password"
                @click="togglePassword()"
              >
                {{ passwordVisible ? 'Ocultar senha' : 'Mostrar senha' }}
              </button>
            </div>
          </div>
          <p v-if="!recoveryNeedsReplacement" id="participation-recovery-guidance" class="field-guidance">
            O link isolado e a senha isolada não recuperam nem identificam uma participação.
          </p>
          <p
            v-if="participationRecoveryError"
            id="participation-recovery-error"
            class="form-error"
            role="alert"
          >
            {{ participationRecoveryError }}
          </p>
          <p
            v-if="roomAccess.errorCode === 'RECOVERY_FAILED' && !recoveryNeedsReplacement"
            class="alternate-path"
          >
            Não consegue usar este link?
            <button type="button" @click="leaveParticipationRecovery()">Voltar ao início</button>
          </p>
          <p v-if="recoveryNeedsReplacement" class="alternate-path">
            Prefere manter as duas sessões atuais?
            <button type="button" @click="leaveParticipationRecovery()">
              Não substituir agora
            </button>
          </p>
        </form>
      </div>
    </section>

    <section
      v-else-if="entryMode === 'create'"
      class="room-setup"
      :class="{ 'room-setup--pending': Boolean(roomCreation.pendingIntent) }"
      aria-labelledby="room-setup-heading"
    >
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">2</span>
        <span class="cue-line"></span>
        <span class="cue-label">Abrir a mesa</span>
      </div>

      <div class="room-stage">
        <p class="service-confirmation" role="status">
          <span class="state-signal" aria-hidden="true"></span>
          Servidor pronto
        </p>
        <h2 id="room-setup-heading">Abra uma sala para o seu grupo</h2>
        <p class="stage-description">
          Você será o anfitrião e continuará reconhecido neste navegador, sem criar uma conta.
        </p>

        <form
          id="create-room"
          class="room-form"
          :aria-busy="roomCreation.status === 'submitting'"
          @submit.prevent="createRoom()"
        >
          <div
            v-if="roomCreation.pendingIntent && roomCreation.status !== 'submitting'"
            class="pending-intent"
          >
            <p role="status">Existe uma criação pendente neste navegador.</p>
            <p>
              Retome com o mesmo nome e senha. Descartar inicia outra sala sem excluir a anterior.
            </p>
            <button type="button" @click="discardPendingRequest()">
              Descartar e começar outra
            </button>
          </div>

          <div class="field">
            <label for="display-name">Seu nome</label>
            <input
              id="display-name"
              v-model="displayName"
              :aria-invalid="Boolean(displayNameError)"
              aria-describedby="display-name-error"
              autocomplete="nickname"
              maxlength="40"
              name="display-name"
              :readonly="roomCreation.status === 'submitting' || Boolean(roomCreation.pendingInput)"
              required
              type="text"
            />
            <p id="display-name-error" class="field-error" role="alert">{{ displayNameError }}</p>
          </div>

          <div class="field">
            <label for="recovery-password">Senha de recuperação</label>
            <div class="password-control">
              <input
                id="recovery-password"
                v-model="recoveryPassword"
                :aria-invalid="Boolean(passwordError)"
                :type="passwordVisible ? 'text' : 'password'"
                aria-describedby="password-guidance password-error"
                autocomplete="new-password"
                maxlength="128"
                minlength="12"
                name="recovery-password"
                :readonly="roomCreation.status === 'submitting' || Boolean(roomCreation.pendingInput)"
                required
              />
              <button
                class="password-toggle"
                type="button"
                aria-controls="recovery-password"
                @click="togglePassword()"
              >
                {{ passwordVisible ? 'Ocultar senha' : 'Mostrar senha' }}
              </button>
            </div>
          </div>
          <p id="password-guidance" class="field-guidance">
            Use ao menos 12 caracteres e evite frases previsíveis. A senha não será exibida de novo.
          </p>
          <p id="password-error" class="field-error" role="alert">{{ passwordError }}</p>

          <p v-if="createFormError" class="form-error" role="alert">{{ createFormError }}</p>
          <p class="alternate-path">
            Já recebeu um código?
            <button type="button" @click="showJoin()">Entrar em uma sala</button>
          </p>
        </form>
      </div>
    </section>

    <section v-else class="room-setup" aria-labelledby="join-heading">
      <div class="cue-rail" aria-hidden="true">
        <span class="cue-number">2</span>
        <span class="cue-line"></span>
        <span class="cue-label">Entrar na mesa</span>
      </div>

      <div class="room-stage">
        <p class="service-confirmation" role="status">
          <span class="state-signal" aria-hidden="true"></span>
          Servidor pronto
        </p>
        <h2 id="join-heading">
          {{ roomAccess.roomLookup ? 'Escolha seu lugar à mesa' : 'Entre na sala do grupo' }}
        </h2>
        <p class="stage-description">
          {{
            roomAccess.roomLookup
              ? `Sala ${lookupCode} está aberta. Escolha somente entre os Heróis disponíveis.`
              : 'Use o código compartilhado pelo anfitrião. Ele localiza a sala, mas não recupera uma participação.'
          }}
        </p>

        <form
          v-if="!roomAccess.roomLookup"
          id="find-room"
          class="room-form"
          :aria-busy="roomAccess.status === 'looking_up' || roomAccess.status === 'joining'"
          @submit.prevent="findOrRecoverRoom()"
        >
          <div
            v-if="roomAccess.pendingJoinIntent && roomAccess.status !== 'joining'"
            class="pending-intent"
          >
            <p role="status">Existe uma entrada pendente neste navegador.</p>
            <p>
              Ela pode já ter sido confirmada. Retomar reapresenta a mesma solicitação sem criar
              outro participante.
            </p>
            <button type="button" @click="discardPendingJoin()">
              Descartar entrada e usar outro código
            </button>
          </div>
          <div class="field">
            <label for="room-code">Código da sala</label>
            <input
              id="room-code"
              v-model="roomCode"
              aria-describedby="room-code-guidance join-form-error"
              autocomplete="off"
              inputmode="text"
              maxlength="8"
              minlength="8"
              name="room-code"
              pattern="[23456789A-HJ-NP-Za-hj-np-z]{8}"
              :readonly="Boolean(roomAccess.pendingJoinIntent)"
              required
              spellcheck="false"
              type="text"
            />
            <p id="room-code-guidance" class="field-guidance">
              O código tem oito letras e números.
            </p>
          </div>
          <p v-if="joinFormError" id="join-form-error" class="form-error" role="alert">
            {{ joinFormError }}
          </p>
          <p v-if="!roomAccess.pendingJoinIntent" class="alternate-path">
            Precisa abrir a mesa?
            <button type="button" @click="showCreate()">Criar uma sala</button>
          </p>
        </form>

        <form
          v-else
          id="join-room"
          class="room-form"
          :aria-busy="roomAccess.status === 'joining'"
          @submit.prevent="joinRoom()"
        >
          <div class="field">
            <label for="join-display-name">Seu nome</label>
            <input
              id="join-display-name"
              v-model="displayName"
              :aria-invalid="Boolean(displayNameError)"
              aria-describedby="join-display-name-error"
              autocomplete="nickname"
              maxlength="40"
              name="join-display-name"
              :readonly="roomAccess.status === 'joining' || Boolean(roomAccess.pendingInput)"
              required
              type="text"
            />
            <p id="join-display-name-error" class="field-error" role="alert">
              {{ displayNameError }}
            </p>
          </div>

          <fieldset class="hero-fieldset">
            <legend>Herói</legend>
            <div class="hero-options">
              <template v-for="hero in lookupHeroes" :key="hero.id">
                <label v-if="hero.available" class="hero-option">
                  <input
                    v-model="selectedHero"
                    :disabled="roomAccess.status === 'joining'"
                    :value="hero.id"
                    name="join-hero"
                    required
                    type="radio"
                  />
                  <span>{{ hero.name }}</span>
                  <small aria-hidden="true">Disponível</small>
                </label>
                <label v-else class="hero-option hero-option--unavailable">
                  <input
                    :disabled="true"
                    :value="hero.id"
                    name="join-hero"
                    required
                    type="radio"
                  />
                  <span>{{ hero.name }}</span>
                  <small aria-hidden="true">Indisponível</small>
                </label>
              </template>
            </div>
          </fieldset>
          <p v-if="joinFormError" class="form-error" role="alert">{{ joinFormError }}</p>
          <p v-if="roomAccess.pendingJoinIntent" class="field-guidance">
            A entrada pode já ter sido confirmada. Descarte somente se aceitar abandonar essa
            posição.
          </p>
          <button
            class="text-button"
            :disabled="roomAccess.status === 'joining'"
            type="button"
            @click="roomAccess.pendingJoinIntent ? discardPendingJoin() : roomAccess.clearLookup()"
          >
            {{
              roomAccess.pendingJoinIntent
                ? 'Descartar entrada e usar outro código'
                : 'Usar outro código'
            }}
          </button>
        </form>
      </div>
    </section>

    <footer class="action-dock">
      <button
        v-if="health.availability !== 'ready'"
        class="retry-button"
        type="button"
        :aria-disabled="health.availability === 'checking'"
        @click="retry()"
      >
        {{ health.availability === 'checking' ? 'Verificando servidor' : 'Tentar novamente' }}
      </button>
      <button
        v-else-if="accessInvalidationReason"
        class="primary-button"
        type="button"
        @click="returnToEntry()"
      >
        Voltar ao início
      </button>
      <button
        v-else-if="sessionNeedsRecovery"
        class="retry-button"
        type="button"
        :aria-disabled="roomAccess.status === 'restoring'"
        @click="retrySession()"
      >
        {{ isRestoringSession ? 'Retomando sessão' : 'Tentar retomar sessão' }}
      </button>
      <button
        v-else-if="game && gameCommand.status === 'uncertain'"
        class="primary-button"
        type="button"
        @click="recoverGameCommand()"
      >
        Verificar resultado da ação
      </button>
      <button
        v-else-if="game && gameCommand.status === 'stale'"
        class="primary-button"
        type="button"
        @click="resyncStaleGame()"
      >
        Atualizar estado da partida
      </button>
      <button
        v-else-if="game && gameCommand.status === 'resyncing'"
        class="primary-button"
        :disabled="true"
        type="button"
      >
        Atualizando estado da partida
      </button>
      <button
        v-else-if="game && (gameCommand.status === 'submitting' || gameCommand.status === 'recovering')"
        class="primary-button"
        :disabled="true"
        type="button"
      >
        {{ gameCommand.status === 'recovering' ? 'Consultando recibo' : 'Aguardando confirmação' }}
      </button>
      <button
        v-else-if="game && isSelectableChoiceForParticipant"
        class="primary-button"
        :disabled="!canResolvePendingChoice"
        type="button"
        @click="resolvePendingChoice()"
      >
        Confirmar escolha
      </button>
      <button
        v-else-if="game && endHeroActionsIsLegal && gameSync.commandsFrozen"
        class="primary-button"
        :disabled="true"
        type="button"
      >
        Sincronizando partida
      </button>
      <button
        v-else-if="game && canEndHeroActions"
        class="primary-button"
        type="button"
        @click="endHeroActions()"
      >
        Encerrar ações do Herói
      </button>
      <p
        v-else-if="game && pendingChoice && !isResponsibleForPendingChoice"
        class="continuity-note"
      >
        <span aria-hidden="true"></span>
        Aguardando {{ pendingChoiceResponsibleName }} concluir a escolha.
      </p>
      <p v-else-if="game" class="continuity-note">
        <span aria-hidden="true"></span>
        {{
          game.turn.phase === 'hero_actions'
            ? game.choice.status === 'pending'
              ? 'Aguardando a escolha oficial do participante responsável.'
              : game.legal_actions.length > 0
                ? 'Escolha uma carta, um vilão ou o mercado na mesa acima, ou encerre suas ações.'
                : 'Aguardando as ações do participante ativo.'
            : 'O servidor está resolvendo esta fase automaticamente.'
        }}
      </p>
      <button
        v-else-if="lobby && lobby.participant.hero && !lobby.participant.ready"
        class="primary-button"
        :disabled="lobbyIsBusy()"
        type="button"
        @click="toggleReadiness()"
      >
        {{ roomAccess.status === 'setting_readiness' ? 'Confirmando prontidão' : 'Estou pronto' }}
      </button>
      <button
        v-else-if="lobby && isHost && canStartGame"
        class="primary-button"
        :disabled="lobbyIsBusy()"
        type="button"
        @click="startGame()"
      >
        {{ roomAccess.status === 'starting_game' ? 'Selando sala' : 'Selar sala e iniciar' }}
      </button>
      <button
        v-else-if="lobby && lobby.participant.ready"
        class="primary-button"
        :disabled="lobbyIsBusy()"
        type="button"
        @click="refreshLobby()"
      >
        {{
          roomAccess.status === 'restoring' ? 'Atualizando sala' : 'Atualizar estado da sala'
        }}
      </button>
      <p v-else-if="lobby" class="continuity-note">
        <span aria-hidden="true"></span>
        Escolha um Herói antes de confirmar sua prontidão.
      </p>
      <button
        v-else-if="entryMode === 'recover'"
        class="primary-button"
        :disabled="
          roomAccess.status === 'recovering_participation' ||
          !recoveryPassword ||
          (recoveryNeedsReplacement && !selectedReplacementSession)
        "
        form="recover-participation"
        type="submit"
      >
        {{
          roomAccess.status === 'recovering_participation'
            ? recoveryNeedsReplacement
              ? 'Substituindo sessão'
              : 'Recuperando participação'
            : selectedReplacementSession
              ? `Substituir ${selectedReplacementSession.label}`
              : recoveryNeedsReplacement
                ? 'Escolha uma sessão'
                : 'Recuperar minha posição'
        }}
      </button>
      <button
        v-else-if="entryMode === 'create'"
        class="primary-button"
        :disabled="roomCreation.status === 'submitting'"
        form="create-room"
        type="submit"
      >
        {{ createSubmitLabel }}
      </button>
      <button
        v-else-if="entryMode === 'join' && !roomAccess.roomLookup"
        class="primary-button"
        :disabled="roomAccess.status === 'looking_up' || roomAccess.status === 'joining'"
        form="find-room"
        type="submit"
      >
        {{ joinSubmitLabel }}
      </button>
      <button
        v-else
        class="primary-button"
        :disabled="roomAccess.status === 'joining' || !selectedHero"
        form="join-room"
        type="submit"
      >
        {{ joinSubmitLabel }}
      </button>
    </footer>
  </main>
</template>
