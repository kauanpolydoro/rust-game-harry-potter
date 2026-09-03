import { expect, test } from '@playwright/test'

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
