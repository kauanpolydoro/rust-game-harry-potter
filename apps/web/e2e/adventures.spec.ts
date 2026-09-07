import { expect, test, type Browser, type Page } from '@playwright/test'
import { writeFile } from 'node:fs/promises'
import gameTwoPurchasePriority from '../../server/tests/fixtures/game-two/purchase-priority.json' with { type: 'json' }
import gameThreePurchasePriority from '../../server/tests/fixtures/game-three/purchase-priority.json' with { type: 'json' }

import { isGameProjectionResponse, isRealtimeEventBatchMessage, type GameProjectionResponse } from '../src/contracts/identity-access.generated'
import { isGameProjectionResponse as isPreviousGameProjectionResponse, isRealtimeEventBatchMessage as isPreviousRealtimeEventBatchMessage } from './fixtures/game-two-client-contract'

test.use({ actionTimeout: 10_000 })

class ObservedPlayer {
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
      const message: unknown = JSON.parse(String(payload))
      if (message && typeof message === 'object' && 'type' in message && message.type === 'events' && 'projection' in message) {
        expect(isRealtimeEventBatchMessage(message), JSON.stringify(message)).toBe(true)
        expect(isPreviousRealtimeEventBatchMessage(message), 'previous client accepts the WebSocket event and projection').toBe(true)
        this.eventBatches += 1
      }
      this.observe(message)
    }))
  }

  private observe(message: unknown) {
    const candidate = message && typeof message === 'object' && 'projection' in message ? message.projection : message
    if (isGameProjectionResponse(candidate) && (!this.projection || candidate.snapshot.sequence >= this.projection.snapshot.sequence)) {
      expect(isPreviousGameProjectionResponse(candidate), 'previous client accepts the HTTP and WebSocket projection').toBe(true)
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

async function startTable(browser: Browser, host: ObservedPlayer, count: number, game: 'one' | 'two' | 'three') {
  const players = [host]
  await host.page.goto('/')
  await host.page.getByLabel('Seu nome').fill('Harry')
  await host.page.getByLabel('Senha de recuperação').fill('a long uncommon game one passphrase')
  await host.page.getByRole('button', { name: 'Criar sala privada' }).click()
  const code = await host.page.locator('output').textContent()
  await host.page.getByRole('radio', { name: 'Harry', exact: true }).check()
  await host.page.getByRole('button', { name: 'Confirmar Herói' }).click()
  await host.page.getByRole('button', { name: 'Estou pronto' }).click()
  for (const hero of ['Hermione', 'Ron', 'Neville'].slice(0, count - 1)) {
    const context = await browser.newContext()
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
  }
  await host.page.getByRole('button', { name: 'Atualizar estado da sala' }).click()
  const selection = host.page.getByLabel('Aventura e conteúdo da partida')
  const option = selection.locator('option').filter({ hasText: new RegExp(` · game-${game}-v1$`) })
  await expect(option).toHaveCount(1)
  await selection.selectOption(await option.getAttribute('value') ?? '')
  await host.page.getByRole('button', { name: 'Selar sala e iniciar' }).click()
  await host.reaches(0)
  for (const guest of players.slice(1)) {
    await guest.page.getByRole('button', { name: 'Atualizar estado da sala' }).click()
    await guest.reaches(0)
    await expect(guest.page.getByText('Atualizações em tempo real conectadas.')).toBeVisible()
  }
  for (const player of players) {
    await player.reaches(0)
    await expect(player.page.locator('.presence-label--online')).toHaveCount(count)
    await expect(player.page.getByRole('region', { name: 'Arte das Trevas revelada' })).toBeVisible()
    const state = player.current()
    expect(state.snapshot.versions.content).toBe(`game-${game}-en-v1`)
    expect(state.table.market).toHaveLength(6)
    if (game === 'three') {
      expect(state.snapshot.snapshot_version).toBe(7)
      expect(state.table.active_villains).toHaveLength(2)
      expect(new Set(state.participants.map((hero) => hero.hero.id)).size).toBe(count)
    }
    expect(state.table.hand.every((card) => Boolean(card.description))).toBe(true)
  }
  return players
}

const purchasePriority = ['001', '005', '010', '008', '002', '007', '004', '006', '011', '009', '013', '003']

function scenarioTargets(projection: GameProjectionResponse, options: string[], min: number, cause: string): string[] {
  const targets = [...options]
  if (projection.snapshot.versions.content === 'game-three-en-v1' && targets.every((id) => id.startsWith('hero:'))) {
    const priority = (id: string) => {
      const position = Number(id.slice('hero:'.length))
      return ['rule:g3-hero-002-ability', 'rule:g3-hero-005-ability'].includes(cause)
        ? Number(position !== projection.turn.active_position)
        : projection.participants.find((hero) => hero.position === position)?.resources.health ?? 10
    }
    targets.sort((a, b) => priority(a) - priority(b))
  }
  return targets.slice(0, min)
}

async function playThroughInterface(player: ObservedPlayer, seekVictory: boolean) {
  const projection = player.current()
  const { page } = player
  const choice = projection.choice
  if (choice.status === 'pending') {
    if (choice.kind === 'effect') {
      expect(choice.source_name).toBeTruthy()
      expect(choice.option_labels?.map((option) => option.option_id)).toEqual(choice.options)
    }
    if (choice.kind === 'target') {
      expect(choice.instruction).toBeTruthy()
      await expect(page.getByRole('region', { name: 'Escolha oficial pendente' })).toContainText(choice.instruction ?? '')
    }
    const inputs = page.locator('.effect-choice input')
    for (const id of scenarioTargets(projection, choice.options, choice.min, choice.cause)) {
      await inputs.nth(choice.options.indexOf(id)).check()
    }
    await page.getByRole('button', { name: 'Confirmar escolha' }).click()
    return
  }
  const legal = projection.legal_intentions
  if (seekVictory && legal.play_cards.length) {
    const card = legal.play_cards[0]
    const handIndex = projection.table.hand.findIndex((item) => item.instance_id === card.card_id)
    const row = page.getByRole('region', { name: 'Sua mão', exact: true }).getByRole('listitem').nth(handIndex)
    for (const [index, slot] of card.target_slots.entries()) {
      const inputs = row.locator('fieldset').nth(index).locator('input')
      const ids = slot.options.map((option) => option.target_id)
      for (const id of scenarioTargets(projection, ids, slot.min, '')) await inputs.nth(ids.indexOf(id)).check()
    }
    await row.getByRole('button', { name: /^Jogar / }).click()
  } else if (seekVictory && legal.assign_attack.length) {
    await page.getByRole('region', { name: 'Vilões ativos', exact: true }).getByRole('button').first().click()
  } else if (seekVictory && legal.acquire_cards.length) {
    // Match the deterministic HTTP scenario, including its last-card tie break.
    const candidates = legal.acquire_cards.map((card) => ({
      card,
      index: projection.table.market.findIndex((item) => item.instance_id === card.card_id),
    })).sort((a, b) => {
      const priority = (index: number) => {
        const catalog = projection.table.market[index].catalog_id
        const order = projection.snapshot.versions.content === 'game-three-en-v1'
          ? gameThreePurchasePriority[String(projection.participants.length) as keyof typeof gameThreePurchasePriority]
          : projection.snapshot.versions.content === 'game-two-en-v1'
          ? gameTwoPurchasePriority[String(projection.participants.length) as keyof typeof gameTwoPurchasePriority]
          : purchasePriority.map((id) => `hogwarts-card:${id}`)
        const rank = order.indexOf(catalog)
        return rank < 0 ? order.length : rank
      }
      return priority(a.index) - priority(b.index) || b.index - a.index
    })
    const { card, index } = candidates[0]
    const row = page.getByRole('region', { name: 'Mercado de Hogwarts', exact: true }).getByRole('listitem').nth(index)
    if (card.destinations.includes('draw_pile')) await row.getByRole('combobox').selectOption('draw_pile')
    await row.getByRole('button', { name: /^Adquirir / }).click()
  } else {
    await page.getByRole('button', { name: 'Encerrar ações do Herói' }).click()
  }
}

for (const game of ['one', 'two', 'three'] as const) {
  for (const count of [2, 3, 4]) {
    for (const outcome of ['lost', 'won'] as const) {
      test(`Game ${game === 'one' ? 1 : game === 'two' ? 2 : 3} with ${count} players reaches ${outcome} through the browser and WebSocket`, async ({ browser, page }, testInfo) => {
        // Game 3 adds a second active Villain and interactive Hero abilities to the full match.
        test.setTimeout(game === 'three' ? 600_000 : 240_000)
        const host = new ObservedPlayer(page)
        const players = await startTable(browser, host, count, game)
        const commands: unknown[] = []
        try {
          if (count === 2 && outcome === 'lost') {
            await page.screenshot({ path: testInfo.outputPath(`game-${game}-mobile.png`), fullPage: true, animations: 'disabled', timeout: 30_000 })
            await page.setViewportSize({ width: 1440, height: 1000 })
            await page.screenshot({ path: testInfo.outputPath(`game-${game}-desktop.png`), fullPage: true, animations: 'disabled', timeout: 30_000 })
            await page.setViewportSize({ width: 393, height: 851 })
          }
          for (let command = 0; command < 600 && host.current().game.status === 'in_progress'; command += 1) {
            const state = host.current()
            const position = state.choice.status === 'pending' ? state.choice.responsible_position : state.turn.active_position
            const actor = players[position - 1]
            await actor.reaches(state.snapshot.sequence)
            const accepted = actor.page.waitForResponse((response) => response.url().endsWith('/api/games/current/commands') && response.request().method() === 'POST')
            await playThroughInterface(actor, outcome === 'won')
            const response = await accepted
            expect(response.status()).toBe(200)
            const request = response.request().postDataJSON() as Record<string, unknown>
            delete request.command_id
            commands.push(request)
            await host.reaches(state.snapshot.sequence + 1)
          }
          expect(host.current().game.status).toBe(outcome)
          for (const player of players) {
            await player.reaches(host.current().snapshot.sequence)
            await expect(player.page.getByRole('heading', { level: 2, name: outcome === 'won' ? 'Vitória da equipe' : 'Derrota da equipe' })).toBeVisible()
            expect(player.current().snapshot.digest).toBe(host.current().snapshot.digest)
            expect(player.eventBatches).toBeGreaterThan(0)
            expect(player.errors).toEqual([])
            await expect(player.page.getByRole('button', { name: 'Encerrar ações do Herói' })).toHaveCount(0)
            await expect(player.page.locator('.action-dock')).toContainText('Partida encerrada')
          }
          const transcriptPath = testInfo.outputPath(`game-${game}-commands.json`)
          await writeFile(transcriptPath, JSON.stringify({ players: count, seed_byte: 7, outcome, commands }, null, 2))
          await testInfo.attach(`game-${game}-commands`, { path: transcriptPath, contentType: 'application/json' })
        } finally {
          await Promise.all(players.slice(1).map((player) => player.page.context().close()))
        }
      })
    }
  }
}
