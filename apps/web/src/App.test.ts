import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/vue'
import { createPinia } from 'pinia'
import { afterEach, describe, expect, it, vi } from 'vitest'

import App from './App.vue'

const availableHeroes = [
  { available: true, id: 'harry', name: 'Harry' },
  { available: true, id: 'hermione', name: 'Hermione' },
  { available: true, id: 'neville', name: 'Neville' },
  { available: true, id: 'ron', name: 'Ron' },
]

function hostRoomResponse() {
  const participant = { display_name: 'Minerva', position: 1, role: 'host' }
  return {
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
    role: 'guest',
  }
  return {
    heroes: availableHeroes.map((hero) => ({
      ...hero,
      available: hero.id !== 'hermione',
    })),
    participant: guest,
    participants: [
      { display_name: 'Minerva', position: 1, role: 'host' },
      guest,
    ],
    room: { code: '9HKGW4RT', status: 'open' },
  }
}

describe('application shell', () => {
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
    expect(screen.getByText('Minerva')).toBeVisible()
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

    await fireEvent.click(screen.getByRole('button', { name: 'Copiar código' }))
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

    cleanup()
    render(App, { global: { plugins: [createPinia()] } })
    await screen.findByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' })
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
      await screen.findByText('Não foi possível criar a sala. Revise os dados e tente novamente.'),
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
            heroes: heroes.map((hero) => ({
              ...hero,
              available: hero.id !== 'hermione',
            })),
            participant: {
              display_name: 'Luna',
              hero: { id: 'hermione', name: 'Hermione' },
              position: 2,
              role: 'guest',
            },
            participants: [
              { display_name: 'Minerva', position: 1, role: 'host' },
              {
                display_name: 'Luna',
                hero: { id: 'hermione', name: 'Hermione' },
                position: 2,
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
    expect(screen.getByText('Luna')).toBeVisible()
    expect(screen.getByText('Posição 2')).toBeVisible()
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
    expect(screen.getByText('Luna')).toBeVisible()
    expect(screen.getByText('Posição 2')).toBeVisible()
    expect(screen.getByText('Hermione')).toBeVisible()
    expect(request).toHaveBeenNthCalledWith(
      2,
      '/api/session',
      expect.objectContaining({ credentials: 'same-origin' }),
    )
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
})
