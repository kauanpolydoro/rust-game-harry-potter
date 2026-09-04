import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { expect, test } from '@playwright/test'

const executeFile = promisify(execFile)

test('expiration clears private data, pending commands and channels in every connected browser', async ({
  browser,
  context,
  page: host,
}) => {
  const guestContext = await browser.newContext()
  const guest = await guestContext.newPage()
  const sockets = new Set<unknown>()
  host.on('websocket', (socket) => {
    sockets.add(socket)
    socket.on('close', () => sockets.delete(socket))
  })
  let releaseCommand = (): void => undefined
  const commandGate = new Promise<void>((resolve) => { releaseCommand = resolve })
  try {
    await host.goto('/')
    await host.getByLabel('Seu nome').fill('Minerva')
    await host.getByLabel('Senha de recuperação').fill('a long uncommon passphrase')
    await host.getByRole('button', { name: 'Criar sala privada' }).click()
    const roomCode = await host.locator('output').textContent()
    expect(roomCode).toMatch(/^[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{8}$/)
    const recoveryLink = await host.getByLabel('Link de recuperação').inputValue()
    await host.getByRole('radio', { name: 'Harry' }).check()
    await host.getByRole('button', { name: 'Confirmar Herói' }).click()
    await host.getByRole('button', { name: 'Estou pronto' }).click()

    await guest.goto('/')
    await guest.getByRole('button', { name: 'Entrar em uma sala' }).click()
    await guest.getByLabel('Código da sala').fill(roomCode ?? '')
    await guest.getByRole('button', { name: 'Localizar sala' }).click()
    await guest.getByLabel('Seu nome').fill('Luna')
    await guest.getByRole('radio', { name: 'Hermione' }).check()
    await guest.getByRole('button', { name: 'Entrar na sala' }).click()
    await guest.getByRole('button', { name: 'Estou pronto' }).click()
    await host.getByRole('button', { name: 'Atualizar estado da sala' }).click()
    await host.getByRole('button', { name: 'Selar sala e iniciar' }).click()
    await guest.getByRole('button', { name: 'Atualizar estado da sala' }).click()
    const secondTab = await context.newPage()
    await secondTab.goto('/')
    for (const page of [host, guest, secondTab]) {
      await expect(page.getByText('Atualizações em tempo real conectadas.')).toBeVisible()
    }

    await host.route('**/api/games/current/commands', async (route) => {
      await commandGate
      await route.continue()
    })
    await host.getByRole('button', { name: 'Encerrar ações do Herói' }).click()
    await expect(host.getByText('Intenção pendente')).toBeVisible()
    expect(await host.evaluate(() => sessionStorage.getItem('hogwarts.game-command.pending-intent'))).not.toBeNull()

    await guestContext.setOffline(true)

    // Change only this test's game in the real PostgreSQL database. The worker,
    // authorization, WebSockets and UI continue running without mocked responses.
    if (!roomCode || !/^[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{8}$/.test(roomCode)) {
      throw new Error('the test room code must be safe to use in the fixture query')
    }
    const database = new URL(process.env.TEST_DATABASE_URL ??
      'postgres://hogwarts:local-development-only@127.0.0.1:55432/hogwarts').pathname.slice(1)
    await executeFile('docker', ['compose', 'exec', '-T', 'postgres', 'psql',
      '-U', 'hogwarts', '-d', database, '-v', 'ON_ERROR_STOP=1', '-c',
      `UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '8 days', expires_at = clock_timestamp() - INTERVAL '1 day' WHERE room_id = (SELECT id FROM rooms WHERE code = '${roomCode}')`,
    ])
    await expect(host.getByRole('heading', { name: 'Partida expirada' })).toBeVisible()
    await guestContext.setOffline(false)
    for (const page of [host, guest, secondTab]) {
      await expect(page.getByRole('heading', { name: 'Partida expirada' })).toBeVisible()
      await expect(page.getByRole('heading', { name: 'Sua fase de ação' })).toHaveCount(0)
      await expect(page.getByLabel('Link de recuperação')).toHaveCount(0)
      expect(await page.evaluate(() => localStorage.getItem('hogwarts.session.expected'))).toBeNull()
      expect(await page.evaluate(() => sessionStorage.getItem('hogwarts.game-command.pending-intent'))).toBeNull()
    }
    await expect.poll(() => sockets.size).toBe(0)
    releaseCommand()
    await expect.poll(async () => (await context.cookies()).some((cookie) => cookie.name === '__Host-session')).toBe(false)

    await host.screenshot({ path: test.info().outputPath('expired-game.png'), fullPage: true })
    await host.reload()
    await expect(host.getByRole('button', { name: 'Criar sala privada' })).toBeVisible()
    await expect(host.getByText('Intenção pendente')).toHaveCount(0)
    const recoveryPage = await guestContext.newPage()
    await recoveryPage.goto(recoveryLink)
    await recoveryPage.getByLabel('Senha de recuperação da sala').fill('a long uncommon passphrase')
    await recoveryPage.getByRole('button', { name: 'Recuperar minha posição' }).click()
    await expect(recoveryPage.getByText('Não foi possível recuperar a participação. Confira o link e a senha da sala.')).toBeVisible()
  } finally {
    releaseCommand()
    await guestContext.close()
  }
})
