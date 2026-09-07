import { expect, test, type Page } from '@playwright/test'
import { writeFile } from 'node:fs/promises'
import { ObservedPlayer, startTable } from './support/table'

test.use({ viewport: { width: 844, height: 390 }, video: 'on' })

async function inspect(page: Page, zone: string, name: string) {
  await page.getByRole('group', { name: zone, exact: true }).getByRole('button', { name: `Inspecionar ${name}`, exact: true }).first().click()
  return page.getByRole('region', { name, exact: true })
}

test('prototype completes selection, target, damage, acquisition and a turn', async ({ page }, testInfo) => {
  await page.goto('/prototype.html')
  const hand = page.getByRole('group', { name: 'Sua mão', exact: true })
  await expect(hand.getByRole('button')).toHaveCount(5)
  await page.screenshot({ path: testInfo.outputPath('prototype-start.png') })
  let inspection = await inspect(page, 'Sua mão', 'Varinha')
  await expect(hand.getByRole('button')).toHaveCount(5)
  await inspection.getByRole('button', { name: 'Jogar Varinha' }).click()
  await expect(hand.getByRole('button')).toHaveCount(4)
  await page.getByRole('button', { name: 'Inspecionar Draco Malfoy, Vida 6' }).click()
  await page.getByRole('button', { name: 'Menos Ataque' }).click()
  await page.getByRole('button', { name: 'Atacar Draco Malfoy com 2' }).click()
  await expect(page.getByRole('button', { name: 'Inspecionar Draco Malfoy, Vida 4' })).toBeVisible()
  inspection = await inspect(page, 'Sua mão', 'Edwiges')
  await expect(inspection.getByRole('button', { name: 'Jogar Edwiges' })).toBeDisabled()
  await inspection.getByRole('radio', { name: 'Hermione', exact: true }).check()
  await inspection.getByRole('button', { name: 'Jogar Edwiges' }).click()
  inspection = await inspect(page, 'Sua mão', 'História de Hogwarts')
  await inspection.getByRole('button', { name: 'Jogar História de Hogwarts' }).click()
  await page.getByRole('button', { name: 'Inspecionar Essência de Ditamno, custo 2' }).click()
  await page.getByRole('combobox', { name: 'Destino' }).selectOption('draw_pile')
  await page.getByRole('button', { name: 'Adquirir Essência de Ditamno por 2 de Influência' }).click()
  await expect(page.getByRole('button', { name: 'Inspecionar Wingardium Leviosa, custo 2' })).toBeVisible()
  await page.getByRole('button', { name: 'Encerrar ações do Herói' }).click()
  await expect(page.getByText('Turno 2', { exact: true })).toBeVisible()
  await expect(hand.getByRole('button')).toHaveCount(5)
  await page.getByRole('button', { name: 'Medir renderização' }).click()
  await page.screenshot({ path: testInfo.outputPath('prototype-turn-complete.png') })
  const measurements = testInfo.outputPath('initial-measurements.txt')
  await writeFile(measurements, await page.locator('.prototype-metrics').innerText())
  await testInfo.attach('initial-measurements', { path: measurements, contentType: 'text/plain' })
})

test('long hands remain inspectable and rotation cancels a local gesture', async ({ page }, testInfo) => {
  await page.goto('/prototype.html')
  const hand = page.getByRole('group', { name: 'Sua mão', exact: true })
  await expect(hand.getByRole('button')).toHaveCount(5)
  await page.getByText('Ensaio e histórico', { exact: true }).click()
  await page.getByRole('button', { name: 'Mão extensa e texto longo' }).click()
  await page.getByText('Ensaio e histórico', { exact: true }).click()
  await expect(hand.getByRole('button')).toHaveCount(7)
  await page.getByRole('button', { name: 'Próximas cartas' }).click()
  await page.getByRole('button', { name: 'Próximas cartas' }).click()
  await expect(hand.getByRole('button')).toHaveCount(6)
  const card = hand.getByRole('button').last()
  await card.click()
  await expect(page.getByRole('region', { name: 'Carta 20 — Encantamento de proteção' })).toContainText('Texto longo demonstrativo.')
  await page.screenshot({ path: testInfo.outputPath('long-hand-inspection.png') })
  await page.getByRole('button', { name: 'Fechar inspeção' }).click()
  const bounds = (await card.boundingBox())!
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
  await page.mouse.down()
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y - 30, { steps: 4 })
  await page.setViewportSize({ width: 390, height: 844 })
  await expect(page.getByText('Gire o celular para abrir a mesa', { exact: true })).toBeVisible()
  await page.mouse.up()
  await page.getByRole('button', { name: 'Modo acessível', exact: true }).click()
  await expect(page.getByRole('region', { name: 'Sua mão', exact: true }).getByRole('listitem')).toHaveCount(20)
  await page.setViewportSize({ width: 844, height: 390 })
  await page.getByRole('button', { name: 'Voltar à mesa 3D', exact: true }).click()
  await expect(page.getByRole('region', { name: 'Carta 20 — Encantamento de proteção' })).toHaveCount(0)
  await expect(hand.getByRole('button')).toHaveCount(6)
})

