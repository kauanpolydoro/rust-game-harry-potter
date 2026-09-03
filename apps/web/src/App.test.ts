import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/vue'
import { createPinia } from 'pinia'
import { afterEach, describe, expect, it, vi } from 'vitest'

import App from './App.vue'

describe('application shell', () => {
  afterEach(() => {
    cleanup()
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
          JSON.stringify({
            participant: { display_name: 'Minerva', role: 'host' },
            room: { code: '9HKGW4RT', status: 'open' },
          }),
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

    await screen.findByRole('heading', { level: 2, name: 'Sala pronta' })
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
          JSON.stringify({
            participant: { display_name: 'Minerva', role: 'host' },
            room: { code: '9HKGW4RT', status: 'open' },
          }),
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
})
