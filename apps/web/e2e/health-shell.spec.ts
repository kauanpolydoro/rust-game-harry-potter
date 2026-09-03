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
  await expect(page.getByText('Minerva')).toBeVisible()
  await expect(page.getByText('Protegida neste navegador')).toBeVisible()

  const session = (await context.cookies()).find((cookie) => cookie.name === '__Host-session')
  expect(session).toMatchObject({ httpOnly: true, sameSite: 'Strict', secure: true })
})
