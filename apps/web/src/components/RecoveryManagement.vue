<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'

import { isUncertainTransportFailure } from '../api/http'
import { useRecoveryManagementStore } from '../stores/recoveryManagement'
import { useRoomAccessStore } from '../stores/roomAccess'
import { useSecuritySyncStore } from '../stores/securitySync'
import RecoveryCredential from './RecoveryCredential.vue'

interface RecoveryParticipantView {
  display_name: string
  position: number
  role: 'host' | 'guest'
}

const props = defineProps<{
  participant: RecoveryParticipantView
  participants: RecoveryParticipantView[]
}>()

const recovery = useRecoveryManagementStore()
const roomAccess = useRoomAccessStore()
const securitySync = useSecuritySyncStore()
const currentPassword = ref('')
const newPassword = ref('')
const newPasswordConfirmation = ref('')
const newPasswordsVisible = ref(false)
const currentPasswordInput = ref<HTMLInputElement | null>(null)
const newPasswordInput = ref<HTMLInputElement | null>(null)
const issuedCredentialHeading = ref<HTMLHeadingElement | null>(null)
const assistedPosition = ref<number | ''>('')
const riskAcknowledged = ref(false)
const copyResult = ref<'idle' | 'copied' | 'failed'>('idle')
let clearingPasswordForm = false

const isHost = computed(() => props.participant.role === 'host')
const assistedParticipants = computed(() =>
  props.participants.filter((participant) => participant.position !== props.participant.position),
)
const issuedToken = computed(
  () => recovery.issuedCredential?.recovery_token ?? roomAccess.issuedRecoveryToken,
)
const issuedParticipantName = computed(
  () => recovery.issuedCredential?.participant.display_name ?? props.participant.display_name,
)
const issuedRecoveryLink = computed(() =>
  issuedToken.value
    ? `${window.location.origin}${window.location.pathname}#recovery=${issuedToken.value}`
    : null,
)
const isBusy = computed(() =>
  [
    'rotating_password',
    'regenerating_directly',
    'regenerating_with_assistance',
  ].includes(recovery.status),
)
const hasUndismissedGeneratedCredential = computed(
  () => recovery.issuedCredential !== null,
)
const currentPasswordError = computed(() =>
  recovery.errorCode === 'RECOVERY_CONFIRMATION_FAILED'
    ? 'A senha atual não confere. Revise-a antes de tentar novamente.'
    : null,
)
const newPasswordError = computed(() =>
  recovery.errorCode === 'WEAK_RECOVERY_PASSWORD'
    ? 'Escolha uma nova senha com ao menos 12 caracteres e menos previsível.'
    : null,
)
const passwordConfirmationError = computed(() =>
  newPasswordConfirmation.value && newPasswordConfirmation.value !== newPassword.value
    ? 'As novas senhas não coincidem.'
    : null,
)
const canRotatePassword = computed(
  () =>
    !isBusy.value &&
    currentPassword.value.length > 0 &&
    newPassword.value.length >= 12 &&
    newPasswordConfirmation.value === newPassword.value,
)
const managementError = computed(() => {
  if (isUncertainTransportFailure(recovery.errorCode)) {
    return 'A confirmação não chegou. Repita a mesma ação sem alterar os dados.'
  }
  switch (recovery.errorCode) {
    case 'RECOVERY_CONFIRMATION_FAILED':
    case 'WEAK_RECOVERY_PASSWORD':
      return null
    case 'ROOM_PARTICIPANT_NOT_FOUND':
      return 'Esse participante não está mais disponível nesta sala.'
    case 'HOST_ASSISTANCE_RISK_NOT_ACKNOWLEDGED':
      return 'Confirme o risco de personificação antes de gerar o link assistido.'
    case 'RECOVERY_ASSISTANCE_NOT_REQUIRED':
      return 'Use a entrega direta para renovar o link da sua própria posição.'
    case 'RECOVERY_CREDENTIAL_SUPERSEDED':
      return 'Outro link foi emitido antes da resposta chegar. Use somente o link mais recente.'
    case null:
      return null
    default:
      return 'Não foi possível concluir a gestão de recuperação. Tente novamente.'
  }
})
const securityNotice = computed(() => {
  const notice = securitySync.latestNotice
  if (!notice) {
    return null
  }
  if (notice.type === 'recovery_password_rotated') {
    return 'A senha de recuperação foi alterada. Suas sessões continuam ativas.'
  }
  if (notice.delivery === 'direct' && notice.target_position === props.participant.position) {
    return 'Seu link de recuperação foi renovado. As cópias anteriores não funcionam mais.'
  }
  if (notice.target_position === props.participant.position) {
    return 'O Anfitrião gerou um novo link para sua posição. Quem recebeu o link e conhece a senha pode assumir sua participação.'
  }
  return `Novo link assistido emitido para a posição ${notice.target_position}.`
})