test('keyboard, invalid drops and pointer cancellation preserve explicit decisions', async ({ page }) => {
  await page.goto('/prototype.html')
  const hand = page.getByRole('group', { name: 'Sua mão', exact: true })
  const card = hand.getByRole('button', { name: 'Inspecionar Varinha', exact: true })
  await expect(card).toBeVisible()
  const bounds = (await card.boundingBox())!
  const start = { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 }
  for (const cancelled of [false, true]) {
    await page.mouse.move(start.x, start.y)
    await page.mouse.down()
    await page.mouse.move(20, start.y - 30, { steps: 4 })
    if (cancelled) await card.dispatchEvent('pointercancel', { pointerId: 1, isPrimary: true })
    await page.mouse.up()
    await expect(hand.getByRole('button')).toHaveCount(5)
    await expect(page.locator('.card-inspection')).toHaveCount(0)
  }
  await card.focus()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('region', { name: 'Varinha', exact: true })).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(card).toBeFocused()
  await page.keyboard.press('Enter')
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Fechar inspeção' })).toBeFocused()
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Jogar Varinha' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(hand.getByRole('button')).toHaveCount(4)
})

test('seven played cards remain individually reachable by touch', async ({ page }) => {
  await page.goto('/prototype.html')
  await page.getByText('Ensaio e histórico', { exact: true }).click()
  await page.getByRole('button', { name: 'Mão extensa e texto longo' }).click()
  await page.getByText('Ensaio e histórico', { exact: true }).click()
  for (let index = 0; index < 7; index++) {
    await page.getByRole('group', { name: 'Sua mão', exact: true }).getByRole('button').first().click()
    await page.getByRole('button', { name: /^Jogar Carta / }).click()
  }
  const played = page.getByRole('group', { name: 'Área de jogo', exact: true })
  await expect(played.getByRole('button')).toHaveCount(4)
  await page.getByRole('button', { name: 'Próximas cartas jogadas' }).click()
  await expect(played.getByRole('button')).toHaveCount(3)
  await played.getByRole('button').first().click()
  await expect(page.getByRole('region', { name: 'Carta 5 — Encantamento de proteção' })).toBeVisible()
  await page.getByRole('button', { name: 'Fechar inspeção' }).click()
  await page.getByRole('button', { name: 'Cartas jogadas anteriores' }).click()
  await played.getByRole('button').first().click()
  await expect(page.getByRole('region', { name: 'Carta 1 — Encantamento de proteção' })).toBeVisible()
})

