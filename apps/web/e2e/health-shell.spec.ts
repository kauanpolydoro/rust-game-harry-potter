import { expect, test } from '@playwright/test'

const initialGameProjection = {
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

const completedGameProjection = {
  ...initialGameProjection,
  effects: {
    outcomes: [
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
    ],
    status: 'resolved',
  },
  game: { ...initialGameProjection.game, expires_at: '2026-09-10T13:00:00Z' },
  legal_actions: [],
  participant: {
    ...initialGameProjection.participant,
    resources: { attack: 2, health: 9, influence: 2 },
  },
  participants: initialGameProjection.participants.map((participant) =>
    participant.position === 1
      ? { ...participant, resources: { attack: 2, health: 9, influence: 2 } }
      : participant,
  ),
  snapshot: {
    ...initialGameProjection.snapshot,
    cursor: 1,
    digest: `blake3:${'d'.repeat(64)}`,
    sequence: 1,
    state_version: 2,
  },
  turn: { ...initialGameProjection.turn, phase: 'hero_action' },
}

function errorResponse(code: string) {
  return {
    error: {
      category: 'request',
      code,
      correlation_id: 'dc8213d3-2941-4ef0-9ce8-b97cc6623410',
      details: {},
      message_key: `error.${code.toLowerCase()}`,
      retry: 'after_correction',
    },
  }
}

test('a player sees when the authoritative service is ready', async ({ page }) => {
  await page.goto('/')

  await expect(page.getByRole('heading', { level: 1, name: 'Batalha de Hogwarts' })).toBeVisible()
  await expect(page.getByRole('status')).toContainText('Servidor pronto')
  await expect(
    page.getByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' }),
  ).toBeVisible()
})

test('a player can retry when the authoritative service is unavailable', async ({ page }) => {
  await page.route('**/health/ready', (route) =>
    route.fulfill({
      body: JSON.stringify({ status: 'unavailable' }),
      contentType: 'application/json',
      status: 503,
    }),
  )

  await page.goto('/')

  await expect(page.getByRole('status')).toContainText('Servidor indisponível')
  await expect(page.getByRole('button', { name: 'Tentar novamente' })).toBeVisible()
})

test('a guest creates a private room and becomes its host', async ({ context, page }) => {
  await page.goto('/')

  await page.getByLabel('Seu nome').fill('Minerva')
  await page.getByLabel('Senha de recuperação').fill('a long uncommon passphrase')
  await page.getByRole('button', { name: 'Criar sala privada' }).click()

  await expect(page.getByRole('heading', { level: 2, name: 'Sala pronta' })).toBeVisible()
  await expect(page.getByText(/^[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{8}$/)).toBeVisible()
  await expect(page.locator('.room-details').getByText('Minerva', { exact: true })).toBeVisible()
  await expect(page.getByText('Protegida neste navegador')).toBeVisible()

  const session = (await context.cookies()).find((cookie) => cookie.name === '__Host-session')
  expect(session).toMatchObject({ httpOnly: true, sameSite: 'Strict', secure: true })
})

test('a guest joins with an available hero and keeps the same position after reload', async ({
  browser,
  page,
}) => {
  await page.goto('/')
  await page.getByLabel('Seu nome').fill('Minerva')
  await page.getByLabel('Senha de recuperação').fill('a long uncommon passphrase')
  await page.getByRole('button', { name: 'Criar sala privada' }).click()
  const roomCode = await page.locator('output').textContent()
  expect(roomCode).toMatch(/^[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{8}$/)

  const guestContext = await browser.newContext()
  const guestPage = await guestContext.newPage()
  try {
    await guestPage.goto('/')
    await guestPage.getByRole('button', { name: 'Entrar em uma sala' }).click()
    await guestPage.getByLabel('Código da sala').fill(roomCode ?? '')
    await guestPage.getByRole('button', { name: 'Localizar sala' }).click()
    await guestPage.getByLabel('Seu nome').fill('Luna')
    await guestPage.getByRole('radio', { name: 'Hermione' }).check()
    await guestPage.getByRole('button', { name: 'Entrar na sala' }).click()

    await expect(guestPage.getByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()
    await expect(guestPage.locator('.room-details').getByText('Posição 2', { exact: true })).toBeVisible()
    await expect(guestPage.locator('.room-details').getByText('Hermione', { exact: true })).toBeVisible()

    await guestPage.reload()

    await expect(guestPage.getByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()
    await expect(guestPage.locator('.room-details').getByText('Posição 2', { exact: true })).toBeVisible()
    await expect(guestPage.locator('.room-details').getByText('Hermione', { exact: true })).toBeVisible()
  } finally {
    await guestContext.close()
  }
})

test('a participant explicitly replaces one of two sessions when recovering on a third device', async ({
  browser,
  page,
}) => {
  await page.goto('/')
  await page.getByLabel('Seu nome').fill('Minerva')
  await page.getByLabel('Senha de recuperação').fill('a long uncommon passphrase')
  await page.getByRole('button', { name: 'Criar sala privada' }).click()
  const recoveryLink = await page.getByLabel('Link de recuperação').inputValue()
  expect(recoveryLink).toMatch(/#recovery=[0-9a-f]{64}$/)

  let successorRecoveryLink = ''
  const secondDevice = await browser.newContext()
  const recoveredPage = await secondDevice.newPage()
  try {
    await recoveredPage.goto(recoveryLink)
    await expect(recoveredPage).toHaveURL((url) => url.hash === '')
    await expect(
      recoveredPage.getByRole('heading', { level: 2, name: 'Recupere sua participação' }),
    ).toBeVisible()
    await recoveredPage
      .getByLabel('Senha de recuperação da sala')
      .fill('a long uncommon passphrase')
    await recoveredPage.getByRole('button', { name: 'Recuperar minha posição' }).click()

    await expect(recoveredPage.getByRole('heading', { level: 2, name: 'Sala pronta' })).toBeVisible()
    await expect(
      recoveredPage.locator('.room-details').getByText('Posição 1', { exact: true }),
    ).toBeVisible()
    successorRecoveryLink = await recoveredPage.getByLabel('Link de recuperação').inputValue()
    expect(successorRecoveryLink).toMatch(/#recovery=[0-9a-f]{64}$/)
    const recoveredSession = (await secondDevice.cookies()).find(
      (cookie) => cookie.name === '__Host-session',
    )
    expect(recoveredSession).toMatchObject({ httpOnly: true, sameSite: 'Strict', secure: true })
  } finally {
    await secondDevice.close()
  }

  const replayDevice = await browser.newContext()
  const replayPage = await replayDevice.newPage()
  try {
    await replayPage.goto(recoveryLink)
    await replayPage
      .getByLabel('Senha de recuperação da sala')
      .fill('a long uncommon passphrase')
    await replayPage.getByRole('button', { name: 'Recuperar minha posição' }).click()
    await expect(
      replayPage.getByText(
        'Não foi possível recuperar a participação. Confira o link e a senha da sala.',
      ),
    ).toBeVisible()
  } finally {
    await replayDevice.close()
  }

  const thirdDevice = await browser.newContext()
  const replacementPage = await thirdDevice.newPage()
  try {
    await replacementPage.goto(successorRecoveryLink)
    await replacementPage
      .getByLabel('Senha de recuperação da sala')
      .fill('a long uncommon passphrase')
    await replacementPage.getByRole('button', { name: 'Recuperar minha posição' }).click()

    await expect(
      replacementPage.getByRole('heading', {
        level: 2,
        name: 'Escolha uma sessão para substituir',
      }),
    ).toBeVisible()
    await expect(replacementPage.getByRole('radio', { name: 'Sessão 1' })).toBeVisible()
    await expect(replacementPage.getByRole('radio', { name: 'Sessão 2' })).toBeVisible()
    await replacementPage.getByRole('radio', { name: 'Sessão 1' }).check()
    await replacementPage.getByRole('button', { name: 'Substituir Sessão 1' }).click()

    await expect(
      replacementPage.getByRole('heading', { level: 2, name: 'Sala pronta' }),
    ).toBeVisible()
    await expect(replacementPage.getByLabel('Link de recuperação')).toHaveValue(
      /#recovery=[0-9a-f]{64}$/,
    )
  } finally {
    await thirdDevice.close()
  }

  const revokedSession = await page.request.get('/api/session')
  expect(revokedSession.status()).toBe(401)
  await page.reload()
  await expect(
    page.getByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' }),
  ).toBeVisible()
})

test('recovery rotation preserves sessions and replaces direct and assisted credentials', async ({
  browser,
  page: hostPage,
}) => {
  const currentPassword = 'a long uncommon passphrase'
  const newPassword = 'a different uncommon passphrase'
  const guestContext = await browser.newContext()
  const guestPage = await guestContext.newPage()

  try {
    await hostPage.goto('/')
    await hostPage.getByLabel('Seu nome').fill('Minerva')
    await hostPage.getByLabel('Senha de recuperação').fill(currentPassword)
    await hostPage.getByRole('button', { name: 'Criar sala privada' }).click()
    const roomCode = await hostPage.locator('output').textContent()
    const originalRecoveryLink = await hostPage.getByLabel('Link de recuperação').inputValue()

    await guestPage.goto('/')
    await guestPage.getByRole('button', { name: 'Entrar em uma sala' }).click()
    await guestPage.getByLabel('Código da sala').fill(roomCode ?? '')
    await guestPage.getByRole('button', { name: 'Localizar sala' }).click()
    await guestPage.getByLabel('Seu nome').fill('Luna')
    await guestPage.getByRole('radio', { name: 'Hermione' }).check()
    await guestPage.getByRole('button', { name: 'Entrar na sala' }).click()
    await expect(guestPage.getByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()

    await hostPage.reload()
    await expect(hostPage.getByText('Luna', { exact: true })).toBeVisible()
    await hostPage.locator('details.recovery-management > summary').click()
    await hostPage.getByLabel('Senha atual da sala').fill(currentPassword)
    await hostPage.getByLabel('Nova senha de recuperação').fill(newPassword)
    await hostPage.getByLabel('Confirmar nova senha').fill(newPassword)
    await hostPage.getByRole('button', { name: 'Alterar senha da sala' }).click()

    await expect(hostPage.getByText('Senha da sala alterada.', { exact: true })).toBeVisible()
    await expect(
      guestPage.getByText(
        'A senha de recuperação foi alterada. Suas sessões continuam ativas.',
        { exact: true },
      ),
    ).toBeVisible()
    await expect(hostPage.getByRole('heading', { level: 2, name: 'Sala pronta' })).toBeVisible()
    await expect(guestPage.getByRole('heading', { level: 2, name: 'Sala aberta' })).toBeVisible()

    const obsoleteLinkContext = await browser.newContext()
    const obsoleteLinkPage = await obsoleteLinkContext.newPage()
    try {
      await obsoleteLinkPage.goto(originalRecoveryLink)
      await obsoleteLinkPage.getByLabel('Senha de recuperação da sala').fill(newPassword)
      await obsoleteLinkPage.getByRole('button', { name: 'Recuperar minha posição' }).click()
      await expect(
        obsoleteLinkPage.getByText(
          'Não foi possível recuperar a participação. Confira o link e a senha da sala.',
        ),
      ).toBeVisible()
    } finally {
      await obsoleteLinkContext.close()
    }

    await hostPage.getByRole('button', { name: 'Gerar novo link para mim' }).click()
    const directRecoveryLink = await hostPage.getByLabel('Link de recuperação').inputValue()
    expect(directRecoveryLink).not.toBe(originalRecoveryLink)

    const recoveredContext = await browser.newContext()
    const recoveredPage = await recoveredContext.newPage()
    try {
      await recoveredPage.goto(directRecoveryLink)
      await recoveredPage.getByLabel('Senha de recuperação da sala').fill(newPassword)
      await recoveredPage.getByRole('button', { name: 'Recuperar minha posição' }).click()
      await expect(
        recoveredPage.getByRole('heading', { level: 2, name: 'Sala pronta' }),
      ).toBeVisible()
    } finally {
      await recoveredContext.close()
    }

    await hostPage.getByRole('button', { name: 'Já guardei o link' }).click()
    await hostPage.locator('details.assisted-recovery > summary').click()
    await expect(hostPage.getByText('Risco de personificação', { exact: true })).toBeVisible()
    await hostPage
      .getByLabel('Participante sem acesso')
      .selectOption({ label: 'Posição 2 · Luna' })
    await hostPage
      .getByLabel('Entendo que o link permite personificar este participante')
      .check()
    await hostPage.getByRole('button', { name: 'Gerar link com assistência' }).click()
    await expect(
      hostPage.getByRole('heading', { level: 3, name: 'Novo link emitido para Luna.' }),
    ).toBeVisible()
  } finally {
    await guestContext.close()
  }
})

test('a player replays a missed event and falls back to Snapshot within recovery SLOs', async ({
  browser,
  page: hostPage,
}) => {
  const guestContext = await browser.newContext()
  const guestPage = await guestContext.newPage()
  await guestPage.addInitScript(() => {
    type BrowserSocket = {
      addEventListener: (
        type: 'message',
        listener: (event: { data: unknown }) => void,
      ) => void
      close: (code?: number, reason?: string) => void
    }
    type BrowserSocketConstructor = new (
      url: string | URL,
      protocols?: string | string[],
    ) => BrowserSocket
    const scope = globalThis as unknown as {
      WebSocket: BrowserSocketConstructor
      __e2eForceBadDigest: boolean
      __e2eMessages: string[]
      __e2eSockets: BrowserSocket[]
    }
    const NativeWebSocket = scope.WebSocket
    const messages: string[] = []
    const observed: BrowserSocket[] = []
    class ObservedWebSocket extends NativeWebSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        let requestedUrl = url
        if (scope.__e2eForceBadDigest) {
          const incompatible = new URL(url)
          incompatible.searchParams.set('digest', `blake3:${'0'.repeat(64)}`)
          requestedUrl = incompatible
          scope.__e2eForceBadDigest = false
        }
        super(requestedUrl, protocols)
        observed.push(this)
        this.addEventListener('message', (event) => {
          const message = JSON.parse(String(event.data)) as { type?: unknown }
          if (typeof message.type === 'string') {
            messages.push(message.type)
          }
        })
      }
    }
    scope.__e2eForceBadDigest = false
    scope.__e2eMessages = messages
    scope.__e2eSockets = observed
    scope.WebSocket = ObservedWebSocket
  })

  try {
    await hostPage.goto('/')
    await hostPage.getByLabel('Seu nome').fill('Minerva')
    await hostPage.getByLabel('Senha de recuperação').fill('a long uncommon passphrase')
    await hostPage.getByRole('button', { name: 'Criar sala privada' }).click()
    const roomCode = await hostPage.locator('output').textContent()

    await hostPage.getByRole('radio', { name: 'Harry' }).check()
    await hostPage.getByRole('button', { name: 'Confirmar Herói' }).click()
    await hostPage.getByRole('button', { name: 'Estou pronto' }).click()

    await guestPage.goto('/')
    await guestPage.getByRole('button', { name: 'Entrar em uma sala' }).click()
    await guestPage.getByLabel('Código da sala').fill(roomCode ?? '')
    await guestPage.getByRole('button', { name: 'Localizar sala' }).click()
    await guestPage.getByLabel('Seu nome').fill('Luna')
    await guestPage.getByRole('radio', { name: 'Hermione' }).check()
    await guestPage.getByRole('button', { name: 'Entrar na sala' }).click()
    await guestPage.getByRole('button', { name: 'Estou pronto' }).click()

    await hostPage.getByRole('button', { name: 'Atualizar estado da sala' }).click()
    await hostPage.getByRole('button', { name: 'Selar sala e iniciar' }).click()
    await guestPage.getByRole('button', { name: 'Atualizar estado da sala' }).click()

    await expect(hostPage.getByText('Atualizações em tempo real conectadas.')).toBeVisible()
    await expect(guestPage.getByText('Atualizações em tempo real conectadas.')).toBeVisible()
    await expect(hostPage.locator('.presence-label--online')).toHaveCount(2)
    await expect(guestPage.locator('.presence-label--online')).toHaveCount(2)

    await guestContext.setOffline(true)
    await guestPage.evaluate(() => {
      const sockets = (
        globalThis as unknown as {
          __e2eSockets: Array<{ close: (code?: number, reason?: string) => void }>
        }
      ).__e2eSockets
      sockets.at(-1)?.close(1000, 'E2E network change')
    })
    await expect(guestPage.locator('.realtime-status')).toContainText(
      /Reconectando|interrompidas/,
    )
    await expect(hostPage.locator('.presence-label--reconnecting')).toContainText('Reconectando')
    await expect(hostPage.locator('.presence-block')).toHaveCount(0)
    await hostPage.getByRole('button', { name: 'Concluir Artes das Trevas' }).click()
    await expect(
      hostPage.getByRole('heading', { level: 3, name: 'Última resolução oficial' }),
    ).toBeVisible()
    await expect(hostPage.getByText('Minerva pagou 1 de Vida (10 → 9).')).toBeVisible()
    await expect(hostPage.getByText('Minerva recebeu 2 de Influência (0 → 2).')).toBeVisible()
    await expect(hostPage.getByText(/Dado D4: resultado [1-4]\./)).toBeVisible()
    await expect(
      hostPage.getByText('Nenhum alvo era elegível. O efeito foi resolvido sem alterar a mesa.'),
    ).toBeVisible()
    await expect(hostPage.getByText('Vida 9 · Ataque 2 · Influência 2')).toBeVisible()
    await guestContext.setOffline(false)

    const replayStartedAt = Date.now()
    await expect(guestPage.getByText('Ação do Herói')).toBeVisible({ timeout: 3_000 })
    await expect(
      guestPage.getByRole('heading', { level: 3, name: 'Última resolução oficial' }),
    ).toBeVisible()
    await expect(guestPage.getByText(/Dado D4: resultado [1-4]\./)).toBeVisible()
    expect(Date.now() - replayStartedAt).toBeLessThan(3_000)
    await expect
      .poll(
        () =>
          guestPage.evaluate(() =>
            (globalThis as unknown as { __e2eMessages: string[] }).__e2eMessages.includes('events'),
          ),
        { timeout: 3_000 },
      )
      .toBe(true)
    await expect(guestPage.getByText('Atualizações em tempo real conectadas.')).toBeVisible()

    await guestPage.evaluate(() => {
      const scope = globalThis as unknown as {
        __e2eForceBadDigest: boolean
        __e2eSockets: Array<{ close: (code?: number, reason?: string) => void }>
      }
      scope.__e2eForceBadDigest = true
      scope.__e2eSockets.at(-1)?.close(1000, 'E2E incompatible digest')
    })
    const snapshotStartedAt = Date.now()
    await expect
      .poll(
        () =>
          guestPage.evaluate(() =>
            (globalThis as unknown as { __e2eMessages: string[] }).__e2eMessages.includes(
              'snapshot',
            ),
          ),
        { timeout: 5_000 },
      )
      .toBe(true)
    expect(Date.now() - snapshotStartedAt).toBeLessThan(5_000)
    await expect(guestPage.getByText('Atualizações em tempo real conectadas.')).toBeVisible()
  } finally {
    await guestContext.close()
  }
})

test('the current interface reflows at an effective 200 percent zoom', async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 900 })
  await page.goto('/')

  await expect(
    page.getByRole('heading', { level: 2, name: 'Abra uma sala para o seu grupo' }),
  ).toBeVisible()
  const overflow = await page.evaluate(
    () => {
      const browserDocument = (
        globalThis as unknown as {
          document: { documentElement: { clientWidth: number; scrollWidth: number } }
        }
      ).document
      return browserDocument.documentElement.scrollWidth - browserDocument.documentElement.clientWidth
    },
  )
  expect(overflow).toBeLessThanOrEqual(1)
})

test('a stale command resynchronizes before the player can decide again', async ({ page }) => {
  let sessionRequests = 0
  let commandRequests = 0
  await page.addInitScript(() => {
    localStorage.setItem('hogwarts.session.expected', 'true')
    class SynchronizedSocket {
      static readonly CONNECTING = 0
      static readonly OPEN = 1
      static readonly CLOSED = 3

      readonly url: string
      readonly requestedProtocol: string
      protocol = ''
      readyState = SynchronizedSocket.CONNECTING
      onopen: (() => void) | null = null
      onmessage: ((event: { data: string }) => void) | null = null
      onerror: (() => void) | null = null
      onclose: ((event: { code: number }) => void) | null = null

      constructor(url: string | URL, protocol: string | string[]) {
        this.url = String(url)
        this.requestedProtocol = Array.isArray(protocol) ? (protocol[0] ?? '') : protocol
        queueMicrotask(() => {
          this.readyState = SynchronizedSocket.OPEN
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

      close(): void {
        this.readyState = SynchronizedSocket.CLOSED
        this.onclose?.({ code: 1000 })
      }
    }
    ;(globalThis as unknown as { WebSocket: typeof SynchronizedSocket }).WebSocket =
      SynchronizedSocket
  })
  await page.route('**/api/session', async (route) => {
    sessionRequests += 1
    await route.fulfill({
      body: JSON.stringify(
        sessionRequests === 1 ? initialGameProjection : completedGameProjection,
      ),
      contentType: 'application/json',
      status: 200,
    })
  })
  await page.route('**/api/games/current/commands', async (route) => {
    commandRequests += 1
    await route.fulfill({
      body: JSON.stringify(errorResponse('STALE_STATE_VERSION')),
      contentType: 'application/json',
      status: 409,
    })
  })

  await page.goto('/')
  await expect(page.getByRole('heading', { level: 2, name: 'Partida iniciada' })).toBeVisible()
  await page.getByRole('button', { name: 'Concluir Artes das Trevas' }).click()

  await expect(page.getByText('Estado oficial atualizado')).toBeVisible()
  await expect(page.getByText('Ação do Herói')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Concluir Artes das Trevas' })).toHaveCount(0)
  expect(sessionRequests).toBe(2)
  expect(commandRequests).toBe(1)
})
