import { expect, test } from '@playwright/test'

const initialGameProjection = {
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

const completedGameProjection = {
  ...initialGameProjection,
  game: { ...initialGameProjection.game, expires_at: '2026-09-10T13:00:00Z' },
  legal_actions: [],
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

test('two players synchronize a committed command and reconnect without reloading', async ({
  browser,
  page: hostPage,
}) => {
  const guestContext = await browser.newContext()
  const guestPage = await guestContext.newPage()
  await guestPage.addInitScript(() => {
    type BrowserSocket = { close: (code?: number, reason?: string) => void }
    type BrowserSocketConstructor = new (
      url: string | URL,
      protocols?: string | string[],
    ) => BrowserSocket
    const scope = globalThis as unknown as {
      WebSocket: BrowserSocketConstructor
      __e2eSockets: BrowserSocket[]
    }
    const NativeWebSocket = scope.WebSocket
    const observed: BrowserSocket[] = []
    class ObservedWebSocket extends NativeWebSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        super(url, protocols)
        observed.push(this)
      }
    }
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
    await hostPage.getByRole('button', { name: 'Concluir Artes das Trevas' }).click()
    await expect(guestPage.getByText('Ação do Herói')).toBeVisible()

    await guestPage.evaluate(() => {
      const sockets = (
        globalThis as unknown as {
          __e2eSockets: Array<{ close: (code?: number, reason?: string) => void }>
        }
      ).__e2eSockets
      sockets.at(-1)?.close(1000, 'E2E reconnect')
    })
    await expect(guestPage.getByText('Reconectando atualizações em tempo real.')).toBeVisible()
    await expect
      .poll(
        () =>
          guestPage.evaluate(
            () =>
              (globalThis as unknown as { __e2eSockets: Array<unknown> }).__e2eSockets.length,
          ),
        { timeout: 10_000 },
      )
      .toBeGreaterThanOrEqual(2)
    await expect(guestPage.getByText('Atualizações em tempo real conectadas.')).toBeVisible({
      timeout: 10_000,
    })
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
  await page.addInitScript(() => localStorage.setItem('hogwarts.session.expected', 'true'))
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