test('real pending, remote, stale and rejected commands require a fresh decision', async ({ browser, page }) => {
  test.setTimeout(120_000)
  const host = new ObservedPlayer(page)
  const players = await startTable(browser, host, 2, 'one', 'visual')
  const commandPath = '**/api/games/current/commands'
  let posts = 0
  page.on('request', (request) => {
    if (request.method() === 'POST' && request.url().endsWith('/api/games/current/commands')) posts++
  })
  const playable = () => {
    const cards = host.current().table.hand.filter((card) => host.current().legal_intentions.play_cards.some((item) => item.card_id === card.instance_id && !item.target_slots.length))
    return cards.find((card) => card.name === 'Alohomora') ?? cards[0]!
  }
  try {
    for (let index = 0; index < 8 && host.current().choice.status === 'pending'; index++) {
      const choice = host.current().choice
      if (choice.status !== 'pending') break
      const actor = players[choice.responsible_position - 1]!
      const sequence = host.current().snapshot.sequence
      await actor.reaches(sequence)
      for (let option = 0; option < choice.min; option++) await actor.page.locator('.effect-choice input').nth(option).check()
      await actor.page.getByRole('button', { name: 'Confirmar escolha' }).click()
      await host.reaches(sequence + 1)
    }
    posts = 0
    const card = playable()
    const count = host.current().table.hand.length
    const sequence = host.current().snapshot.sequence
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    await page.route(commandPath, async (route) => { await gate; await route.continue() })
    const inspection = await inspect(page, 'Sua mão', card.name)
    expect(posts).toBe(0)
    const confirm = inspection.getByRole('button', { name: `Jogar ${card.name}`, exact: true })
    await confirm.click()
    await expect(confirm).toBeDisabled()
    await page.keyboard.press('Enter')
    await expect.poll(() => posts).toBe(1)
    expect(host.current().snapshot.sequence).toBe(sequence)
    await expect(page.getByRole('group', { name: 'Sua mão', exact: true }).getByRole('button')).toHaveCount(count)
    release()
    await host.reaches(sequence + 1)
    await page.unroute(commandPath)

    const otherPage = await page.context().newPage()
    try {
      const other = new ObservedPlayer(otherPage)
      await otherPage.goto('/')
      await other.reaches(sequence + 1)
      const next = playable()
      await inspect(page, 'Sua mão', next.name)
      const remoteInspection = await inspect(otherPage, 'Sua mão', next.name)
      await remoteInspection.getByRole('button', { name: `Jogar ${next.name}`, exact: true }).click()
      await host.reaches(sequence + 2)
      await expect(page.locator('.card-inspection')).toHaveCount(0)
      expect(posts).toBe(1)
    } finally { await otherPage.close() }

    const currentSequence = host.current().snapshot.sequence
    await page.setViewportSize({ width: 390, height: 844 })
    await expect(page.getByText('Gire o celular para abrir a mesa', { exact: true })).toBeVisible()
    await page.getByRole('button', { name: 'Modo acessível', exact: true }).click()
    await expect(page.getByRole('region', { name: 'Sua mão', exact: true })).toBeVisible()
    await page.context().setOffline(true)
    await page.context().setOffline(false)
    await page.reload()
    await host.reaches(currentSequence)
    await page.setViewportSize({ width: 844, height: 390 })
    await page.getByRole('button', { name: 'Voltar à mesa 3D', exact: true }).click()
    expect(posts).toBe(1)

    for (const failure of ['stale', 'rejected'] as const) {
      await page.route(commandPath, (route) => {
        const request = route.request().postDataJSON() as Record<string, unknown>
        const override = failure === 'stale'
          ? { expected_state_version: host.current().snapshot.state_version - 1 }
          : { card_id: '00000000-0000-4000-8000-000000000001' }
        return route.continue({ postData: JSON.stringify({ ...request, ...override }) })
      })
      const next = playable()
      const detail = await inspect(page, 'Sua mão', next.name)
      await detail.getByRole('button', { name: `Jogar ${next.name}`, exact: true }).click()
      await expect(page.locator('.card-inspection')).toHaveCount(0)
      await expect(page.locator('.game-information')).toContainText(failure === 'stale' ? 'Estado oficial atualizado' : 'Ação não aceita')
      expect(host.current().snapshot.sequence).toBe(currentSequence)
      await page.unroute(commandPath)
      if (await page.locator('.game-information').getAttribute('open') !== null) {
        await page.getByText('Partida e conexão', { exact: true }).click()
      }
    }
    expect(posts).toBe(3)
    const next = playable()
    const detail = await inspect(page, 'Sua mão', next.name)
    await detail.getByRole('button', { name: `Jogar ${next.name}`, exact: true }).click()
    await host.reaches(currentSequence + 1)
    expect(posts).toBe(4)
    expect(host.errors).toEqual([])
  } finally {
    await page.context().setOffline(false)
    for (const player of players.slice(1)) await player.page.context().close()
  }
})

