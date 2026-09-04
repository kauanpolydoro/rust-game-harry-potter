<script setup lang="ts">
import { computed } from 'vue'

import type { EffectOutcomeSummary } from '../contracts/identity-access.generated'
import { useGameCommandStore } from '../stores/gameCommand'
import { useGameSyncStore } from '../stores/gameSync'
import { useRoomAccessStore } from '../stores/roomAccess'

const gameCommand = useGameCommandStore()
const gameSync = useGameSyncStore()
const roomAccess = useRoomAccessStore()

const game = computed(() => roomAccess.game)
const activeParticipant = computed(() =>
  game.value?.participants.find(
    (participant) => participant.position === game.value?.turn.active_position,
  ),
)
const currentParticipantPosition = computed(() => game.value?.participant.position)
const requiredParticipant = computed(() =>
  game.value?.participants.find(
    (participant) => participant.position === gameSync.requiredParticipantPosition,
  ),
)
const commandIsBusy = computed(() =>
  ['submitting', 'recovering', 'resyncing'].includes(gameCommand.status),
)
const phaseLabel = computed(() => {
  switch (game.value?.turn.phase) {
    case 'dark_arts':
      return 'Artes das Trevas'
    case 'hero_action':
      return 'Ação do Herói'
    default:
      return game.value?.turn.phase ?? ''
  }
})
const commandError = computed(() => {
  switch (gameCommand.errorCode) {
    case 'STALE_STATE_VERSION':
      return 'O estado oficial avançou. Atualize a partida e decida novamente.'
    case 'GAME_ACTION_NOT_ALLOWED':
      return 'Esta ação não está disponível para você no estado oficial atual.'
    case 'GAME_EXPIRED':
      return 'A partida expirou e não aceita novas ações.'
    case null:
      return null
    default:
      return 'Não foi possível confirmar a ação. Consulte o resultado antes de decidir novamente.'
  }
})
const acceptedCommandSummary = computed(() => {
  const receipt = gameCommand.receipt
  return receipt
    ? `Recibo aceito no estado v${receipt.accepted_state_version}, sequência ${receipt.accepted_sequence}.`
    : ''
})
const realtimeStatus = computed(() => {
  switch (gameSync.status) {
    case 'connected':
      return 'Atualizações em tempo real conectadas.'
    case 'connecting':
      return 'Conectando atualizações em tempo real.'
    case 'reconnecting':
      return 'Reconectando atualizações em tempo real.'
    case 'failed':
      return 'Atualizações automáticas interrompidas.'
    default:
      return 'Atualizações em tempo real desconectadas.'
  }
})

const pendingChoiceOwner = computed(() =>
  game.value?.participants.find(
    (participant) => participant.position === game.value?.choice.responsible_position,
  ),
)

function formatExpiration(value: string): string {
  const expiration = new Date(value)
  if (Number.isNaN(expiration.getTime())) {
    return 'Prazo indisponível'
  }
  return new Intl.DateTimeFormat('pt-BR', {
    dateStyle: 'short',
    timeStyle: 'short',
  }).format(expiration)
}

function presenceLabel(position: number): string {
  switch (gameSync.presenceFor(position)) {
    case 'online':
      return 'Online'
    case 'reconnecting':
      return 'Reconectando'
    case 'offline':
      return 'Offline'
    default:
      return 'Confirmando presença'
  }
}

function participantName(position?: number): string {
  if (position === undefined) {
    return 'A mesa'
  }
  return (
    game.value?.participants.find((participant) => participant.position === position)
      ?.display_name ?? `Posição ${position}`
  )
}

function resourceLabel(resource?: string): string {
  switch (resource) {
    case 'attack':
      return 'Ataque'
    case 'control':
      return 'Controle'
    case 'health':
      return 'Vida'
    case 'influence':
      return 'Influência'
    default:
      return 'recurso'
  }
}

function zoneLabel(zone?: string): string {
  switch (zone) {
    case 'active_location':
      return 'local ativo'
    case 'active_villains':
      return 'vilões ativos'
    case 'dark_arts_deck':
      return 'baralho de Artes das Trevas'
    case 'dark_arts_discard':
      return 'descarte de Artes das Trevas'
    case 'hero_discard_pile':
      return 'descarte do Herói'
    case 'hero_draw_pile':
      return 'baralho do Herói'
    case 'hero_hand':
      return 'mão do Herói'
    case 'hero_play_area':
      return 'área de jogo do Herói'
    case 'heroes':
      return 'Heróis'
    case 'hogwarts_deck':
      return 'baralho de Hogwarts'
    case 'market':
      return 'mercado'
    case 'villain_deck':
      return 'baralho de Vilões'
    default:
      return zone ?? 'zona desconhecida'
  }
}