async function regenerateOwnCredential(): Promise<void> {
  const regenerated = await recovery.regenerateOwnCredential()
  if (regenerated) {
    roomAccess.dismissRecoveryCredential()
  }
  copyResult.value = 'idle'
  if (regenerated) {
    await nextTick()
    issuedCredentialHeading.value?.focus()
  }
}

async function rotatePassword(): Promise<void> {
  if (!canRotatePassword.value) {
    return
  }
  const rotated = await recovery.rotatePassword({
    current_recovery_password: currentPassword.value,
    new_recovery_password: newPassword.value,
  })
  if (rotated) {
    clearingPasswordForm = true
    currentPassword.value = ''
    newPassword.value = ''
    newPasswordConfirmation.value = ''
    newPasswordsVisible.value = false
    roomAccess.dismissRecoveryCredential()
    await nextTick()
    clearingPasswordForm = false
    return
  }
  await nextTick()
  if (recovery.errorCode === 'RECOVERY_CONFIRMATION_FAILED') {
    currentPasswordInput.value?.focus()
  } else if (recovery.errorCode === 'WEAK_RECOVERY_PASSWORD') {
    newPasswordInput.value?.focus()
  }
}

async function regenerateWithAssistance(): Promise<void> {
  if (assistedPosition.value === '' || !riskAcknowledged.value) {
    return
  }
  const regenerated = await recovery.regenerateAssistedCredential(assistedPosition.value)
  if (regenerated) {
    assistedPosition.value = ''
    riskAcknowledged.value = false
    copyResult.value = 'idle'
    await nextTick()
    issuedCredentialHeading.value?.focus()
  }
}

async function copyRecoveryLink(): Promise<void> {
  if (!issuedRecoveryLink.value) {
    return
  }
  try {
    await navigator.clipboard.writeText(issuedRecoveryLink.value)
    copyResult.value = 'copied'
  } catch {
    copyResult.value = 'failed'
  }
}

function dismissRecoveryLink(): void {
  if (recovery.issuedCredential) {
    recovery.dismissIssuedCredential()
  } else {
    roomAccess.dismissRecoveryCredential()
  }
  copyResult.value = 'idle'
}

watch(
  [
    currentPassword,
    newPassword,
    newPasswordConfirmation,
    assistedPosition,
    riskAcknowledged,
  ],
  () => {
    if (!clearingPasswordForm) {
      recovery.resetPendingOperation()
    }
  },
)
watch(
  () => securitySync.cursor,
  (currentCursor, previousCursor) => {
    let credentialWasCleared = false
    if (
      roomAccess.issuedRecoveryToken &&
      securitySync.credentialWasSuperseded(props.participant.position, 0)
    ) {
      roomAccess.dismissRecoveryCredential()
      credentialWasCleared = true
    }
    const issuedCredential = recovery.issuedCredential
    if (
      issuedCredential &&
      ((previousCursor !== undefined &&
        currentCursor < previousCursor &&
        issuedCredential.security_event.cursor > currentCursor) ||
        securitySync.credentialWasSuperseded(
          issuedCredential.participant.position,
          issuedCredential.security_event.cursor,
        ))
    ) {
      recovery.dismissIssuedCredential()
      credentialWasCleared = true
    }
    if (credentialWasCleared) {
      copyResult.value = 'idle'
    }
  },
  { flush: 'sync', immediate: true },
)
</script>