for (const count of [2, 3, 4]) {
  test(`${count} participants finish a real Game 1 turn through the 3D table`, async ({ browser, page }, testInfo) => {
    test.setTimeout(count === 2 ? 180_000 : 120_000)
    const host = new ObservedPlayer(page)
    const players = await startTable(browser, host, count, 'one', 'visual')
    try {
      for (const player of players) await player.page.setViewportSize({ width: 844, height: 390 })
      const firstTurn = host.current().turn.number
      for (let action = 0; action < 30 && host.current().turn.number === firstTurn; action++) {
        const state = host.current()
        const actor = players[(state.choice.status === 'pending' ? state.choice.responsible_position : state.turn.active_position) - 1]!
        await actor.reaches(state.snapshot.sequence)
        const choice = actor.current().choice
        const nextSequence = state.snapshot.sequence + 1
        if (choice.status === 'pending') {
          const region = actor.page.locator('.effect-choice')
          for (let index = 0; index < choice.min; index++) await region.locator('input').nth(index).check()
          await region.getByRole('button', { name: 'Confirmar escolha' }).click()
        } else if (actor.current().legal_intentions.play_cards.length) {
          const play = actor.current().legal_intentions.play_cards[0]!
          const card = actor.current().table.hand.find((item) => item.instance_id === play.card_id)!
          const before = actor.current().snapshot.state_version
          const inspection = await inspect(actor.page, 'Sua mão', card.name)
          expect(actor.current().snapshot.state_version).toBe(before)
          for (const [index, slot] of play.target_slots.entries()) {
            const fieldset = inspection.getByRole('group').nth(index)
            for (let target = 0; target < slot.min; target++) await fieldset.locator('input').nth(target).check()
          }
          await inspection.getByRole('button', { name: `Jogar ${card.name}`, exact: true }).click()
        } else if (actor.current().legal_intentions.assign_attack.length) {
          const attack = actor.current().legal_intentions.assign_attack[0]!
          const villain = actor.current().table.active_villains.find((item) => item.instance_id === attack.villain_id)!
          await actor.page.getByRole('button', { name: `Inspecionar ${villain.name}, Vida ${villain.health}`, exact: true }).click()
          await actor.page.getByRole('button', { name: `Atacar ${villain.name} com ${attack.max_amount}`, exact: true }).click()
        } else if (actor.current().legal_intentions.acquire_cards.length) {
          const acquisition = actor.current().legal_intentions.acquire_cards[0]!
          const card = actor.current().table.market.find((item) => item.instance_id === acquisition.card_id)!
          await actor.page.getByRole('button', { name: `Inspecionar ${card.name}, custo ${card.cost}`, exact: true }).first().click()
          await actor.page.getByRole('button', { name: `Adquirir ${card.name} por ${card.cost} de Influência`, exact: true }).click()
        } else {
          await actor.page.getByRole('button', { name: 'Encerrar ações do Herói', exact: true }).click()
        }
        await host.reaches(nextSequence)
      }
      expect(host.current().turn.number).toBe(firstTurn + 1)
      for (const player of players) {
        await player.reaches(host.current().snapshot.sequence)
        expect(player.errors).toEqual([])
        expect(player.eventBatches).toBeGreaterThan(0)
        await expect(player.page.locator('.table-canvas')).toBeVisible()
      }
      await page.screenshot({ path: testInfo.outputPath(`real-turn-${count}-players.png`) })
      if (count === 2) {
        for (const [width, height] of [[667, 375], [844, 390], [915, 412], [1024, 768], [1366, 768]]) {
          await page.setViewportSize({ width: width!, height: height! })
          const canvas = page.locator('.table-canvas')
          await expect(canvas).toBeVisible()
          await expect.poll(async () => (await canvas.boundingBox())?.width).toBeGreaterThan(width! * 0.95)
          const targets = await page.locator('.card-hit-target').evaluateAll((elements) => elements.map((element) => {
            const box = element.getBoundingClientRect()
            return { name: element.getAttribute('aria-label'), x: box.x, y: box.y, width: box.width, height: box.height,
              reachable: element.contains(element.ownerDocument.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2)) }
          }))
          for (const bounds of targets) {
            expect(bounds.width).toBeGreaterThanOrEqual(44)
            expect(bounds.height).toBeGreaterThanOrEqual(44)
            expect(bounds.x).toBeGreaterThanOrEqual(0)
            expect(bounds.y + bounds.height).toBeLessThanOrEqual(height!)
            expect(bounds.reachable, bounds.name ?? '').toBe(true)
          }
          await page.screenshot({ path: testInfo.outputPath(`table-${width}x${height}.png`) })
        }
      }
    } finally {
      for (const player of players.slice(1)) await player.page.context().close()
    }
  })
}