function choiceOptionLabel(option: string): string {
  const heroPosition = /^hero:(\d+)$/.exec(option)?.[1]
  if (heroPosition !== undefined) {
    return participantName(Number(heroPosition))
  }
  const optionNumber = /^option:(\d+)$/.exec(option)?.[1]
  return optionNumber === undefined ? option : `Opção ${optionNumber}`
}

function effectOutcomeLabel(outcome: EffectOutcomeSummary): string {
  switch (outcome.type) {
    case 'die_rolled':
      return `Dado ${(outcome.die ?? '').toUpperCase()}: resultado ${outcome.result ?? '?'}.`
    case 'moved':
      return `Um alvo de ${participantName(outcome.target_position)} foi movido de ${zoneLabel(outcome.from)} para ${zoneLabel(outcome.to)}.`
    case 'no_op':
      return outcome.reason === 'no_eligible_target'
        ? 'Nenhum alvo era elegível. O efeito foi resolvido sem alterar a mesa.'
        : 'O efeito foi resolvido sem alterar a mesa.'
    case 'resource_changed': {
      const before = outcome.before ?? 0
      const after = outcome.after ?? before
      const difference = after - before
      if (difference === 0) {
        return `${participantName(outcome.target_position)} permaneceu com ${after} de ${resourceLabel(outcome.resource)}.`
      }
      const action = outcome.cause === 'cost' ? 'pagou' : difference >= 0 ? 'recebeu' : 'perdeu'
      return `${participantName(outcome.target_position)} ${action} ${Math.abs(difference)} de ${resourceLabel(outcome.resource)} (${before} → ${after}).`
    }
    case 'terminal':
      return outcome.outcome === 'won'
        ? 'A resolução encerrou a partida com vitória.'
        : 'A resolução encerrou a partida com derrota.'
    default:
      return 'O servidor confirmou um efeito oficial.'
  }
}
</script>

