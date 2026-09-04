import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/vue'
import { createPinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import App from './App.vue'
import { useGameCommandStore } from './stores/gameCommand'
import { useRoomAccessStore } from './stores/roomAccess'

class SynchronizedWebSocket {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSED = 3
  static instances: SynchronizedWebSocket[] = []

  readonly url: string
  readonly requestedProtocol: string
  protocol = ''
  readyState = SynchronizedWebSocket.CONNECTING
  onopen: (() => void) | null = null
  onmessage: ((event: { data: string }) => void) | null = null
  onerror: (() => void) | null = null
  onclose: ((event: { code: number }) => void) | null = null

  constructor(url: string | URL, protocol: string | string[]) {
    this.url = String(url)
    this.requestedProtocol = Array.isArray(protocol) ? (protocol[0] ?? '') : protocol
    SynchronizedWebSocket.instances.push(this)
    queueMicrotask(() => {
      this.readyState = SynchronizedWebSocket.OPEN
      this.protocol = this.requestedProtocol
      this.onopen?.()
      const request = new URL(this.url)
      this.onmessage?.({
        data: JSON.stringify({
          cursor: Number(request.searchParams.get('cursor')),
          digest: request.searchParams.get('digest'),
          protocol_version: 2,
          snapshot_version: Number(request.searchParams.get('snapshot_version')),
          type: 'synchronized',
        }),
      })
    })
  }

  receive(message: unknown): void {
    this.onmessage?.({ data: JSON.stringify(message) })
  }

  close(): void {
    this.readyState = SynchronizedWebSocket.CLOSED
    this.onclose?.({ code: 1000 })
  }
}

const availableHeroes = [
  { available: true, id: 'harry', name: 'Harry' },
  { available: true, id: 'hermione', name: 'Hermione' },
  { available: true, id: 'neville', name: 'Neville' },
  { available: true, id: 'ron', name: 'Ron' },
]

const contentOptions = [
  {
    adventures: [{ id: 'adventure:001', name: 'Game 1', playable: false }],
    content_version: 'base-en-candidate-2026-09-02',
    manifest_digest: `blake3:${'a'.repeat(64)}`,
    manifest_version: 1,
    playable: false,
    ruleset_version: 'base-candidate-v1',
  },
]

const playableContentOptions = [
  {
    adventures: [{ id: 'adventure:001', name: 'Game 1', playable: true }],
    content_version: 'fixture-v1',
    manifest_digest: `blake3:${'b'.repeat(64)}`,
    manifest_version: 1,
    playable: true,
    ruleset_version: 'fixture-rules-v1',
  },
]

function readyHostLobbyResponse() {
  const host = {
    display_name: 'Minerva',
    hero: { id: 'harry', name: 'Harry' },
    position: 1,
    ready: true,
    role: 'host',
  }
  return {
    content_options: playableContentOptions,
    heroes: availableHeroes.map((hero) => ({ ...hero, available: false })),
    participant: host,
    participants: [
      host,
      {
        display_name: 'Luna',
        hero: { id: 'hermione', name: 'Hermione' },
        position: 2,
        ready: true,
        role: 'guest',
      },
    ],
    room: { code: '9HKGW4RT', status: 'open' },
  }
}

function unreadyHostLobbyResponse() {
  const lobby = readyHostLobbyResponse()
  const host = { ...lobby.participant, ready: false }
  return {
    ...lobby,
    participant: host,
    participants: [host, lobby.participants[1]],
  }
}

function gameProjectionResponse() {
  return {
    choice: { status: 'none' },
    effects: { outcomes: [], status: 'idle' },
    game: {
      adventure: { id: 'adventure:001', name: 'Game 1' },
      expires_at: '2026-09-10T12:00:00Z',
      id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
      status: 'in_progress',
    },
    legal_actions: ['complete_dark_arts'],
    participant: {
      display_name: 'Minerva',
      hero: { id: 'harry', name: 'Harry' },
      position: 1,
      resources: { attack: 0, health: 10, influence: 0 },
      role: 'host',
    },
    participants: [
      {
        display_name: 'Minerva',
        hero: { id: 'harry', name: 'Harry' },
        position: 1,
        resources: { attack: 0, health: 10, influence: 0 },
        role: 'host',
      },
      {
        display_name: 'Luna',
        hero: { id: 'hermione', name: 'Hermione' },
        position: 2,
        resources: { attack: 0, health: 10, influence: 0 },
        role: 'guest',
      },
    ],
    snapshot: {
      cursor: 0,
      digest: `blake3:${'c'.repeat(64)}`,
      sequence: 0,
      snapshot_version: 1,
      state_version: 1,
      versions: {
        content: 'fixture-v1',
        manifest: 1,
        manifest_digest: `blake3:${'b'.repeat(64)}`,
        prng: 'chacha20-v1',
        ruleset: 'fixture-rules-v1',
        sampling: 'rejection-sampling-v1',
        shuffle: 'fisher-yates-v1',
      },
    },
    turn: { active_position: 1, number: 1, phase: 'dark_arts' },
  }
}

function completedGameProjectionResponse() {
  const projection = gameProjectionResponse()
  const participants = projection.participants.map((participant) =>
    participant.position === 1
      ? { ...participant, resources: { attack: 2, health: 9, influence: 2 } }
      : participant,
  )
  return {
    ...projection,
    effects: {
      outcomes: [
        {
          after: 9,
          before: 10,
          cause: 'cost',
          resource: 'health',
          rule_id: 'rule:functional',
          target_id: 'hero:1',
          target_position: 1,
          type: 'resource_changed',
        },
        {
          after: 2,
          before: 0,
          cause: 'effect',
          resource: 'influence',
          rule_id: 'rule:functional',
          target_id: 'hero:1',
          target_position: 1,
          type: 'resource_changed',
        },
        {
          die: 'd4',
          result: 3,
          rule_id: 'rule:functional',
          type: 'die_rolled',
        },
        {
          reason: 'no_eligible_target',
          rule_id: 'rule:functional',
          type: 'no_op',
        },
      ],
      status: 'resolved',
    },
    game: { ...projection.game, expires_at: '2026-09-10T13:00:00Z' },
    legal_actions: [],
    participant: participants[0],
    participants,
    snapshot: {
      ...projection.snapshot,
      cursor: 1,
      digest: `blake3:${'d'.repeat(64)}`,
      sequence: 1,
      state_version: 2,
    },
    turn: { ...projection.turn, phase: 'hero_action' },
  }
}

const pendingChoiceId = 'rule:functional:target:0'

function pendingChoiceProjectionResponse() {
  const projection = gameProjectionResponse()
  const responsibleParticipant = projection.participants[1]
  if (!responsibleParticipant) {
    throw new Error('the choice fixture requires a second participant')
  }
  return {
    ...projection,
    choice: {
      cause: 'rule:functional',
      id: pendingChoiceId,
      kind: 'target',
      max: 1,
      min: 1,
      options: ['hero:1', 'hero:2'],
      responsible_position: 2,
      status: 'pending',
    },
    effects: { outcomes: [], status: 'choice' },
    legal_actions: ['resolve_choice'],
    participant: responsibleParticipant,
    snapshot: {
      ...projection.snapshot,
      cursor: 1,
      digest: `blake3:${'e'.repeat(64)}`,
      sequence: 1,
      state_version: 2,
    },
  }
}

function multiplePendingChoiceProjectionResponse() {
  const projection = pendingChoiceProjectionResponse()
  return {
    ...projection,
    choice: {
      ...projection.choice,
      max: 2,
      options: ['hero:1', 'hero:2', 'hero:3'],
    },
    participants: [
      ...projection.participants,
      {
        display_name: 'Pomona',
        hero: { id: 'neville', name: 'Neville' },
        position: 3,
        resources: { attack: 0, health: 10, influence: 0 },
        role: 'guest',
      },
    ],
  }
}

function acceptedChoiceResponse(commandId: string) {
  const projection = pendingChoiceProjectionResponse()
  return {
    projection: {
      ...projection,
      choice: { status: 'none' },
      effects: { outcomes: [], status: 'resolved' },
      legal_actions: [],
      snapshot: {
        ...projection.snapshot,
        cursor: 2,
        digest: `blake3:${'f'.repeat(64)}`,
        sequence: 2,
        state_version: 3,
      },
      turn: { ...projection.turn, phase: 'hero_action' },
    },
    receipt: {
      accepted_sequence: 2,
      accepted_state_version: 3,
      command_id: commandId,
      expected_state_version: 2,
      expires_at: '2026-09-10T13:00:00Z',
      status: 'accepted',
      type: 'resolve_choice',
    },
  }
}

function acceptedCommandResponse(commandId: string) {
  return {
    projection: completedGameProjectionResponse(),
    receipt: {
      accepted_sequence: 1,
      accepted_state_version: 2,
      command_id: commandId,
      expected_state_version: 1,
      expires_at: '2026-09-10T13:00:00Z',
      status: 'accepted',
      type: 'complete_dark_arts',
    },
  }
}

function recoveredGuestLobbyResponse() {
  return {
    kind: 'lobby',
    lobby: guestLobbyResponse(),
    recovery_token: '8'.repeat(64),
  }
}

function errorResponse(code: string) {
  return {
    error: {
      category: 'request',
      code,
      correlation_id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
      details: {},
      message_key: `error.${code.toLowerCase()}`,
      retry: 'not_retryable',
    },
  }
}

function hostRoomResponse() {
  const participant = { display_name: 'Minerva', position: 1, ready: false, role: 'host' }
  return {
    content_options: contentOptions,
    heroes: availableHeroes,
    participant,
    participants: [participant],
    recovery_token: 'a'.repeat(64),
    room: { code: '9HKGW4RT', status: 'open' },
  }
}

function guestLobbyResponse() {
  const guest = {
    display_name: 'Luna',
    hero: { id: 'hermione', name: 'Hermione' },
    position: 2,
    ready: false,
    role: 'guest',
  }
  return {
    content_options: contentOptions,
    heroes: availableHeroes.map((hero) => ({
      ...hero,
      available: hero.id !== 'hermione',
    })),
    participant: guest,
    participants: [
      { display_name: 'Minerva', position: 1, ready: false, role: 'host' },
      guest,
    ],
    room: { code: '9HKGW4RT', status: 'open' },
  }
}

function guestJoinResponse() {
  return { ...guestLobbyResponse(), recovery_token: 'b'.repeat(64) }
}

describe('application shell', () => {
  beforeEach(() => {
    SynchronizedWebSocket.instances = []
    vi.stubGlobal('WebSocket', SynchronizedWebSocket)
  })

  afterEach(() => {
    cleanup()
    delete window.__HOGWARTS_RECOVERY_TOKEN__
    localStorage.clear()
    sessionStorage.clear()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('announces when the authoritative service is ready', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      ),
    )

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByText('Servidor pronto')
    expect(screen.getByRole('status')).toHaveTextContent('Servidor pronto')
    expect(
      screen.getByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' }),
    ).toBeVisible()
  })

  it('shows an unavailable state and lets the player retry', async () => {
    const request = vi
      .fn()
      .mockRejectedValueOnce(new TypeError('network unavailable'))
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByText('Servidor indisponível')
    await fireEvent.click(screen.getByRole('button', { name: 'Tentar novamente' }))

    await screen.findByText('Servidor pronto')
    expect(request).toHaveBeenCalledTimes(2)
  })

  it('keeps the retry control focused while checking again', async () => {
    let completeRetry = (_response: Response): void => undefined
    const retryResponse = new Promise<Response>((resolve) => {
      completeRetry = resolve
    })
    vi.stubGlobal(
      'fetch',
      vi.fn().mockRejectedValueOnce(new TypeError('network unavailable')).mockReturnValueOnce(retryResponse),
    )

    render(App, { global: { plugins: [createPinia()] } })

    const retry = await screen.findByRole('button', { name: 'Tentar novamente' })
    retry.focus()
    await fireEvent.click(retry)

    expect(retry).toHaveFocus()
    expect(retry).toHaveAttribute('aria-disabled', 'true')
    expect(retry).toHaveTextContent('Verificando servidor')

    completeRetry(
      new Response(JSON.stringify({ status: 'ready' }), {
        headers: { 'Content-Type': 'application/json' },
        status: 200,
      }),
    )
    await screen.findByText('Servidor pronto')
  })

  it('creates a private room and shows the committed room code', async () => {
    const clipboard = { writeText: vi.fn().mockResolvedValue(undefined) }
    vi.stubGlobal('navigator', Object.assign(Object.create(navigator), { clipboard }))
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify(hostRoomResponse()),
          {
            headers: { 'Content-Type': 'application/json' },
            status: 201,
          },
        ),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' })
    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Minerva')
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Criar sala privada' }))

    const successHeading = await screen.findByRole('heading', { level: 2, name: 'Sala pronta' })
    await waitFor(() => expect(successHeading).toHaveFocus())
    expect(screen.getByText('9HKGW4RT')).toBeVisible()
    expect(screen.getAllByText('Minerva')[0]).toBeVisible()
    const recoveryLink = screen.getByLabelText('Link de recuperação')
    const expectedRecoveryLink = `${window.location.origin}${window.location.pathname}#recovery=${'a'.repeat(64)}`
    expect(recoveryLink).toHaveValue(expectedRecoveryLink)
    expect(request).toHaveBeenNthCalledWith(
      2,
      '/api/rooms',
      expect.objectContaining({
        body: JSON.stringify({
          display_name: 'Minerva',
          recovery_password: 'a long uncommon passphrase',
        }),
        credentials: 'same-origin',
        method: 'POST',
      }),
    )
    expect(
      (request.mock.calls[1]?.[1] as RequestInit | undefined)?.headers,
    ).toEqual(expect.objectContaining({ 'Idempotency-Key': expect.any(String) }))

    await fireEvent.click(screen.getByRole('button', { name: 'Copiar link' }))
    await screen.findByText('Link individual copiado.')
    expect(clipboard.writeText).toHaveBeenNthCalledWith(1, expectedRecoveryLink)
    await fireEvent.click(screen.getByRole('button', { name: 'Já guardei o link' }))
    expect(screen.queryByLabelText('Link de recuperação')).not.toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Copiar código da sala' }))
    await screen.findByText('Código copiado.')
    expect(clipboard.writeText).toHaveBeenNthCalledWith(2, '9HKGW4RT')
  })

  it('recovers the linked position with the room password and no room identifier', async () => {
    const token = 'c'.repeat(64)
    window.__HOGWARTS_RECOVERY_TOKEN__ = token
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(recoveredGuestLobbyResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    expect(
      await screen.findByRole('heading', { level: 2, name: 'Recupere sua participação' }),
    ).toBeVisible()
    expect(window.__HOGWARTS_RECOVERY_TOKEN__).toBeUndefined()
    expect(screen.queryByLabelText('Código da sala')).not.toBeInTheDocument()
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação da sala'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Recuperar minha posição' }))

    expect(await screen.findByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()
    expect(screen.getAllByText('Posição 2')[0]).toBeVisible()
    expect(request).toHaveBeenNthCalledWith(
      2,
      '/api/session/recover',
      expect.objectContaining({
        credentials: 'same-origin',
        method: 'POST',
      }),
    )
    expect(
      JSON.parse(String((request.mock.calls[1]?.[1] as RequestInit | undefined)?.body)),
    ).toEqual({
      recovery_password: 'a long uncommon passphrase',
      recovery_token: token,
      recovery_attempt_id: expect.stringMatching(
        /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
      ),
    })
    expect(localStorage.getItem('hogwarts.session.expected')).toBe('true')
    expect(sessionStorage.getItem('hogwarts.participant-recovery.attempt-id')).toBeNull()
  })

  it('requires an explicit session choice before replacing access on a third device', async () => {
    const token = '6'.repeat(64)
    const successorToken = '7'.repeat(64)
    const firstSessionId = '2fe6c1be-50fc-42ac-8c4f-6ef270099c24'
    const secondSessionId = '8aa543d4-9d6f-4a8c-bd7b-6c6605be48fc'
    window.__HOGWARTS_RECOVERY_TOKEN__ = token
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            sessions: [
              {
                created_at: '2026-09-01T14:20:00Z',
                id: firstSessionId,
                label: 'Sessão 1',
              },
              {
                created_at: '2026-09-03T10:05:00Z',
                id: secondSessionId,
                label: 'Sessão 2',
              },
            ],
            status: 'replacement_required',
          }),
          {
            headers: { 'Content-Type': 'application/json' },
            status: 409,
          },
        ),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            kind: 'lobby',
            lobby: guestLobbyResponse(),
            recovery_token: successorToken,
          }),
          {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          },
        ),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByRole('heading', { level: 2, name: 'Recupere sua participação' })
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação da sala'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Recuperar minha posição' }))

    expect(
      await screen.findByRole('heading', { level: 2, name: 'Escolha uma sessão para substituir' }),
    ).toBeVisible()
    expect(screen.getByText('Criada em 01/09/2026, 11:20')).toBeVisible()
    expect(screen.getByText('Criada em 03/09/2026, 07:05')).toBeVisible()
    expect(request).toHaveBeenCalledTimes(2)

    await fireEvent.click(screen.getByRole('radio', { name: 'Sessão 1' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Substituir Sessão 1' }))

    expect(await screen.findByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()
    expect(screen.getByLabelText('Link de recuperação')).toHaveValue(
      `${window.location.origin}${window.location.pathname}#recovery=${successorToken}`,
    )
    const discovery = JSON.parse(
      String((request.mock.calls[1]?.[1] as RequestInit | undefined)?.body),
    ) as { recovery_attempt_id: string }
    expect(
      JSON.parse(String((request.mock.calls[2]?.[1] as RequestInit | undefined)?.body)),
    ).toEqual({
      recovery_attempt_id: discovery.recovery_attempt_id,
      recovery_password: 'a long uncommon passphrase',
      recovery_token: token,
      replace_session_id: firstSessionId,
    })
  })

  it('gives a recovery link precedence over an older pending room join', async () => {
    const token = 'f'.repeat(64)
    window.__HOGWARTS_RECOVERY_TOKEN__ = token
    sessionStorage.setItem(
      'hogwarts.room-join.pending-intent',
      JSON.stringify({
        commandType: 'join_room',
        createdAt: new Date().toISOString(),
        idempotencyKey: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
        input: { display_name: 'Luna', hero_id: 'hermione' },
        roomCode: '9HKGW4RT',
      }),
    )
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(recoveredGuestLobbyResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    expect(
      await screen.findByRole('heading', { level: 2, name: 'Recupere sua participação' }),
    ).toBeVisible()
    expect(request).toHaveBeenCalledTimes(1)
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação da sala'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Recuperar minha posição' }))

    expect(await screen.findByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()
    expect(request).toHaveBeenNthCalledWith(
      2,
      '/api/session/recover',
      expect.objectContaining({ method: 'POST' }),
    )
    expect(sessionStorage.getItem('hogwarts.room-join.pending-intent')).toBeNull()
  })

  it('reuses the browser-bound recovery attempt after a lost response', async () => {
    const token = '1'.repeat(64)
    window.__HOGWARTS_RECOVERY_TOKEN__ = token
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockRejectedValueOnce(new TypeError('response lost after commit'))
      .mockResolvedValueOnce(
        new Response(JSON.stringify(recoveredGuestLobbyResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByRole('heading', { level: 2, name: 'Recupere sua participação' })
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação da sala'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Recuperar minha posição' }))
    await screen.findByText(
      'A confirmação não chegou. Tente novamente nesta tela ou reabra o link nesta mesma aba.',
    )

    const firstAttempt = JSON.parse(
      String((request.mock.calls[1]?.[1] as RequestInit | undefined)?.body),
    ) as { recovery_attempt_id: string }
    expect(sessionStorage.getItem('hogwarts.participant-recovery.attempt-id')).toBe(
      firstAttempt.recovery_attempt_id,
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Recuperar minha posição' }))
    expect(await screen.findByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()
    const retriedAttempt = JSON.parse(
      String((request.mock.calls[2]?.[1] as RequestInit | undefined)?.body),
    ) as { recovery_attempt_id: string }
    expect(retriedAttempt.recovery_attempt_id).toBe(firstAttempt.recovery_attempt_id)
    expect(sessionStorage.getItem('hogwarts.participant-recovery.attempt-id')).toBeNull()
  })

  it('uses one safe error for an invalid recovery link or room password', async () => {
    window.__HOGWARTS_RECOVERY_TOKEN__ = 'd'.repeat(64)
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(errorResponse('RECOVERY_FAILED')), {
          headers: { 'Content-Type': 'application/json' },
          status: 401,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByRole('heading', { level: 2, name: 'Recupere sua participação' })
    await fireEvent.update(screen.getByLabelText('Senha de recuperação da sala'), 'wrong value')
    await fireEvent.click(screen.getByRole('button', { name: 'Recuperar minha posição' }))

    expect(
      await screen.findByText(
        'Não foi possível recuperar a participação. Confira o link e a senha da sala.',
      ),
    ).toHaveAttribute('role', 'alert')
    expect(screen.getByLabelText('Senha de recuperação da sala')).toHaveAttribute(
      'aria-invalid',
      'true',
    )
    expect(screen.queryByText(/Minerva|Luna/)).not.toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Voltar ao início' }))
    const createHeading = await screen.findByRole('heading', {
      level: 2,
      name: 'Abra uma sala para o seu grupo',
    })
    expect(createHeading).toBeVisible()
    await waitFor(() => expect(screen.getByLabelText('Seu nome')).toHaveFocus())
  })

  it('distinguishes a service failure from invalid recovery credentials', async () => {
    window.__HOGWARTS_RECOVERY_TOKEN__ = 'e'.repeat(64)
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(errorResponse('INTERNAL_ERROR')), {
          headers: { 'Content-Type': 'application/json' },
          status: 500,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByRole('heading', { level: 2, name: 'Recupere sua participação' })
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação da sala'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Recuperar minha posição' }))

    expect(
      await screen.findByText(
        'O serviço não conseguiu confirmar a recuperação. Tente novamente com o mesmo link.',
      ),
    ).toHaveAttribute('role', 'alert')
    expect(screen.queryByRole('button', { name: 'Voltar ao início' })).not.toBeInTheDocument()
  })

  it('reuses the same idempotency key after an uncertain server failure', async () => {
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            error: {
              category: 'internal',
              code: 'INTERNAL_ERROR',
              correlation_id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
              details: {},
              message_key: 'internal.error',
              retry: 'safe_to_retry',
            },
          }),
          {
            headers: { 'Content-Type': 'application/json' },
            status: 503,
          },
        ),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify(hostRoomResponse()),
          {
            headers: { 'Content-Type': 'application/json' },
            status: 201,
          },
        ),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' })
    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Minerva')
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Criar sala privada' }))
    await screen.findByRole('button', { name: 'Tentar criar novamente' })

    const firstKey = (request.mock.calls[1]?.[1] as RequestInit | undefined)?.headers
    await fireEvent.click(screen.getByRole('button', { name: 'Tentar criar novamente' }))
    await screen.findByRole('heading', { level: 2, name: 'Sala pronta' })
    const retryKey = (request.mock.calls[2]?.[1] as RequestInit | undefined)?.headers

    expect(retryKey).toEqual(firstKey)
  })

  it('recovers the pending idempotency key after an uncertain failure and reload', async () => {
    const readyResponse = (): Response =>
      new Response(JSON.stringify({ status: 'ready' }), {
        headers: { 'Content-Type': 'application/json' },
        status: 200,
      })
    const request = vi
      .fn()
      .mockResolvedValueOnce(readyResponse())
      .mockRejectedValueOnce(new TypeError('response lost after commit'))
      .mockResolvedValueOnce(readyResponse())
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify(hostRoomResponse()),
          {
            headers: { 'Content-Type': 'application/json' },
            status: 201,
          },
        ),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' })
    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Minerva')
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Criar sala privada' }))
    await screen.findByRole('button', { name: 'Tentar criar novamente' })
    const firstKey = (request.mock.calls[1]?.[1] as RequestInit | undefined)?.headers
    const persistedIntent = JSON.parse(
      sessionStorage.getItem('hogwarts.room-creation.pending-intent') ?? 'null',
    ) as unknown
    expect(persistedIntent).toEqual({
      commandType: 'create_room',
      createdAt: expect.any(String),
      idempotencyKey: expect.any(String),
    })
    expect(JSON.stringify(persistedIntent)).not.toContain('Minerva')
    expect(JSON.stringify(persistedIntent)).not.toContain('a long uncommon passphrase')
    expect((persistedIntent as { idempotencyKey: string }).idempotencyKey).toMatch(
      /^[A-Za-z0-9_.:-]{8,128}$/,
    )
    expect(
      Date.parse((persistedIntent as { createdAt: string }).createdAt),
    ).not.toBeNaN()

    cleanup()
    expect(sessionStorage.getItem('hogwarts.room-creation.pending-intent')).not.toBeNull()
    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' })
    expect(sessionStorage.getItem('hogwarts.room-creation.pending-intent')).not.toBeNull()
    expect(screen.getByLabelText('Seu nome')).toHaveValue('')
    expect(screen.getByLabelText('Seu nome')).not.toHaveAttribute('readonly')
    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Minerva')
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Retomar criação pendente' }))
    await screen.findByRole('heading', { level: 2, name: 'Sala pronta' })
    const retryKey = (request.mock.calls[3]?.[1] as RequestInit | undefined)?.headers

    expect(retryKey).toEqual(firstKey)
  })

  it('keeps a recovered intent until the host resumes or explicitly discards it', async () => {
    const readyResponse = (): Response =>
      new Response(JSON.stringify({ status: 'ready' }), {
        headers: { 'Content-Type': 'application/json' },
        status: 200,
      })
    const request = vi
      .fn()
      .mockResolvedValueOnce(readyResponse())
      .mockRejectedValueOnce(new TypeError('response lost after commit'))
      .mockResolvedValueOnce(readyResponse())
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            error: {
              category: 'conflict',
              code: 'IDEMPOTENCY_KEY_REUSED',
              correlation_id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
              details: {},
              message_key: 'request.idempotency_key.reused',
              retry: 'with_new_idempotency_key',
            },
          }),
          { headers: { 'Content-Type': 'application/json' }, status: 409 },
        ),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify(hostRoomResponse()),
          { headers: { 'Content-Type': 'application/json' }, status: 201 },
        ),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' })
    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Minerva')
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Criar sala privada' }))
    await screen.findByRole('button', { name: 'Tentar criar novamente' })
    const firstKey = (request.mock.calls[1]?.[1] as RequestInit | undefined)?.headers

    cleanup()
    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByText('Existe uma criação pendente neste navegador.')
    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Pomona')
    await fireEvent.update(screen.getByLabelText('Senha de recuperação'), 'a different passphrase')
    await fireEvent.click(screen.getByRole('button', { name: 'Retomar criação pendente' }))

    await screen.findByText(
      'O nome ou a senha não correspondem à criação pendente. Reinsira os mesmos dados ou descarte a tentativa.',
    )
    expect(screen.getByRole('button', { name: 'Descartar e começar outra' })).toBeVisible()
    expect(screen.getByLabelText('Senha de recuperação')).not.toHaveAttribute('readonly')
    const mismatchedKey = (request.mock.calls[3]?.[1] as RequestInit | undefined)?.headers

    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Minerva')
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Retomar criação pendente' }))
    await screen.findByRole('heading', { level: 2, name: 'Sala pronta' })
    const recoveredKey = (request.mock.calls[4]?.[1] as RequestInit | undefined)?.headers

    expect(mismatchedKey).toEqual(firstKey)
    expect(recoveredKey).toEqual(firstKey)
  })

  it('keeps the submitted payload immutable while confirmation is pending', async () => {
    let completeCreation = (_response: Response): void => undefined
    const creationResponse = new Promise<Response>((resolve) => {
      completeCreation = resolve
    })
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
        .mockReturnValueOnce(creationResponse),
    )

    render(App, { global: { plugins: [createPinia()] } })
    const name = await screen.findByLabelText('Seu nome')
    const password = screen.getByLabelText('Senha de recuperação')
    await fireEvent.update(name, 'Minerva')
    await fireEvent.update(password, 'a long uncommon passphrase')
    await fireEvent.click(screen.getByRole('button', { name: 'Criar sala privada' }))

    expect(name).toHaveAttribute('readonly')
    expect(password).toHaveAttribute('readonly')

    completeCreation(
      new Response(
        JSON.stringify(hostRoomResponse()),
        { headers: { 'Content-Type': 'application/json' }, status: 201 },
      ),
    )
    await screen.findByRole('heading', { level: 2, name: 'Sala pronta' })
  })

  it('fails closed when a successful response violates the room contract', async () => {
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            participant: { display_name: '', role: 'host' },
            room: { code: 'not-a-room-code', status: 'open' },
          }),
          { headers: { 'Content-Type': 'application/json' }, status: 201 },
        ),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' })
    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Minerva')
    await fireEvent.update(
      screen.getByLabelText('Senha de recuperação'),
      'a long uncommon passphrase',
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Criar sala privada' }))

    expect(
      await screen.findByText(
        'A confirmação não chegou. Tente novamente para consultar a mesma criação.',
      ),
    ).toHaveAttribute('role', 'alert')
    expect(screen.queryByRole('heading', { level: 2, name: 'Sala pronta' })).not.toBeInTheDocument()
  })

  it('lets the host inspect the recovery password before creating the room', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      ),
    )

    render(App, { global: { plugins: [createPinia()] } })

    const password = await screen.findByLabelText('Senha de recuperação')
    expect(password).toHaveAttribute('type', 'password')
    await fireEvent.click(screen.getByRole('button', { name: 'Mostrar senha' }))
    expect(password).toHaveAttribute('type', 'text')
    expect(screen.getByRole('button', { name: 'Ocultar senha' })).toBeVisible()
  })

  it('associates a weak-password error with the password field and focuses it', async () => {
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            error: {
              category: 'validation',
              code: 'WEAK_RECOVERY_PASSWORD',
              correlation_id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
              details: {},
              message_key: 'room.recovery_password.weak',
              retry: 'after_correction',
            },
          }),
          {
            headers: { 'Content-Type': 'application/json' },
            status: 422,
          },
        ),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' })
    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Minerva')
    const password = screen.getByLabelText('Senha de recuperação')
    await fireEvent.update(password, 'passwordpassword')
    await fireEvent.click(screen.getByRole('button', { name: 'Criar sala privada' }))

    await screen.findByText('Escolha uma senha mais longa e menos previsível.')
    expect(password).toHaveAttribute('aria-invalid', 'true')
    await waitFor(() => expect(password).toHaveFocus())
  })

  it('lets a guest find a room, choose an available hero and receive a position', async () => {
    const heroes = availableHeroes
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({ room: { code: '9HKGW4RT', status: 'open' }, heroes }),
          { headers: { 'Content-Type': 'application/json' }, status: 200 },
        ),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            content_options: contentOptions,
            heroes: heroes.map((hero) => ({
              ...hero,
              available: hero.id !== 'hermione',
            })),
            participant: {
              display_name: 'Luna',
              hero: { id: 'hermione', name: 'Hermione' },
              position: 2,
              ready: false,
              role: 'guest',
            },
            participants: [
              { display_name: 'Minerva', position: 1, ready: false, role: 'host' },
              {
                display_name: 'Luna',
                hero: { id: 'hermione', name: 'Hermione' },
                position: 2,
                ready: false,
                role: 'guest',
              },
            ],
            recovery_token: 'b'.repeat(64),
            room: { code: '9HKGW4RT', status: 'open' },
          }),
          { headers: { 'Content-Type': 'application/json' }, status: 201 },
        ),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' })
    await fireEvent.click(screen.getByRole('button', { name: 'Entrar em uma sala' }))
    await fireEvent.update(screen.getByLabelText('Código da sala'), '9hkgw4rt')
    await fireEvent.click(screen.getByRole('button', { name: 'Localizar sala' }))

    await screen.findByRole('heading', { level: 2, name: 'Escolha seu lugar à mesa' })
    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Luna')
    await fireEvent.click(screen.getByRole('radio', { name: 'Hermione' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Entrar na sala' }))

    expect(await screen.findByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()
    expect(screen.getAllByText('Luna')[0]).toBeVisible()
    expect(screen.getAllByText('Posição 2')[0]).toBeVisible()
    expect(localStorage.getItem('hogwarts.session.expected')).toBe('true')
    expect(request).toHaveBeenNthCalledWith(
      2,
      '/api/rooms/9HKGW4RT',
      expect.objectContaining({ credentials: 'same-origin' }),
    )
    expect(request).toHaveBeenNthCalledWith(
      3,
      '/api/rooms/9HKGW4RT/participants',
      expect.objectContaining({ method: 'POST' }),
    )
  })

  it('replays a pending join after a lost response and reload', async () => {
    let rejectPendingJoin!: (reason?: unknown) => void
    const pendingJoin = new Promise<Response>((_resolve, reject) => {
      rejectPendingJoin = reject
    })
    const readyResponse = (): Response =>
      new Response(JSON.stringify({ status: 'ready' }), {
        headers: { 'Content-Type': 'application/json' },
        status: 200,
      })
    const request = vi
      .fn()
      .mockResolvedValueOnce(readyResponse())
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            room: { code: '9HKGW4RT', status: 'open' },
            heroes: availableHeroes,
          }),
          { headers: { 'Content-Type': 'application/json' }, status: 200 },
        ),
      )
      .mockReturnValueOnce(pendingJoin)
      .mockResolvedValueOnce(readyResponse())
      .mockResolvedValueOnce(
        new Response(JSON.stringify(errorResponse('SESSION_INVALID')), {
          headers: { 'Content-Type': 'application/json' },
          status: 401,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(guestJoinResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 201,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' })
    await fireEvent.click(screen.getByRole('button', { name: 'Entrar em uma sala' }))
    await fireEvent.update(screen.getByLabelText('Código da sala'), '9hkgw4rt')
    await fireEvent.click(screen.getByRole('button', { name: 'Localizar sala' }))
    await screen.findByRole('heading', { level: 2, name: 'Escolha seu lugar à mesa' })
    await fireEvent.update(screen.getByLabelText('Seu nome'), 'Luna')
    await fireEvent.click(screen.getByRole('radio', { name: 'Hermione' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Entrar na sala' }))
    const pendingDiscard = screen.getByRole('button', {
      name: 'Descartar entrada e usar outro código',
    })
    expect(pendingDiscard).toBeDisabled()
    await fireEvent.click(pendingDiscard)
    expect(screen.getByLabelText('Seu nome')).toHaveValue('Luna')
    rejectPendingJoin(new TypeError('response lost after commit'))
    await screen.findByText('A confirmação não chegou. Tente entrar novamente com os mesmos dados.')
    expect(screen.queryByRole('button', { name: 'Usar outro código' })).not.toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Descartar entrada e usar outro código' }),
    ).toBeVisible()

    const initialRequest = request.mock.calls[2]?.[1] as RequestInit | undefined
    const persistedIntent = JSON.parse(
      sessionStorage.getItem('hogwarts.room-join.pending-intent') ?? 'null',
    ) as unknown
    expect(persistedIntent).toEqual({
      commandType: 'join_room',
      createdAt: expect.any(String),
      idempotencyKey: expect.any(String),
      input: { display_name: 'Luna', hero_id: 'hermione' },
      roomCode: '9HKGW4RT',
    })

    await fireEvent.click(
      screen.getByRole('button', { name: 'Descartar entrada e usar outro código' }),
    )
    expect(sessionStorage.getItem('hogwarts.room-join.pending-intent')).toBeNull()
    expect(screen.getByLabelText('Código da sala')).toHaveFocus()

    sessionStorage.setItem('hogwarts.room-join.pending-intent', JSON.stringify(persistedIntent))

    cleanup()
    render(App, { global: { plugins: [createPinia()] } })

    expect(await screen.findByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()
    const replayRequest = request.mock.calls[5]?.[1] as RequestInit | undefined
    expect(request).toHaveBeenNthCalledWith(
      6,
      '/api/rooms/9HKGW4RT/participants',
      expect.objectContaining({ method: 'POST' }),
    )
    expect(replayRequest?.headers).toEqual(initialRequest?.headers)
    expect(replayRequest?.body).toBe(initialRequest?.body)
    expect(sessionStorage.getItem('hogwarts.room-join.pending-intent')).toBeNull()
    expect(localStorage.getItem('hogwarts.session.expected')).toBe('true')
  })

  it('lets a ready host seal the room and renders only the redacted initial projection', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const request = vi.fn().mockImplementation((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(readyHostLobbyResponse()), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/games' && init?.method === 'POST') {
        return Promise.resolve(
          new Response(JSON.stringify(gameProjectionResponse()), {
            headers: { 'Content-Type': 'application/json' },
            status: 201,
          }),
        )
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByRole('heading', { level: 2, name: 'Sala pronta' })
    expect(screen.getByLabelText('Aventura e conteúdo da partida')).toHaveValue(
      `${playableContentOptions[0]?.manifest_digest}:adventure:001`,
    )
    expect(
      screen.queryByText(
        'Nenhum Manifesto jogável está publicado. Lacunas funcionais impedem o selo da sala.',
      ),
    ).not.toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: 'Selar sala e iniciar' }))

    const heading = await screen.findByRole('heading', { level: 2, name: 'Partida iniciada' })
    await waitFor(() => expect(heading).toHaveFocus())
    expect(screen.getByText('Artes das Trevas')).toBeVisible()
    expect(screen.getByText('Snapshot inicial confirmado')).toBeVisible()
    expect(screen.getByText('A seed permanece secreta enquanto a partida estiver em andamento.')).toBeVisible()
    expect(document.body.textContent).not.toContain('0123456789abcdef0123456789abcdef')

    const gameCall = request.mock.calls.find(([url]) => String(url) === '/api/games')
    expect(gameCall?.[1]).toEqual(
      expect.objectContaining({
        body: JSON.stringify({
          adventure_id: 'adventure:001',
          manifest_digest: playableContentOptions[0]?.manifest_digest,
          ruleset_version: 'fixture-rules-v1',
        }),
        credentials: 'same-origin',
        method: 'POST',
      }),
    )
    expect((gameCall?.[1] as RequestInit | undefined)?.headers).toEqual(
      expect.objectContaining({ 'Idempotency-Key': expect.any(String) }),
    )
  })

  it('lets the responsible participant select and submit a simple pending choice', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const request = vi.fn().mockImplementation((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(pendingChoiceProjectionResponse()), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/games/current/commands' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body)) as { command_id: string }
        return Promise.resolve(
          new Response(JSON.stringify(acceptedChoiceResponse(body.command_id)), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByRole('heading', { level: 3, name: 'Escolha oficial pendente' })
    await fireEvent.click(screen.getByRole('radio', { name: 'Minerva' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Confirmar escolha' }))

    await waitFor(() =>
      expect(
        request.mock.calls.some(
          ([url, init]) =>
            String(url) === '/api/games/current/commands' && init?.method === 'POST',
        ),
      ).toBe(true),
    )
    const commandCall = request.mock.calls.find(
      ([url, init]) => String(url) === '/api/games/current/commands' && init?.method === 'POST',
    )
    const submitted = JSON.parse(String(commandCall?.[1]?.body)) as Record<string, unknown>
    expect(submitted).toEqual({
      choice_id: pendingChoiceId,
      command_id: expect.stringMatching(
        /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
      ),
      expected_state_version: 2,
      selected_options: ['hero:1'],
      type: 'resolve_choice',
    })
  })

  it('submits a multi-select choice in the official option order', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const projection = multiplePendingChoiceProjectionResponse()
    let completeCommand = (_response: Response): void => undefined
    const commandResponse = new Promise<Response>((resolve) => {
      completeCommand = resolve
    })
    let submittedCommandId = ''
    const request = vi.fn().mockImplementation((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(projection), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/games/current/commands' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body)) as { command_id: string }
        submittedCommandId = body.command_id
        return commandResponse
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    expect(await screen.findAllByRole('checkbox')).toHaveLength(3)
    const choiceGroup = screen.getByRole('group', { name: 'Selecione de 1 a 2 opções' })
    expect(choiceGroup).toHaveAccessibleDescription(
      'Luna precisa escolher 1 a 2 entre as opções elegíveis. Causa oficial rule:functional',
    )
    const confirmation = screen.getByRole('button', { name: 'Confirmar escolha' })
    expect(confirmation).toBeDisabled()

    const pomonaOption = screen.getByRole('checkbox', { name: 'Pomona' })
    const minervaOption = screen.getByRole('checkbox', { name: 'Minerva' })
    const lunaOption = screen.getByRole('checkbox', { name: 'Luna' })
    await fireEvent.click(pomonaOption)
    expect(confirmation).toBeEnabled()
    expect(lunaOption).toBeEnabled()
    await fireEvent.click(minervaOption)
    expect(confirmation).toBeEnabled()
    expect(pomonaOption).toBeEnabled()
    expect(minervaOption).toBeEnabled()
    expect(lunaOption).toBeDisabled()
    void fireEvent.click(confirmation)

    await waitFor(() =>
      expect(
        request.mock.calls.some(
          ([url, init]) =>
            String(url) === '/api/games/current/commands' && init?.method === 'POST',
        ),
      ).toBe(true),
    )
    expect(screen.getByRole('button', { name: 'Aguardando confirmação' })).toBeDisabled()
    const commandCall = request.mock.calls.find(
      ([url, init]) => String(url) === '/api/games/current/commands' && init?.method === 'POST',
    )
    const submitted = JSON.parse(String(commandCall?.[1]?.body)) as Record<string, unknown>
    expect(submitted.selected_options).toEqual(['hero:1', 'hero:3'])

    completeCommand(
      new Response(JSON.stringify(acceptedChoiceResponse(submittedCommandId)), {
        headers: { 'Content-Type': 'application/json' },
        status: 200,
      }),
    )
    expect(await screen.findByText('Ação do Herói')).toBeVisible()
  })

  it('keeps a newer realtime projection after a delayed choice response', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const projection = pendingChoiceProjectionResponse()
    let completeCommand = (_response: Response): void => undefined
    const commandResponse = new Promise<Response>((resolve) => {
      completeCommand = resolve
    })
    let submittedCommandId = ''
    const request = vi.fn().mockImplementation((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(projection), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/games/current/commands' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body)) as { command_id: string }
        submittedCommandId = body.command_id
        return commandResponse
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    const pinia = createPinia()
    const roomAccess = useRoomAccessStore(pinia)
    render(App, { global: { plugins: [pinia] } })

    const option = await screen.findByRole('radio', { name: 'Minerva' })
    await waitFor(() => expect(option).toBeEnabled())
    await fireEvent.click(screen.getByText('Ver versões do Snapshot'))
    await fireEvent.click(option)
    void fireEvent.click(screen.getByRole('button', { name: 'Confirmar escolha' }))
    expect(
      await screen.findByRole('button', { name: 'Aguardando confirmação' }),
    ).toBeDisabled()

    const newerProjection = {
      ...projection,
      choice: { status: 'none' },
      effects: { outcomes: [], status: 'resolved' },
      legal_actions: [],
      snapshot: {
        ...projection.snapshot,
        cursor: 3,
        digest: `blake3:${'a'.repeat(64)}`,
        sequence: 3,
        state_version: 4,
      },
      turn: { active_position: 1, number: 2, phase: 'hero_action' },
    }
    const socket = SynchronizedWebSocket.instances[0]
    if (!socket) {
      throw new Error('the realtime race fixture requires an open socket')
    }
    socket.receive({
      cursor: 3,
      events: [
        {
          actor_position: 2,
          choice_cause: 'rule:functional',
          choice_id: pendingChoiceId,
          effect_stop: 'stable',
          effects: [],
          event_version: 3,
          prng_counter: 0,
          selected_options: ['hero:1'],
          sequence: 2,
          state_version: 3,
          turn: 1,
          type: 'choice_resolved',
        },
        {
          actor_position: 1,
          effect_stop: 'stable',
          effects: [],
          event_version: 3,
          prng_counter: 0,
          sequence: 3,
          state_version: 4,
          turn: 2,
          type: 'dark_arts_completed',
        },
      ],
      from_cursor: 1,
      projection: newerProjection,
      protocol_version: 2,
      type: 'events',
    })

    expect(await screen.findByText('v4 · sequência 3')).toBeVisible()
    const snapshotDisclosure = screen.getByText('Ver versões do Snapshot')
    snapshotDisclosure.focus()
    expect(snapshotDisclosure).toHaveFocus()
    completeCommand(
      new Response(JSON.stringify(acceptedChoiceResponse(submittedCommandId)), {
        headers: { 'Content-Type': 'application/json' },
        status: 200,
      }),
    )

    expect(await screen.findByText('Recibo aceito no estado v3, sequência 2.')).toBeVisible()
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 2, name: 'Partida em andamento' })).toHaveFocus(),
    )
    expect(roomAccess.game?.snapshot).toMatchObject({ sequence: 3, state_version: 4 })
    expect(screen.getByText('v4 · sequência 3')).toBeVisible()
    expect(screen.queryByText('v3 · sequência 2')).not.toBeInTheDocument()
  })

  it('submits an optional single choice empty after clearing its checkbox', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const baseProjection = multiplePendingChoiceProjectionResponse()
    const projection = {
      ...baseProjection,
      choice: {
        ...baseProjection.choice,
        max: 1,
        min: 0,
      },
    }
    const request = vi.fn().mockImplementation((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(projection), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/games/current/commands' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body)) as { command_id: string }
        return Promise.resolve(
          new Response(JSON.stringify(acceptedChoiceResponse(body.command_id)), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    expect(await screen.findAllByRole('checkbox')).toHaveLength(3)
    const firstOption = screen.getByRole('checkbox', { name: 'Minerva' })
    const confirmation = screen.getByRole('button', { name: 'Confirmar escolha' })
    expect(confirmation).toBeEnabled()

    await fireEvent.click(firstOption)
    expect(firstOption).toBeChecked()
    await fireEvent.click(firstOption)
    expect(firstOption).not.toBeChecked()
    expect(confirmation).toBeEnabled()
    await fireEvent.click(confirmation)

    await waitFor(() =>
      expect(
        request.mock.calls.some(
          ([url, init]) =>
            String(url) === '/api/games/current/commands' && init?.method === 'POST',
        ),
      ).toBe(true),
    )
    const commandCall = request.mock.calls.find(
      ([url, init]) => String(url) === '/api/games/current/commands' && init?.method === 'POST',
    )
    const submitted = JSON.parse(String(commandCall?.[1]?.body)) as Record<string, unknown>
    expect(submitted.selected_options).toEqual([])
  })

  it('keeps the official choice visible when the server says it belongs to another participant', async () => {
    class ChoiceResyncWebSocket {
      static readonly CONNECTING = 0
      static readonly OPEN = 1
      static readonly CLOSED = 3
      static instances: ChoiceResyncWebSocket[] = []

      readonly url: string
      readonly requestedProtocol: string
      protocol = ''
      readyState = ChoiceResyncWebSocket.CONNECTING
      onopen: (() => void) | null = null
      onmessage: ((event: { data: string }) => void) | null = null
      onerror: (() => void) | null = null
      onclose: ((event: { code: number }) => void) | null = null

      constructor(url: string | URL, protocol: string | string[]) {
        this.url = String(url)
        this.requestedProtocol = Array.isArray(protocol) ? (protocol[0] ?? '') : protocol
        ChoiceResyncWebSocket.instances.push(this)
      }

      open(): void {
        this.readyState = ChoiceResyncWebSocket.OPEN
        this.protocol = this.requestedProtocol
        this.onopen?.()
      }

      receive(message: unknown): void {
        this.onmessage?.({ data: JSON.stringify(message) })
      }

      close(): void {
        this.readyState = ChoiceResyncWebSocket.CLOSED
        this.onclose?.({ code: 1000 })
      }
    }

    vi.stubGlobal('WebSocket', ChoiceResyncWebSocket)
    localStorage.setItem('hogwarts.session.expected', 'true')
    const projection = pendingChoiceProjectionResponse()
    const request = vi.fn().mockImplementation((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(projection), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/games/current/commands' && init?.method === 'POST') {
        return Promise.resolve(
          new Response(JSON.stringify(errorResponse('CHOICE_NOT_ASSIGNED')), {
            headers: { 'Content-Type': 'application/json' },
            status: 403,
          }),
        )
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    const choiceHeading = await screen.findByRole('heading', {
      level: 3,
      name: 'Escolha oficial pendente',
    })
    const firstSocket = ChoiceResyncWebSocket.instances[0]
    if (!firstSocket) {
      throw new Error('the rejected choice fixture requires an initial socket')
    }
    firstSocket.open()
    firstSocket.receive({
      cursor: projection.snapshot.cursor,
      digest: projection.snapshot.digest,
      protocol_version: 2,
      snapshot_version: projection.snapshot.snapshot_version,
      type: 'synchronized',
    })
    const selectedOption = screen.getByRole('radio', { name: 'Minerva' })
    await waitFor(() => expect(selectedOption).toBeEnabled())
    await fireEvent.click(selectedOption)
    await fireEvent.click(screen.getByRole('button', { name: 'Confirmar escolha' }))

    const alert = await screen.findByRole('alert')
    expect(within(alert).getByText('Ação não aceita')).toBeVisible()
    expect(
      within(alert).getByText(
        'A escolha oficial pertence a outro participante. Aguarde a sincronização da partida antes de tentar novamente.',
      ),
    ).toBeVisible()
    expect(choiceHeading).toBeVisible()
    expect(screen.getByText('rule:functional')).toBeVisible()
    await waitFor(() => expect(ChoiceResyncWebSocket.instances).toHaveLength(2))
    const resyncSocket = ChoiceResyncWebSocket.instances[1]
    expect(resyncSocket?.url).toContain('snapshot_version=0')
    expect(screen.getByRole('radio', { name: 'Minerva' })).toBeChecked()
    expect(screen.getByRole('radio', { name: 'Minerva' })).toBeDisabled()
    expect(screen.getByRole('radio', { name: 'Luna' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Confirmar escolha' })).toBeDisabled()
    expect(sessionStorage.getItem('hogwarts.game-command.pending-intent')).toBeNull()

    const reassignedProjection = {
      ...projection,
      choice: { ...projection.choice, responsible_position: 1 },
      snapshot: {
        ...projection.snapshot,
        cursor: 2,
        digest: `blake3:${'f'.repeat(64)}`,
        sequence: 2,
        state_version: 3,
      },
    }
    resyncSocket?.open()
    resyncSocket?.receive({
      cursor: reassignedProjection.snapshot.cursor,
      projection: reassignedProjection,
      protocol_version: 2,
      type: 'snapshot',
    })

    await waitFor(() => expect(screen.queryByRole('radio')).not.toBeInTheDocument())
    expect(screen.queryByRole('button', { name: 'Confirmar escolha' })).not.toBeInTheDocument()
    expect(screen.getByText('Aguardando Minerva concluir a escolha.')).toBeVisible()
    expect(alert).toBeVisible()
  })

  it('restores the owned choice focus after an offline replay reconnection', async () => {
    class OfflineJourneyWebSocket {
      static readonly CONNECTING = 0
      static readonly OPEN = 1
      static readonly CLOSED = 3
      static instances: OfflineJourneyWebSocket[] = []

      readonly url: string
      readonly requestedProtocol: string
      protocol = ''
      readyState = OfflineJourneyWebSocket.CONNECTING
      onopen: (() => void) | null = null
      onmessage: ((event: { data: string }) => void) | null = null
      onerror: (() => void) | null = null
      onclose: ((event: { code: number }) => void) | null = null

      constructor(url: string | URL, protocol: string | string[]) {
        this.url = String(url)
        this.requestedProtocol = Array.isArray(protocol) ? (protocol[0] ?? '') : protocol
        OfflineJourneyWebSocket.instances.push(this)
      }

      open(): void {
        this.readyState = OfflineJourneyWebSocket.OPEN
        this.protocol = this.requestedProtocol
        this.onopen?.()
      }

      receive(message: unknown): void {
        this.onmessage?.({ data: JSON.stringify(message) })
      }

      close(): void {
        this.readyState = OfflineJourneyWebSocket.CLOSED
        this.onclose?.({ code: 1000 })
      }
    }

    let browserIsOnline = true
    vi.spyOn(window.navigator, 'onLine', 'get').mockImplementation(() => browserIsOnline)
    vi.spyOn(Math, 'random').mockReturnValue(0)
    vi.stubGlobal('WebSocket', OfflineJourneyWebSocket)
    localStorage.setItem('hogwarts.session.expected', 'true')
    const projection = pendingChoiceProjectionResponse()
    const request = vi.fn().mockImplementation((input: RequestInfo | URL) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(projection), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    const choicePanel = await screen.findByRole('region', {
      name: 'Escolha oficial pendente',
    })
    const gameHeading = screen.getByRole('heading', { level: 2, name: 'Partida em andamento' })
    await waitFor(() => expect(gameHeading).toHaveFocus())
    const firstSocket = OfflineJourneyWebSocket.instances[0]
    firstSocket?.open()
    firstSocket?.receive({
      cursor: projection.snapshot.cursor,
      digest: projection.snapshot.digest,
      protocol_version: 2,
      snapshot_version: projection.snapshot.snapshot_version,
      type: 'synchronized',
    })

    const firstOption = screen.getByRole('radio', { name: 'Minerva' })
    await waitFor(() => expect(firstOption).toHaveFocus())
    expect(firstOption).toBeEnabled()

    browserIsOnline = false
    window.dispatchEvent(new Event('offline'))

    expect(firstSocket?.readyState).toBe(OfflineJourneyWebSocket.CLOSED)
    expect(await screen.findByText('Atualizações automáticas interrompidas.')).toBeVisible()
    expect(firstOption).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Confirmar escolha' })).toBeDisabled()
    expect(choicePanel).toBeVisible()
    expect(within(choicePanel).getByText('rule:functional')).toBeVisible()
    expect(within(choicePanel).getByRole('radio', { name: 'Luna' })).toBeDisabled()

    gameHeading.focus()
    expect(gameHeading).toHaveFocus()

    browserIsOnline = true
    window.dispatchEvent(new Event('online'))
    await waitFor(() => expect(OfflineJourneyWebSocket.instances).toHaveLength(2))
    const secondSocket = OfflineJourneyWebSocket.instances[1]
    expect(secondSocket?.url).toContain(`cursor=${projection.snapshot.cursor}`)
    expect(secondSocket?.url).toContain(
      `snapshot_version=${projection.snapshot.snapshot_version}`,
    )
    secondSocket?.open()
    secondSocket?.receive({
      cursor: projection.snapshot.cursor,
      digest: projection.snapshot.digest,
      protocol_version: 2,
      snapshot_version: projection.snapshot.snapshot_version,
      type: 'synchronized',
    })
    secondSocket?.receive({
      blocked: false,
      game_id: projection.game.id,
      participants: [
        { position: 1, status: 'online' },
        { position: 2, status: 'online' },
      ],
      protocol_version: 2,
      required_participant_position: 2,
      type: 'presence',
    })

    const restoredFirstOption = screen.getByRole('radio', { name: 'Minerva' })
    await waitFor(() => expect(restoredFirstOption).toHaveFocus())
    expect(restoredFirstOption).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Confirmar escolha' })).toBeDisabled()
    expect(within(choicePanel).getByText('rule:functional')).toBeVisible()
    expect(screen.getByText('Online · Você')).toBeVisible()
  })

  it('keeps a tampered legal action read-only for a participant who does not own the choice', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const projection = pendingChoiceProjectionResponse()
    const observer = projection.participants[0]
    if (!observer) {
      throw new Error('the choice fixture requires an observing participant')
    }
    const request = vi.fn().mockImplementation((input: RequestInfo | URL) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(
            JSON.stringify({
              ...projection,
              legal_actions: ['resolve_choice'],
              participant: observer,
            }),
            {
              headers: { 'Content-Type': 'application/json' },
              status: 200,
            },
          ),
        )
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    const choicePanel = await screen.findByRole('region', {
      name: 'Escolha oficial pendente',
    })
    expect(within(choicePanel).getByText('Causa oficial')).toBeVisible()
    expect(within(choicePanel).getByText('rule:functional')).toBeVisible()
    expect(within(choicePanel).getByText(/Luna precisa escolher 1 entre as opções elegíveis/)).toBeVisible()
    expect(within(choicePanel).getByText('Minerva')).toBeVisible()
    expect(within(choicePanel).getByText('Luna')).toBeVisible()
    expect(within(choicePanel).queryByRole('radio')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Confirmar escolha' })).not.toBeInTheDocument()
    expect(screen.getByText('Aguardando Luna concluir a escolha.')).toBeVisible()
  })

  it('shows ephemeral availability and waits only for the required offline participant', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
        .mockResolvedValueOnce(
          new Response(JSON.stringify(gameProjectionResponse()), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        ),
    )

    render(App, { global: { plugins: [createPinia()] } })

    expect(
      await screen.findByRole('button', { name: 'Concluir Artes das Trevas' }),
    ).toBeEnabled()
    const socket = SynchronizedWebSocket.instances[0]
    socket?.receive({
      blocked: true,
      game_id: gameProjectionResponse().game.id,
      participants: [
        { position: 1, status: 'offline' },
        { position: 2, status: 'online' },
      ],
      protocol_version: 2,
      required_participant_position: 1,
      type: 'presence',
    })

    expect(await screen.findByText('Aguardando Minerva')).toBeVisible()
    expect(screen.getByText(/Não há bot, timeout ou pulo automático/)).toBeVisible()
    expect(screen.getByText('Offline · Você')).toBeVisible()
    expect(screen.getByText('Online')).toBeVisible()
    expect(screen.getByRole('button', { name: 'Concluir Artes das Trevas' })).toBeEnabled()

    socket?.receive({
      blocked: false,
      game_id: gameProjectionResponse().game.id,
      participants: [
        { position: 1, status: 'online' },
        { position: 2, status: 'offline' },
      ],
      protocol_version: 2,
      required_participant_position: 1,
      type: 'presence',
    })
    await waitFor(() => expect(screen.queryByText('Aguardando Minerva')).not.toBeInTheDocument())
    expect(screen.getByText('Online · Você')).toBeVisible()
    expect(screen.getByText('Offline')).toBeVisible()
  })

  it('confirms readiness and moves focus to the newly available primary action', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(unreadyHostLobbyResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(readyHostLobbyResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    const readyButton = await screen.findByRole('button', { name: 'Estou pronto' })
    await fireEvent.click(readyButton)

    const startButton = await screen.findByRole('button', { name: 'Selar sala e iniciar' })
    await waitFor(() => expect(startButton).toHaveFocus())
    expect(request).toHaveBeenNthCalledWith(
      3,
      '/api/session/readiness',
      expect.objectContaining({ body: JSON.stringify({ ready: true }), method: 'PUT' }),
    )
  })

  it('retries an uncertain start with the same idempotency key and locked selection', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(readyHostLobbyResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(errorResponse('INTERNAL_ERROR')), {
          headers: { 'Content-Type': 'application/json' },
          status: 500,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(gameProjectionResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 201,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByRole('heading', { level: 2, name: 'Sala pronta' })
    await fireEvent.click(screen.getByRole('button', { name: 'Selar sala e iniciar' }))

    expect(
      await screen.findByText(
        'A confirmação da partida falhou. Tente novamente com a mesma solicitação.',
      ),
    ).toBeVisible()
    expect(screen.getByLabelText('Aventura e conteúdo da partida')).toBeDisabled()
    expect(
      screen.getByText('Escolha preservada para repetir a mesma solicitação com segurança.'),
    ).toBeVisible()

    await fireEvent.click(screen.getByRole('button', { name: 'Selar sala e iniciar' }))
    await screen.findByRole('heading', { level: 2, name: 'Partida iniciada' })

    const gameCalls = request.mock.calls.filter(([url]) => String(url) === '/api/games')
    expect(gameCalls).toHaveLength(2)
    const firstHeaders = gameCalls[0]?.[1]?.headers as Record<string, string>
    const secondHeaders = gameCalls[1]?.[1]?.headers as Record<string, string>
    expect(secondHeaders['Idempotency-Key']).toBe(firstHeaders['Idempotency-Key'])
  })

  it('restores a guest position after the browser reloads', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(guestLobbyResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    expect(await screen.findByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()
    expect(screen.getAllByText('Luna')[0]).toBeVisible()
    expect(screen.getAllByText('Posição 2')[0]).toBeVisible()
    expect(screen.getAllByText('Hermione')[0]).toBeVisible()
    expect(request).toHaveBeenNthCalledWith(
      2,
      '/api/session',
      expect.objectContaining({ credentials: 'same-origin' }),
    )
  })

  it('shows realtime connection failure and offers an explicit retry', async () => {
    class FailingWebSocket {
      static attempts = 0

      constructor() {
        FailingWebSocket.attempts += 1
        throw new Error('socket unavailable')
      }
    }
    localStorage.setItem('hogwarts.session.expected', 'true')
    vi.stubGlobal('WebSocket', FailingWebSocket)
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
        .mockResolvedValueOnce(
          new Response(JSON.stringify(gameProjectionResponse()), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        ),
    )

    render(App, { global: { plugins: [createPinia()] } })

    expect(await screen.findByText('Atualizações automáticas interrompidas.')).toBeVisible()
    await fireEvent.click(screen.getByRole('button', { name: 'Reconectar atualizações' }))
    expect(FailingWebSocket.attempts).toBe(2)
  })

  it('keeps the active command frozen until realtime convergence is proven', async () => {
    class ControlledWebSocket {
      static readonly CONNECTING = 0
      static readonly OPEN = 1
      static readonly CLOSED = 3
      static instance: ControlledWebSocket | null = null

      readonly requestedProtocol: string
      protocol = ''
      readyState = ControlledWebSocket.CONNECTING
      onopen: (() => void) | null = null
      onmessage: ((event: { data: string }) => void) | null = null
      onerror: (() => void) | null = null
      onclose: ((event: { code: number }) => void) | null = null

      constructor(_url: string | URL, protocol: string | string[]) {
        this.requestedProtocol = Array.isArray(protocol) ? (protocol[0] ?? '') : protocol
        ControlledWebSocket.instance = this
      }

      open(): void {
        this.readyState = ControlledWebSocket.OPEN
        this.protocol = this.requestedProtocol
        this.onopen?.()
      }

      receive(message: unknown): void {
        this.onmessage?.({ data: JSON.stringify(message) })
      }

      close(): void {
        this.readyState = ControlledWebSocket.CLOSED
        this.onclose?.({ code: 1000 })
      }
    }
    localStorage.setItem('hogwarts.session.expected', 'true')
    const game = gameProjectionResponse()
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify(game), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)
    vi.stubGlobal('WebSocket', ControlledWebSocket)

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByRole('heading', { level: 2, name: 'Partida iniciada' })
    ControlledWebSocket.instance?.open()
    expect(screen.getByRole('button', { name: 'Sincronizando partida' })).toBeDisabled()
    expect(screen.queryByRole('button', { name: 'Concluir Artes das Trevas' })).toBeNull()

    ControlledWebSocket.instance?.receive({
      cursor: 0,
      digest: game.snapshot.digest,
      protocol_version: 2,
      snapshot_version: 1,
      type: 'synchronized',
    })
    expect(await screen.findByRole('button', { name: 'Concluir Artes das Trevas' })).toBeEnabled()

    const gapProjection = completedGameProjectionResponse()
    gapProjection.snapshot.cursor = 2
    gapProjection.snapshot.sequence = 2
    gapProjection.snapshot.state_version = 3
    gapProjection.snapshot.digest = `blake3:${'e'.repeat(64)}`
    ControlledWebSocket.instance?.receive({
      cursor: 2,
      events: [
        {
          actor_position: 1,
          event_version: 1,
          sequence: 2,
          state_version: 3,
          turn: 1,
          type: 'dark_arts_completed',
        },
      ],
      from_cursor: 1,
      projection: gapProjection,
      protocol_version: 2,
      type: 'events',
    })

    expect(await screen.findByRole('button', { name: 'Sincronizando partida' })).toBeDisabled()
    expect(request).toHaveBeenCalledTimes(2)
  })

  it('keeps a valid browser binding recoverable when session restoration loses the network', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const request = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
      .mockRejectedValueOnce(new TypeError('network unavailable'))
      .mockResolvedValueOnce(
        new Response(JSON.stringify(guestLobbyResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      )
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByRole('heading', { level: 2, name: 'Não foi possível retomar' })
    await fireEvent.click(screen.getByRole('button', { name: 'Tentar retomar sessão' }))

    expect(await screen.findByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()
    expect(localStorage.getItem('hogwarts.session.expected')).toBe('true')
  })

  it('keeps a submitted intention separate until the committed receipt returns', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    let completeCommand = (_response: Response): void => undefined
    const commandResponse = new Promise<Response>((resolve) => {
      completeCommand = resolve
    })
    let submittedCommandId = ''
    const request = vi.fn().mockImplementation((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(gameProjectionResponse()), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/games/current/commands' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body)) as { command_id: string }
        submittedCommandId = body.command_id
        return commandResponse
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    const pinia = createPinia()
    const gameCommand = useGameCommandStore(pinia)
    render(App, { global: { plugins: [pinia] } })
    await screen.findByRole('heading', { level: 2, name: 'Partida iniciada' })
    void fireEvent.click(screen.getByRole('button', { name: 'Concluir Artes das Trevas' }))

    await waitFor(() => expect(screen.getByText('Intenção pendente')).toBeVisible())
    expect(gameCommand.errorCode).toBeNull()
    expect(screen.getByText('Artes das Trevas')).toBeVisible()
    expect(screen.getByRole('button', { name: 'Aguardando confirmação' })).toBeDisabled()
    const persisted = JSON.parse(
      sessionStorage.getItem('hogwarts.game-command.pending-intent') ?? '{}',
    ) as Record<string, unknown>
    expect(Object.keys(persisted).sort()).toEqual([
      'commandId',
      'commandType',
      'createdAt',
      'gameId',
    ])

    completeCommand(
      new Response(JSON.stringify(acceptedCommandResponse(submittedCommandId)), {
        headers: { 'Content-Type': 'application/json' },
        status: 200,
      }),
    )

    expect(
      await screen.findByRole('heading', { level: 2, name: 'Partida em andamento' }),
    ).toBeVisible()
    expect(screen.getByText('Ação do Herói')).toBeVisible()
    expect(screen.getByText('Ação oficial')).toBeVisible()
    expect(screen.getByText(/Recibo aceito no estado v2, sequência 1/)).toBeVisible()
    expect(screen.getByRole('heading', { level: 3, name: 'Última resolução oficial' })).toBeVisible()
    expect(screen.getByText('Minerva pagou 1 de Vida (10 → 9).')).toBeVisible()
    expect(screen.getByText('Minerva recebeu 2 de Influência (0 → 2).')).toBeVisible()
    expect(screen.getByText('Dado D4: resultado 3.')).toBeVisible()
    expect(
      screen.getByText('Nenhum alvo era elegível. O efeito foi resolvido sem alterar a mesa.'),
    ).toBeVisible()
    expect(screen.getByText('Vida 9 · Ataque 2 · Influência 2')).toBeVisible()
    expect(sessionStorage.getItem('hogwarts.game-command.pending-intent')).toBeNull()

    const commandCall = request.mock.calls.find(
      ([url]) => String(url) === '/api/games/current/commands',
    )
    expect(JSON.parse(String(commandCall?.[1]?.body))).toEqual({
      command_id: submittedCommandId,
      expected_state_version: 1,
      type: 'complete_dark_arts',
    })
  })

  it('resynchronizes a stale game and waits for a new human decision', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    let sessionRequests = 0
    const request = vi.fn().mockImplementation((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        sessionRequests += 1
        return Promise.resolve(
          new Response(
            JSON.stringify(
              sessionRequests === 1 ? gameProjectionResponse() : completedGameProjectionResponse(),
            ),
            {
              headers: { 'Content-Type': 'application/json' },
              status: 200,
            },
          ),
        )
      }
      if (url === '/api/games/current/commands' && init?.method === 'POST') {
        return Promise.resolve(
          new Response(JSON.stringify(errorResponse('STALE_STATE_VERSION')), {
            headers: { 'Content-Type': 'application/json' },
            status: 409,
          }),
        )
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    const pinia = createPinia()
    const gameCommand = useGameCommandStore(pinia)
    render(App, { global: { plugins: [pinia] } })
    await screen.findByRole('heading', { level: 2, name: 'Partida iniciada' })
    await fireEvent.click(screen.getByRole('button', { name: 'Concluir Artes das Trevas' }))

    expect(await screen.findByText('Estado oficial atualizado')).toBeVisible()
    expect(screen.getByText('Ação do Herói')).toBeVisible()
    expect(screen.queryByRole('button', { name: 'Concluir Artes das Trevas' })).not.toBeInTheDocument()
    expect(gameCommand.status).toBe('resynced')
    expect(gameCommand.pendingIntent).toBeNull()
    expect(sessionStorage.getItem('hogwarts.game-command.pending-intent')).toBeNull()
    expect(sessionRequests).toBe(2)
    expect(
      request.mock.calls.filter(([url]) => String(url) === '/api/games/current/commands'),
    ).toHaveLength(1)
  })

  it('recovers an accepted command after the response is lost and the app reloads', async () => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    const firstRequest = vi.fn().mockImplementation((input: RequestInfo | URL) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(gameProjectionResponse()), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/games/current/commands') {
        return Promise.reject(new TypeError('response lost after commit'))
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', firstRequest)
    render(App, { global: { plugins: [createPinia()] } })

    await screen.findByRole('heading', { level: 2, name: 'Partida iniciada' })
    await fireEvent.click(screen.getByRole('button', { name: 'Concluir Artes das Trevas' }))
    expect(await screen.findByText('Confirmação ainda desconhecida')).toBeVisible()
    const pending = JSON.parse(
      sessionStorage.getItem('hogwarts.game-command.pending-intent') ?? '{}',
    ) as { commandId: string }

    cleanup()
    const secondRequest = vi.fn().mockImplementation((input: RequestInfo | URL) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(gameProjectionResponse()), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === `/api/games/current/commands/${pending.commandId}`) {
        return Promise.resolve(
          new Response(JSON.stringify(acceptedCommandResponse(pending.commandId)), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', secondRequest)
    render(App, { global: { plugins: [createPinia()] } })

    expect(
      await screen.findByRole('heading', { level: 2, name: 'Partida em andamento' }),
    ).toBeVisible()
    expect(screen.getByText('Ação oficial')).toBeVisible()
    expect(sessionStorage.getItem('hogwarts.game-command.pending-intent')).toBeNull()
  })

  it('recovers a pre-commit crash as an unaccepted intention', async () => {
    const game = gameProjectionResponse()
    const commandId = '642103d0-d780-48ea-bf65-c40228751911'
    localStorage.setItem('hogwarts.session.expected', 'true')
    sessionStorage.setItem(
      'hogwarts.game-command.pending-intent',
      JSON.stringify({
        commandId,
        commandType: 'complete_dark_arts',
        createdAt: '2026-09-03T12:00:00Z',
        gameId: game.game.id,
      }),
    )
    const request = vi.fn().mockImplementation((input: RequestInfo | URL) => {
      const url = String(input)
      if (url === '/health/ready') {
        return Promise.resolve(
          new Response(JSON.stringify({ status: 'ready' }), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === '/api/session') {
        return Promise.resolve(
          new Response(JSON.stringify(game), {
            headers: { 'Content-Type': 'application/json' },
            status: 200,
          }),
        )
      }
      if (url === `/api/games/current/commands/${commandId}`) {
        return Promise.resolve(
          new Response(JSON.stringify(errorResponse('COMMAND_NOT_FOUND')), {
            headers: { 'Content-Type': 'application/json' },
            status: 404,
          }),
        )
      }
      throw new Error(`unexpected request: ${url}`)
    })
    vi.stubGlobal('fetch', request)

    render(App, { global: { plugins: [createPinia()] } })

    expect(await screen.findByText('Nenhum aceite encontrado')).toBeVisible()
    expect(screen.getByText('Artes das Trevas')).toBeVisible()
    expect(screen.getByRole('button', { name: 'Concluir Artes das Trevas' })).toBeEnabled()
    expect(sessionStorage.getItem('hogwarts.game-command.pending-intent')).toBeNull()
  })
})
