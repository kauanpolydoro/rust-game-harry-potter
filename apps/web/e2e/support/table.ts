import { expect, type Browser, type Page } from '@playwright/test'
import { isGameProjectionResponse, isRealtimeEventBatchMessage, type GameProjectionResponse } from '../../src/contracts/identity-access.generated'

export class ObservedPlayer {
  projection?: GameProjectionResponse
  eventBatches = 0
  errors: string[] = []

  constructor(readonly page: Page) {
    page.on('pageerror', (error) => this.errors.push(error.message))
    page.on('response', async (response) => {
      if (response.url().includes('/api/') && response.ok()) {
        this.observe(await response.json().catch(() => null))
      }
    })
    page.on('websocket', (socket) => socket.on('framereceived', ({ payload }) => {
      // Firefox's Juggler observer exposes text frames as byte strings even
      // though the browser's MessageEvent.data has already decoded UTF-8.
      const text = typeof payload === 'string' && page.context().browser()?.browserType().name() === 'firefox'
        ? Buffer.from(payload, 'latin1').toString('utf8') : String(payload)
      const message: unknown = JSON.parse(text)
      if (message && typeof message === 'object' && 'type' in message && message.type === 'events' && 'projection' in message) {
        expect(isRealtimeEventBatchMessage(message), JSON.stringify(message)).toBe(true)
        this.eventBatches += 1
      }
      this.observe(message)
    }))
  }

  private observe(message: unknown) {
    const candidate = message && typeof message === 'object' && 'projection' in message ? message.projection : message
    if (isGameProjectionResponse(candidate) && (!this.projection || candidate.snapshot.sequence >= this.projection.snapshot.sequence)) {
      this.projection = candidate
    }
  }

  current(): GameProjectionResponse {
    if (!this.projection) throw new Error('The browser has not received a game projection')
    return this.projection
  }

  async reaches(sequence: number) {
    await expect.poll(() => this.projection?.snapshot.sequence).toBe(sequence)
    await expect(this.page.locator('.snapshot-details')).toContainText(`v${sequence + 1} · sequência ${sequence}`)
  }
}

export async function startTable(browser: Browser, host: ObservedPlayer, count: number, game: 'one' | 'two', mode: 'accessible' | 'visual' = 'accessible') {
  const players = [host]
  await host.page.goto('/')
  await host.page.getByLabel('Seu nome').fill('Harry')
  await host.page.getByLabel('Senha de recuperação').fill('a long uncommon game one passphrase')
  await host.page.getByRole('button', { name: 'Criar sala privada' }).click()
  const code = await host.page.locator('output').textContent()
  await host.page.getByRole('radio', { name: 'Harry', exact: true }).check()
  await host.page.getByRole('button', { name: 'Confirmar Herói' }).click()
  await host.page.getByRole('button', { name: 'Estou pronto' }).click()
  await expect(host.page.getByRole('button', { name: 'Atualizar estado da sala', exact: true })).toBeEnabled()
  for (const hero of ['Hermione', 'Rony', 'Neville'].slice(0, count - 1)) {
    const context = await browser.newContext({ ignoreHTTPSErrors: true })
    const player = new ObservedPlayer(await context.newPage())
    players.push(player)
    await player.page.goto('/')
    await player.page.getByRole('button', { name: 'Entrar em uma sala' }).click()
    await player.page.getByLabel('Código da sala').fill(code ?? '')
    await player.page.getByRole('button', { name: 'Localizar sala' }).click()
    await player.page.getByLabel('Seu nome').fill(hero)
    await player.page.getByRole('radio', { name: hero, exact: true }).check()
    await player.page.getByRole('button', { name: 'Entrar na sala' }).click()
    await player.page.getByRole('button', { name: 'Estou pronto' }).click()
    await expect(player.page.getByRole('button', { name: 'Atualizar estado da sala', exact: true })).toBeEnabled()
  }
  await host.page.getByRole('button', { name: 'Atualizar estado da sala' }).click()
  const selection = host.page.getByLabel('Aventura e conteúdo da partida')
  const option = selection.locator('option').filter({ hasText: new RegExp(` · game-${game}-v1$`) })
  await expect(option).toHaveCount(1)
  await selection.selectOption(await option.getAttribute('value') ?? '')
  await host.page.getByRole('button', { name: 'Selar sala e iniciar' }).click()
  await host.reaches(0)
  if (mode === 'accessible') await host.page.getByRole('button', { name: 'Modo acessível', exact: true }).click()
  for (const guest of players.slice(1)) {
    await guest.page.getByRole('button', { name: 'Atualizar estado da sala' }).click()
    await guest.reaches(0)
    if (mode === 'accessible') {
      await guest.page.getByRole('button', { name: 'Modo acessível', exact: true }).click()
      await expect(guest.page.getByText('Atualizações em tempo real conectadas.')).toBeVisible()
    }
  }
  for (const player of players) {
    await player.reaches(0)
    await expect(player.page.locator('.presence-label--online')).toHaveCount(count)
    if (mode === 'accessible') await expect(player.page.getByRole('region', { name: 'Arte das Trevas revelada' })).toBeVisible()
    const state = player.current()
    expect(state.snapshot.versions.content).toBe(`game-${game}-en-v1`)
    expect(state.table.market).toHaveLength(6)
    expect(state.table.hand.every((card) => Boolean(card.description))).toBe(true)
  }
  return players
}