<template>
  <section
    v-if="game"
    class="room-success game-stage"
    aria-labelledby="game-heading"
    :aria-busy="commandIsBusy"
  >
    <div class="cue-rail" aria-hidden="true">
      <span class="cue-number">4</span>
      <span class="cue-line"></span>
      <span class="cue-label">Partida selada</span>
    </div>

    <div class="room-stage room-stage--success">
      <p class="service-confirmation" role="status">
        <span class="state-signal" aria-hidden="true"></span>
        {{ game.snapshot.sequence === 0 ? 'Snapshot inicial confirmado' : 'Estado oficial confirmado' }}
      </p>
      <h2 id="game-heading" tabindex="-1">
        {{ game.snapshot.sequence === 0 ? 'Partida iniciada' : 'Partida em andamento' }}
      </h2>
      <p class="stage-description">
        A sala está selada. Posições, Heróis, aventura e versões permanecem fixos nesta partida.
      </p>

      <div
        class="realtime-status"
        :class="`realtime-status--${gameSync.status}`"
        role="status"
        aria-live="polite"
      >
        <span>{{ realtimeStatus }}</span>
        <button
          v-if="gameSync.status === 'failed'"
          class="text-button"
          type="button"
          @click="gameSync.resynchronize()"
        >
          Reconectar atualizações
        </button>
      </div>

      <div
        v-if="gameSync.gameBlocked && requiredParticipant"
        class="presence-block"
        role="status"
        aria-live="polite"
      >
        <strong>Aguardando {{ requiredParticipant.display_name }}</strong>
        <p>
          Esta decisão continua exclusivamente com esse participante. Não há bot, timeout ou pulo
          automático.
        </p>
      </div>

      <div
        v-if="commandIsBusy"
        class="command-feedback command-feedback--pending"
        role="status"
        aria-live="polite"
      >
        <strong>
          {{ gameCommand.status === 'resyncing' ? 'Atualizando estado oficial' : 'Intenção pendente' }}
        </strong>
        <p>
          {{
            gameCommand.status === 'resyncing'
              ? 'O último Snapshot confirmado permanece visível até a nova projeção chegar.'
              : gameCommand.status === 'recovering'
                ? 'Consultando o recibo persistido. O estado abaixo continua sendo a última versão oficial.'
                : 'A solicitação foi enviada. Nada muda na mesa até o servidor concluir o commit.'
          }}
        </p>
      </div>
      <div
        v-else-if="gameCommand.status === 'uncertain'"
        class="command-feedback command-feedback--warning"
        role="alert"
      >
        <strong>Confirmação ainda desconhecida</strong>
        <p>A conexão terminou sem resposta. Consulte o mesmo comando antes de tomar outra decisão.</p>
      </div>
      <div
        v-else-if="gameCommand.status === 'resynced'"
        class="command-feedback command-feedback--accepted"
        role="status"
        aria-live="polite"
      >
        <strong>Estado oficial atualizado</strong>
        <p>
          Outro comando avançou esta partida. Revise o Snapshot recebido antes de decidir uma nova
          ação.
        </p>
      </div>
      <div
        v-else-if="gameCommand.status === 'stale'"
        class="command-feedback command-feedback--warning"
        role="alert"
      >
        <strong>Estado oficial desatualizado</strong>
        <p>Outro comando avançou esta partida, mas o Snapshot atualizado ainda não foi recebido.</p>
      </div>
      <div
        v-else-if="gameCommand.status === 'accepted' && acceptedCommandSummary"
        class="command-feedback command-feedback--accepted"
        role="status"
        aria-live="polite"
      >
        <strong>Ação oficial</strong>
        <p>{{ acceptedCommandSummary }}</p>
      </div>
      <div v-else-if="gameCommand.status === 'not_committed'" class="command-feedback" role="status">
        <strong>Nenhum aceite encontrado</strong>
        <p>A intenção anterior não foi oficializada. Revise a mesa e decida novamente.</p>
      </div>
      <div
        v-else-if="gameCommand.status === 'failed' && commandError"
        class="command-feedback command-feedback--warning"
        role="alert"
      >
        <strong>Ação não aceita</strong>
        <p>{{ commandError }}</p>
      </div>

      <dl class="game-situation">
        <div>
          <dt>Turno</dt>
          <dd>{{ game.turn.number }}</dd>
        </div>
        <div>
          <dt>Fase</dt>
          <dd>{{ phaseLabel }}</dd>
        </div>
        <div>
          <dt>Participante ativo</dt>
          <dd>{{ activeParticipant?.display_name ?? `Posição ${game.turn.active_position}` }}</dd>
        </div>
        <div>
          <dt>Aventura</dt>
          <dd>{{ game.game.adventure.name }}</dd>
        </div>
        <div>
          <dt>Retenção até</dt>
          <dd>{{ formatExpiration(game.game.expires_at) }}</dd>
        </div>
      </dl>

      <section
        v-if="game.choice.status === 'pending'"
        class="effect-choice"
        aria-labelledby="effect-choice-heading"
      >
        <h3 id="effect-choice-heading">Escolha oficial pendente</h3>
        <p>
          {{ pendingChoiceOwner?.display_name ?? 'O participante responsável' }} precisa escolher
          {{
            (game.choice.min ?? 1) === (game.choice.max ?? 1)
              ? (game.choice.min ?? 1)
              : `${game.choice.min ?? 1} a ${game.choice.max ?? 1}`
          }}
          entre as opções elegíveis.
        </p>
        <ol>
          <li v-for="option in game.choice.options" :key="option">{{ choiceOptionLabel(option) }}</li>
        </ol>
      </section>

      <section
        v-if="game.effects.outcomes.length > 0"
        class="effect-history"
        aria-labelledby="effect-history-heading"
        aria-live="polite"
      >
        <h3 id="effect-history-heading">Última resolução oficial</h3>
        <ol>
          <li v-for="(outcome, index) in game.effects.outcomes" :key="`${outcome.rule_id}:${index}`">
            {{ effectOutcomeLabel(outcome) }}
          </li>
        </ol>
      </section>

      <div class="participant-lineup">
        <h3>Posições seladas</h3>
        <ol>
          <li v-for="participant in game.participants" :key="participant.position">
            <span>Posição {{ participant.position }}</span>
            <strong>{{ participant.display_name }}</strong>
            <span>{{ participant.hero.name }}</span>
            <span
              class="presence-label"
              :class="`presence-label--${gameSync.presenceFor(participant.position) ?? 'unknown'}`"
            >
              {{ presenceLabel(participant.position) }}
              <template v-if="participant.position === currentParticipantPosition"> · Você</template>
            </span>
            <span class="resource-line">
              Vida {{ participant.resources.health }} · Ataque {{ participant.resources.attack }} ·
              Influência {{ participant.resources.influence }}
            </span>
          </li>
        </ol>
      </div>

      <details class="snapshot-details">
        <summary>Ver versões do Snapshot</summary>
        <dl>
          <div>
            <dt>Estado</dt>
            <dd>v{{ game.snapshot.state_version }} · sequência {{ game.snapshot.sequence }}</dd>
          </div>
          <div>
            <dt>Ruleset</dt>
            <dd>{{ game.snapshot.versions.ruleset }}</dd>
          </div>
          <div>
            <dt>Manifesto</dt>
            <dd>v{{ game.snapshot.versions.manifest }}</dd>
          </div>
          <div>
            <dt>Digest</dt>
            <dd class="digest-value">{{ game.snapshot.digest }}</dd>
          </div>
          <div>
            <dt>PRNG</dt>
            <dd>{{ game.snapshot.versions.prng }}</dd>
          </div>
        </dl>
      </details>
      <p class="seed-note">A seed permanece secreta enquanto a partida estiver em andamento.</p>
    </div>
  </section>
</template>
