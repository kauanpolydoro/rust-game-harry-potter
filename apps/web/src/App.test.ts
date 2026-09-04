import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/vue'
import { createPinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import App from './App.vue'
import { useGameCommandStore } from './stores/gameCommand'

class SynchronizedWebSocket {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSED = 3

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
    queueMicrotask(() => {
      this.readyState = SynchronizedWebSocket.OPEN
      this.protocol = this.requestedProtocol
      this.onopen?.()
      const request = new URL(this.url)
      this.onmessage?.({
        data: JSON.stringify({
          cursor: Number(request.searchParams.get('cursor')),
          digest: request.searchParams.get('digest'),
          protocol_version: 1,
          snapshot_version: Number(request.searchParams.get('snapshot_version')),
          type: 'synchronized',
        }),
      })
    })
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
      role: 'host',
    },
    participants: [
      {
        display_name: 'Minerva',
        hero: { id: 'harry', name: 'Harry' },
        position: 1,
        role: 'host',
      },
      {
        display_name: 'Luna',
        hero: { id: 'hermione', name: 'Hermione' },
        position: 2,
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
  return {
    ...projection,
    game: { ...projection.game, expires_at: '2026-09-10T13:00:00Z' },
    legal_actions: [],
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

describe('application shell', () => {
  beforeEach(() => {
    vi.stubGlobal('WebSocket', SynchronizedWebSocket)
  })

  afterEach(() => {
    cleanup()
    localStorage.clear()
    sessionStorage.clear()
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

    await fireEvent.click(screen.getByRole('button', { name: 'Copiar código da sala' }))
    await screen.findByText('Código copiado.')
    expect(clipboard.writeText).toHaveBeenCalledWith('9HKGW4RT')
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
        new Response(JSON.stringify(guestLobbyResponse()), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
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
      protocol_version: 1,
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
      protocol_version: 1,
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
