import { expect, test } from '@playwright/test'

test('a recovered participant emits anonymous timing without leaking the recovery URL or session', async ({ browser, page }) => {
  await page.goto('/')
  await page.getByLabel('Seu nome').fill('Telemetry private name')
  await page.getByLabel('Senha de recuperação').fill('a long uncommon passphrase')
  await page.getByRole('button', { name: 'Criar sala privada' }).click()
  await expect(page.getByRole('heading', { name: 'Sala pronta' })).toBeVisible()
  const link = await page.getByLabel('Link de recuperação').inputValue()
  const device = await browser.newContext()
  const recovered = await device.newPage()
  const observations: { metric: string, value: number, outcome: string }[] = []
  const privateHeaders: string[] = []
  const statuses: number[] = []
  recovered.on('request', (request) => {
    if (new URL(request.url()).pathname !== '/api/telemetry') return
    observations.push(request.postDataJSON())
    const headers = request.headers()
    privateHeaders.push(headers.cookie ?? '', headers.referer ?? '', headers.authorization ?? '')
  })
  recovered.on('response', (response) => {
    if (new URL(response.url()).pathname === '/api/telemetry') statuses.push(response.status())
  })
  try {
    await recovered.goto(link)
    await expect(recovered).toHaveURL((url) => url.hash === '')
    await recovered.getByLabel('Senha de recuperação da sala').fill('a long uncommon passphrase')
    await recovered.getByRole('button', { name: 'Recuperar minha posição' }).click()
    await expect(recovered.getByRole('heading', { name: 'Sala pronta' })).toBeVisible()
    await expect.poll(() => observations.some(({ metric, outcome }) =>
      metric === 'recovery_human_seconds' && outcome === 'success')).toBe(true)
    await recovered.goto('about:blank')
    await expect.poll(() => observations.some(({ metric }) => metric === 'web_lcp_seconds')).toBe(true)
    await expect.poll(() => statuses.length).toBe(observations.length)
    expect(statuses.every((status) => status === 204)).toBe(true)
    expect(privateHeaders.every((value) => value === '')).toBe(true)
    for (const observation of observations) {
      expect(Object.keys(observation).sort()).toEqual(['metric', 'outcome', 'value'])
      expect(Number.isFinite(observation.value) && observation.value >= 0).toBe(true)
    }
    const serialized = JSON.stringify(observations)
    for (const canary of ['Telemetry private name', 'a long uncommon passphrase', new URL(link).hash.slice(10)]) {
      expect(serialized).not.toContain(canary)
    }
  } finally {
    await device.close()
  }
})
