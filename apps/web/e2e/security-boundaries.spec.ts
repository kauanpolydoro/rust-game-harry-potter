import { expect, test } from '@playwright/test'

test('a modified browser cannot bypass CSRF, strict schemas or server session identity', async ({ page, context, browser }) => {
  await page.goto('/')
  await page.getByLabel('Seu nome').fill('Minerva')
  await page.getByLabel('Senha de recuperação').fill('a long uncommon security passphrase')
  await page.getByRole('button', { name: 'Criar sala privada' }).click()
  await expect(page.locator('output')).toBeVisible()
  const origin = new URL(page.url()).origin
  const cookie = (await context.cookies()).find((item) => item.name === '__Host-session')
  expect(cookie).toMatchObject({ httpOnly: true, secure: true, sameSite: 'Strict', path: '/' })
  expect(await page.evaluate('document.cookie')).not.toContain('__Host-session')

  const readSession = () => page.evaluate(async () => {
    const response = await fetch('/api/session')
    return { status: response.status, body: await response.json() }
  })
  const before = await readSession()
  expect(before.status).toBe(200)
  const missingCsrf = await page.evaluate(async () => {
    const response = await fetch('/api/session/readiness', {
      method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ready: true }),
    })
    return { status: response.status, body: await response.json() }
  })
  expect(missingCsrf).toMatchObject({ status: 403, body: { error: { code: 'CSRF_REQUIRED' } } })
  const forged = await page.request.put('/api/session/readiness', {
    headers: { Origin: origin, 'x-csrf-protection': '1', Cookie: `__Host-session=${cookie?.value}` },
    data: { ready: true, participant_id: 'another-participant', role: 'host' },
  })
  expect(forged.status()).toBe(422)
  const foreign = await page.request.put('/api/session/readiness', {
    headers: { Origin: 'https://attacker.invalid', 'x-csrf-protection': '1', Cookie: `__Host-session=${cookie?.value}` }, data: { ready: true },
  })
  expect(foreign.status()).toBe(403)
  const preflight = await page.request.fetch('/api/session/readiness', {
    method: 'OPTIONS', headers: { Origin: 'http://localhost:9999', 'Access-Control-Request-Method': 'PUT', 'Access-Control-Request-Headers': 'x-csrf-protection' },
  })
  expect(preflight.headers()['access-control-allow-origin']).toBeUndefined()
  const anonymous = await browser.newContext({ ignoreHTTPSErrors: true })
  try {
    const response = await anonymous.request.put(`${origin}/api/session/readiness`, {
      headers: { Origin: origin, 'x-csrf-protection': '1' }, data: { ready: true },
    })
    expect(response.status()).toBe(401)
  } finally {
    await anonymous.close()
  }
  expect(await readSession()).toEqual(before)
  await page.getByRole('radio', { name: 'Harry', exact: true }).check()
  await page.getByRole('button', { name: 'Confirmar Herói' }).click()
  await page.getByRole('button', { name: 'Estou pronto' }).click()
  await expect(page.getByRole('button', { name: 'Reabrir minha preparação' })).toBeVisible()
})

test('recovery document enforces CSP and clears its credential before application requests', async ({ page }) => {
  const token = 'a'.repeat(64)
  const requests: string[] = []
  page.on('request', (request) => requests.push(request.url()))
  const response = await page.goto(`/#recovery=${token}`)
  expect(response?.headers()['cache-control']).toBe('no-store')
  expect(response?.headers()['referrer-policy']).toBe('no-referrer')
  const csp = response?.headers()['content-security-policy'] ?? ''
  for (const directive of ["script-src 'self'", "frame-ancestors 'none'", "object-src 'none'", "base-uri 'none'", "form-action 'self'"]) {
    expect(csp).toContain(directive)
  }
  expect(csp).not.toMatch(/unsafe-inline|unsafe-eval/)
  await expect(page.getByRole('heading', { name: 'Recupere sua participação' })).toBeVisible()
  expect(new URL(page.url()).hash).toBe('')
  expect(requests.every((url) => !url.includes(token))).toBe(true)
  const inlineExecuted = await page.evaluate(`(() => {
    const script = document.createElement('script')
    script.textContent = 'document.documentElement.dataset.unsafeScript = "executed"'
    document.body.append(script)
    return document.documentElement.dataset.unsafeScript
  })()`)
  expect(inlineExecuted).toBeUndefined()
})

test('recovery tells the player when to retry after the real credential budget is exhausted', async ({ page, browser }) => {
  await page.goto('/')
  await page.getByLabel('Seu nome').fill('Minerva')
  await page.getByLabel('Senha de recuperação').fill('a long uncommon recovery passphrase')
  await page.getByRole('button', { name: 'Criar sala privada' }).click()
  const link = page.getByLabel('Link de recuperação')
  await expect(link).toBeVisible()
  const recoveryContext = await browser.newContext({ ignoreHTTPSErrors: true })
  try {
    const recovery = await recoveryContext.newPage()
    await recovery.goto(await link.inputValue())
    await recovery.getByLabel('Senha de recuperação da sala').fill('an incorrect recovery passphrase')
    for (let attempt = 0; attempt < 11; attempt += 1) {
      const result = recovery.waitForResponse((response) => response.url().endsWith('/api/session/recover'))
      await recovery.getByRole('button', { name: 'Recuperar minha posição' }).click()
      expect((await result).status()).toBe(attempt < 10 ? 401 : 429)
    }
    await expect(recovery.getByRole('alert')).toHaveText('Muitas tentativas. Aguarde um minuto antes de tentar novamente com o mesmo link.')
    expect(new URL(recovery.url()).hash).toBe('')
    expect((await recoveryContext.cookies()).some((cookie) => cookie.name === '__Host-session')).toBe(false)
  } finally {
    await recoveryContext.close()
  }
})
