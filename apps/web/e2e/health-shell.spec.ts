import { expect, test } from '@playwright/test'

test('a player sees when the authoritative service is ready', async ({ page }) => {
  await page.goto('/')

  await expect(page.getByRole('heading', { level: 1, name: 'Batalha de Hogwarts' })).toBeVisible()
  await expect(page.getByRole('status')).toContainText('Servidor pronto')
  await expect(page.getByRole('heading', { level: 2, name: 'Servidor pronto' })).toBeVisible()
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
