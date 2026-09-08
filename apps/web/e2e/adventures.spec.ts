import { expect, test } from '@playwright/test'
import { writeFile } from 'node:fs/promises'
import gameTwoPurchasePriority from '../../server/tests/fixtures/game-two/purchase-priority.json' with { type: 'json' }

import { ObservedPlayer, startTable } from './support/table'

test.use({ actionTimeout: 10_000 })

const purchasePriority = ['001', '005', '010', '008', '002', '007', '004', '006', '011', '009', '013', '003']

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
    for (let index = 0; index < choice.min; index += 1) await inputs.nth(index).check()
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
      for (let target = 0; target < slot.min; target += 1) await inputs.nth(target).check()
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
        const order = projection.snapshot.versions.content === 'game-two-en-v1'
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

for (const game of ['one', 'two'] as const) {
  for (const count of [2, 3, 4]) {
    for (const outcome of ['lost', 'won'] as const) {
      test(`Game ${game === 'one' ? 1 : 2} with ${count} players reaches ${outcome} through the browser and WebSocket`, async ({ browser, page }, testInfo) => {
        test.setTimeout(240_000)
        const host = new ObservedPlayer(page)
        const players = await startTable(browser, host, count, game)
        const commands: unknown[] = []
        try {
          if (count === 2 && outcome === 'lost') {
            await page.screenshot({ path: testInfo.outputPath(`game-${game}-mobile.png`), fullPage: true, scale: 'css' })
            await page.setViewportSize({ width: 1440, height: 1000 })
            await page.screenshot({ path: testInfo.outputPath(`game-${game}-desktop.png`), fullPage: true, scale: 'css' })
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