<template>
  <div class="recovery-tools">
    <div
      v-if="securitySync.status === 'failed' || securitySync.status === 'reconnecting'"
      class="security-notice security-notice--warning"
    >
      <p role="status" aria-live="polite">
        Os avisos de segurança estão desconectados. As ações continuam protegidas pelo servidor.
      </p>
      <button class="text-button" type="button" @click="securitySync.retry()">
        Reconectar avisos
      </button>
    </div>
    <div v-if="securityNotice" class="security-notice">
      <p role="status" aria-live="polite">{{ securityNotice }}</p>
      <button class="text-button" type="button" @click="securitySync.dismissLatestNotice()">
        Dispensar aviso
      </button>
    </div>

    <RecoveryCredential
      v-if="roomAccess.issuedRecoveryToken && !recovery.issuedCredential"
      :token="roomAccess.issuedRecoveryToken"
      @dismiss="roomAccess.dismissRecoveryCredential()"
    />

    <section
      v-else-if="issuedRecoveryLink"
      class="recovery-credential"
      aria-labelledby="recovery-credential-heading"
    >
      <h3
        id="recovery-credential-heading"
        ref="issuedCredentialHeading"
        tabindex="-1"
      >
        {{
          recovery.issuedCredential
            ? `Novo link emitido para ${issuedParticipantName}.`
            : 'Guarde seu link individual'
        }}
      </h3>
      <p
        v-if="recovery.issuedCredential"
        class="management-confirmation"
        role="status"
      >
        Novo link confirmado. As cópias anteriores não funcionam mais.
      </p>
      <p id="recovery-credential-guidance">
        Ele recupera somente a posição indicada e exige também a senha da sala. Ao fechar este
        aviso, o link deixa de ser exibido.
      </p>
      <label for="issued-recovery-link">Link de recuperação</label>
      <textarea
        id="issued-recovery-link"
        aria-describedby="recovery-credential-guidance"
        readonly
        rows="3"
        :value="issuedRecoveryLink"
      ></textarea>
      <div class="recovery-credential-actions">
        <button class="text-button" type="button" @click="copyRecoveryLink()">
          {{ copyResult === 'copied' ? 'Copiar link novamente' : 'Copiar link' }}
        </button>
        <button class="text-button" type="button" @click="dismissRecoveryLink()">
          Já guardei o link
        </button>
      </div>
      <p v-if="copyResult === 'copied'" class="copy-feedback" role="status">
        Link individual copiado.
      </p>
      <p
        v-else-if="copyResult === 'failed'"
        class="copy-feedback copy-feedback--error"
        role="alert"
      >
        Não foi possível copiar. Selecione o link e copie manualmente.
      </p>
    </section>

    <details class="recovery-management">
      <summary>Gerenciar recuperação</summary>
      <div class="recovery-management__body" :aria-busy="isBusy">
        <p
          v-if="hasUndismissedGeneratedCredential"
          class="management-guidance"
          role="status"
        >
          Guarde ou dispense o novo link exibido antes de emitir outra credencial.
        </p>
        <div class="recovery-direct">
          <h3>Renove seu próprio acesso</h3>
          <p>
            A entrega direta reduz a exposição. O link anterior deixa de funcionar assim que o
            novo for confirmado.
          </p>
          <button
            class="secondary-button"
            :disabled="isBusy || hasUndismissedGeneratedCredential"
            type="button"
            @click="regenerateOwnCredential()"
          >
            {{
              recovery.status === 'regenerating_directly'
                ? 'Gerando novo link'
                : 'Gerar novo link para mim'
            }}
          </button>
        </div>

        <form
          v-if="isHost"
          class="recovery-password-form"
          @submit.prevent="rotatePassword()"
        >
          <h3>Alterar senha da sala</h3>
          <p>
            Confirme a senha atual neste gesto. As sessões autenticadas continuam ativas, mas todos
            os links anteriores deixam de funcionar.
          </p>
          <div class="field">
            <label for="current-room-recovery-password">Senha atual da sala</label>
            <input
              id="current-room-recovery-password"
              ref="currentPasswordInput"
              v-model="currentPassword"
              :aria-describedby="currentPasswordError ? 'current-room-recovery-password-error' : undefined"
              :aria-invalid="currentPasswordError ? 'true' : undefined"
              autocomplete="current-password"
              :disabled="isBusy"
              maxlength="128"
              required
              type="password"
            />
            <p
              v-if="currentPasswordError"
              id="current-room-recovery-password-error"
              class="field-error"
              role="alert"
            >
              {{ currentPasswordError }}
            </p>
          </div>
          <div class="field">
            <label for="new-room-recovery-password">Nova senha de recuperação</label>
            <input
              id="new-room-recovery-password"
              ref="newPasswordInput"
              v-model="newPassword"
              :aria-describedby="
                newPasswordError
                  ? 'new-room-recovery-password-guidance new-room-recovery-password-error'
                  : 'new-room-recovery-password-guidance'
              "
              :aria-invalid="newPasswordError ? 'true' : undefined"
              autocomplete="new-password"
              :disabled="isBusy"
              maxlength="128"
              minlength="12"
              required
              :type="newPasswordsVisible ? 'text' : 'password'"
            />
            <p id="new-room-recovery-password-guidance" class="field-guidance">
              Use ao menos 12 caracteres e evite frases previsíveis.
            </p>
            <p
              v-if="newPasswordError"
              id="new-room-recovery-password-error"
              class="field-error"
              role="alert"
            >
              {{ newPasswordError }}
            </p>
          </div>
          <div class="field">
            <label for="new-room-recovery-password-confirmation">Confirmar nova senha</label>
            <input
              id="new-room-recovery-password-confirmation"
              v-model="newPasswordConfirmation"
              :aria-describedby="
                passwordConfirmationError
                  ? 'new-room-recovery-password-confirmation-error'
                  : undefined
              "
              :aria-invalid="passwordConfirmationError ? 'true' : undefined"
              autocomplete="new-password"
              :disabled="isBusy"
              maxlength="128"
              required
              :type="newPasswordsVisible ? 'text' : 'password'"
            />
            <p
              v-if="passwordConfirmationError"
              id="new-room-recovery-password-confirmation-error"
              class="field-error"
              role="alert"
            >
              {{ passwordConfirmationError }}
            </p>
          </div>
          <button
            class="password-toggle password-visibility-button"
            type="button"
            aria-controls="new-room-recovery-password new-room-recovery-password-confirmation"
            :aria-pressed="newPasswordsVisible"
            @click="newPasswordsVisible = !newPasswordsVisible"
          >
            {{ newPasswordsVisible ? 'Ocultar novas senhas' : 'Mostrar novas senhas' }}
          </button>
          <button class="secondary-button" :disabled="!canRotatePassword" type="submit">
            {{
              recovery.status === 'rotating_password'
                ? 'Alterando senha da sala'
                : 'Alterar senha da sala'
            }}
          </button>
        </form>

        <details v-if="isHost && assistedParticipants.length > 0" class="assisted-recovery">
          <summary>Ajudar participante sem acesso</summary>
          <form @submit.prevent="regenerateWithAssistance()">
            <strong>Risco de personificação</strong>
            <p>
              Quem receber o novo link e souber a senha da sala poderá assumir a participação
              escolhida.
            </p>
            <div class="field">
              <label for="assisted-participant">Participante sem acesso</label>
              <select
                id="assisted-participant"
                v-model.number="assistedPosition"
                :disabled="isBusy"
                required
              >
                <option disabled value="">Escolha uma posição</option>
                <option
                  v-for="candidate in assistedParticipants"
                  :key="candidate.position"
                  :value="candidate.position"
                >
                  Posição {{ candidate.position }} · {{ candidate.display_name }}
                </option>
              </select>
            </div>
            <label class="risk-acknowledgement">
              <input v-model="riskAcknowledged" :disabled="isBusy" required type="checkbox" />
              <span>Entendo que o link permite personificar este participante</span>
            </label>
            <button
              class="secondary-button"
              :disabled="
                isBusy ||
                hasUndismissedGeneratedCredential ||
                assistedPosition === '' ||
                !riskAcknowledged
              "
              type="submit"
            >
              {{
                recovery.status === 'regenerating_with_assistance'
                  ? 'Gerando link com assistência'
                  : 'Gerar link com assistência'
              }}
            </button>
          </form>
        </details>

        <p
          v-if="recovery.confirmation === 'password_rotated'"
          class="management-confirmation"
          role="status"
        >
          Senha da sala alterada.
        </p>
        <p v-if="managementError" class="form-error" role="alert">{{ managementError }}</p>
      </div>
    </details>
  </div>
</template>
