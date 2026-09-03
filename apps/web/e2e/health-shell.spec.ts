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
      body: JSON.stringify({ error: { code: 'STALE_STATE_VERSION' } }),
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
