import { expect, test, type Page } from '@playwright/test'
import { writeFile } from 'node:fs/promises'
import { ObservedPlayer, startTable } from './support/table'

test.use({ viewport: { width: 844, height: 390 }, video: 'on' })
// Functional flows run on a shared software GPU; physical latency has its own protocol.
test.setTimeout(60_000)

test('HTTP N+2 precedes WebSocket N+1 while both confirmed plays animate once', async ({ browser, page }) => {
  test.setTimeout(120_000)
  let hold = false
  const delayed: string[] = []
  let deliver: ((message: string) => void) | undefined
  await page.routeWebSocket('**/api/games/current/events?**', client => {
    const server = client.connectToServer()
    deliver = message => client.send(message)
    server.onMessage(message => {
      const serialized = String(message)
      if (hold && JSON.parse(serialized).type === 'events') delayed.push(serialized)
      else client.send(message)
    })
  })
  const host = new ObservedPlayer(page)
  const players = await startTable(browser, host, 2, 'one', 'visual')
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
    await expect(page.locator('.table-render-status')).toHaveCount(0)
    await page.evaluate(`(() => {
      const events = []
      Object.assign(window, { observedTableEffects: events })
      new MutationObserver(records => {
        for (const record of records) for (const node of record.addedNodes) {
          if (node instanceof HTMLElement && node.classList.contains('table-effect')) events.push(node.textContent ?? '')
        }
      }).observe(document.querySelector('.table-experience'), { childList: true, subtree: true })
    })()`)
    hold = true
    const before = host.current().snapshot.sequence
    for (let index = 0; index < 2; index++) {
      const state = host.current()
      const card = state.table.hand.find(item => state.legal_intentions.play_cards.some(intent => intent.card_id === item.instance_id && !intent.target_slots.length))!
      await page.getByRole('group', { name: 'Sua mão', exact: true }).getByRole('button', { name: `Inspecionar ${card.name}`, exact: true }).first().click()
      await page.getByRole('button', { name: `Jogar ${card.name}`, exact: true }).click()
      await host.reaches(before + index + 1)
    }
    await expect.poll(() => delayed.length).toBeGreaterThanOrEqual(2)
    expect(await page.evaluate('window.observedTableEffects')).not.toContain('Carta jogada')
    hold = false
    for (const message of delayed) deliver!(message)
    for (const message of delayed) deliver!(message)
    await expect.poll(async () => page.evaluate("window.observedTableEffects.filter(text => text === 'Carta jogada').length")).toBe(2)
    await expect(page.locator('.snapshot-details')).toContainText(`sequência ${before + 2}`)
    await expect(page.getByText('Atualizações em tempo real conectadas.')).toBeAttached()
    let recoveryWrites = 0
    page.on('request', request => { if (request.method() === 'POST') recoveryWrites++ })
    const cookiesBefore = await page.context().cookies()
    await page.evaluate(`(() => {
      const extension = document.querySelector('.table-canvas').getContext('webgl2').getExtension('WEBGL_lose_context')
      if (!extension) throw new Error('WEBGL_lose_context unavailable for this browser')
      Object.assign(window, { lostTableContext: extension })
      extension.loseContext()
    })()`)
    await expect(page.getByRole('button', { name: 'Tentar mesa 3D novamente' })).toBeVisible()
    await expect(page.locator('.table-effect')).toHaveCount(0)
    await page.evaluate('window.lostTableContext.restoreContext()')
    await expect(page.locator('.table-render-status')).toHaveCount(0)
    await expect(page.locator('.snapshot-details')).toContainText(`sequência ${before + 2}`)
    expect(recoveryWrites).toBe(0)
    const cookiesAfter = await page.context().cookies()
    expect(cookiesBefore.every(cookie => cookiesAfter.some(after => after.name === cookie.name && after.value === cookie.value))).toBe(true)
    expect(host.errors).toEqual([])
  } finally {
    await Promise.all(players.slice(1).map(player => player.page.context().close()))
  }
})

async function inspect(page: Page, zone: string, name: string) {
  await page.getByRole('group', { name: zone, exact: true }).getByRole('button', { name: `Inspecionar ${name}`, exact: true }).first().click()
  return page.getByRole('region', { name, exact: true })
}

test('prototype completes selection, target, damage, acquisition and a turn', async ({ page, browserName }, testInfo) => {
  const blockedResources: string[] = []
  let capturing = false
  page.on('console', message => {
    // Playwright synchronizes WebKit screenshots with an inline `body {}`
    // stylesheet. Keep the application's CSP intact and exclude that capture.
    if (capturing && browserName === 'webkit' && message.text().startsWith('Refused to apply a stylesheet')) return
    if (message.type() === 'error' && /Content Security Policy|Content-Security-Policy/i.test(message.text())) blockedResources.push(message.text())
  })
  async function capture(name: string) {
    capturing = true
    try { await page.screenshot({ path: testInfo.outputPath(name) }) }
    finally { capturing = false }
  }
  await page.goto('/prototype.html')
  const hand = page.getByRole('group', { name: 'Sua mão', exact: true })
  await expect(hand.getByRole('button')).toHaveCount(5)
  await capture('prototype-start.png')
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
  await capture('prototype-turn-complete.png')
  const resources = JSON.parse((await page.locator('.prototype-metrics').getAttribute('data-measurements'))!)
  expect(resources.compressedMaterials).toBe(2)
  expect(resources.loadedCardModel).toBe(true)
  expect(blockedResources).toEqual([])
  expect(resources.decodedImages).toBeLessThanOrEqual(16)
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
  await expect(page.getByRole('region', { name: 'Carta 20 - Encantamento de proteção' })).toContainText('Texto longo demonstrativo.')
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
  await expect(page.getByRole('region', { name: 'Carta 20 - Encantamento de proteção' })).toHaveCount(0)
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
  await expect(page.getByRole('region', { name: 'Carta 5 - Encantamento de proteção' })).toBeVisible()
  await page.getByRole('button', { name: 'Fechar inspeção' }).click()
  await page.getByRole('button', { name: 'Cartas jogadas anteriores' }).click()
  await played.getByRole('button').first().click()
  await expect(page.getByRole('region', { name: 'Carta 1 - Encantamento de proteção' })).toBeVisible()
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
    await Promise.all(players.slice(1).map(player => player.page.context().close()))
  }
})

for (const count of [2, 3, 4]) {
  test(`${count} participants finish a real Game 1 turn through the 3D table`, async ({ browser, page }, testInfo) => {
    test.setTimeout(180_000)
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
      await Promise.all(players.slice(1).map(player => player.page.context().close()))
    }
  })
}

test('basic reduced motion preserves separate damage, healing, health trail and gesture-gated audio', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' })
  const audioRequests: string[] = []
  page.on('request', request => { if (request.url().includes('/table-audio/')) audioRequests.push(request.url()) })
  await page.goto('/prototype.html')
  await expect(page.getByRole('group', { name: 'Sua mão', exact: true }).getByRole('button')).toHaveCount(5)
  expect(audioRequests).toEqual([])
  await page.getByText('Imagem e som', { exact: true }).click()
  await page.getByRole('combobox', { name: 'Qualidade da mesa' }).selectOption('basic')
  await page.getByText('Imagem e som', { exact: true }).click()
  await page.getByText('Ensaio e histórico', { exact: true }).click()
  await page.getByRole('button', { name: 'Ensaiar dano e cura' }).click()
  await expect(page.locator('.hero-feedback').getByText('−2 Vida', { exact: true })).toBeVisible()
  await expect(page.locator('.hero-feedback').getByText('+2 Vida', { exact: true })).toBeVisible()
  await expect(page.locator('.health-meter__damage')).toHaveCount(1)
  await expect(page.locator('.hero-vitals b').first()).toHaveText('Vida 10')
  expect(audioRequests).toEqual([])
  await page.getByText('Imagem e som', { exact: true }).click()
  await page.getByRole('button', { name: 'Ativar som', exact: true }).click()
  await expect.poll(() => audioRequests.length).toBe(7)
  await expect(page.getByRole('button', { name: 'Desativar som', exact: true })).toBeVisible()
  await page.getByText('Imagem e som', { exact: true }).click()
  await page.getByRole('button', { name: 'Medir renderização' }).click()
  const metrics = JSON.parse((await page.locator('.prototype-metrics').getAttribute('data-measurements'))!)
  expect(metrics.quality).toBe('basic')
  expect(metrics.audio.unlocked).toBe(true)
  expect(metrics.audio.soundsPlayed).toBe(0)
  await inspect(page, 'Sua mão', 'Varinha')
  await page.getByRole('button', { name: 'Jogar Varinha' }).click()
  await expect(page.getByRole('group', { name: 'Sua mão', exact: true }).getByRole('button')).toHaveCount(4)
})

test('failed texture worker uses image fallback and leaves cards selectable', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.route('**/ktx2.worker-*.js', route => route.abort())
  const fallback = page.waitForResponse(response => response.url().endsWith('/table-art/card-back.png') && response.ok())
  await page.goto('/prototype.html')
  await fallback
  await inspect(page, 'Sua mão', 'Varinha')
  await page.getByRole('button', { name: 'Jogar Varinha' }).click()
  await expect(page.getByRole('group', { name: 'Sua mão', exact: true }).getByRole('button')).toHaveCount(4)
  await page.getByRole('button', { name: 'Modo acessível', exact: true }).click()
  await expect(page.locator('.table-canvas')).toHaveCount(0)
  expect(errors).toEqual([])
})

test('visibility signal pauses GPU updates and audio until the newest state is visible', async ({ page }) => {
  await page.goto('/prototype.html')
  await expect(page.getByRole('group', { name: 'Sua mão', exact: true }).getByRole('button')).toHaveCount(5)
  await page.getByText('Imagem e som', { exact: true }).click()
  await page.getByRole('button', { name: 'Ativar som', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Desativar som', exact: true })).toBeVisible()
  await page.getByText('Imagem e som', { exact: true }).click()
  // Headless browser tabs remain visible, so deliver the browser's visibility
  // signal explicitly while retaining real WebGL, AudioContext and DOM updates.
  await page.evaluate(`Object.defineProperty(document, 'hidden', { configurable: true, value: true }); document.dispatchEvent(new Event('visibilitychange'))`)
  const metrics = async () => JSON.parse((await page.locator('.prototype-metrics').getAttribute('data-measurements'))!)
  const audioState = async () => {
    await page.getByRole('button', { name: 'Medir renderização' }).click()
    return (await metrics()).audio.state
  }
  await expect.poll(audioState).toBe('suspended')
  const paused = await metrics()
  await page.getByText('Ensaio e histórico', { exact: true }).click()
  await page.getByRole('button', { name: 'Mão extensa e texto longo' }).click()
  await page.getByText('Ensaio e histórico', { exact: true }).click()
  await page.getByRole('button', { name: 'Medir renderização' }).click()
  const hidden = await metrics()
  expect(hidden.frames).toBe(paused.frames)
  expect(hidden.textures).toBe(paused.textures)
  expect(hidden.meshes).toBe(paused.meshes)
  expect(hidden.audio.activeSounds).toBe(0)
  await page.evaluate(`delete document.hidden; document.dispatchEvent(new Event('visibilitychange'))`)
  await expect(page.getByRole('group', { name: 'Sua mão', exact: true }).getByRole('button')).toHaveCount(7)
  await expect.poll(audioState).toBe('running')
  expect((await metrics()).textures).toBeGreaterThan(paused.textures)
})

test('accessible mode rejects late texture work and releases the visual animation loop', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.addInitScript(`window.workerCreations = []; const NativeWorker = window.Worker;
    window.Worker = class extends NativeWorker { constructor(url, options) { window.workerCreations.push(String(url)); super(url, options) } }`)
  let release!: () => void
  const waiting = new Promise<void>(resolve => { release = resolve })
  let held = 0
  let finished = 0
  let cancelled = 0
  page.on('requestfinished', request => { if (request.url().endsWith('.ktx2')) finished++ })
  page.on('requestfailed', request => {
    if (request.url().endsWith('.ktx2') && /abort|cancel/i.test(request.failure()?.errorText ?? '')) cancelled++
  })
  await page.route('**/*.ktx2', async route => { held++; await waiting; await route.continue() })
  await page.goto('/prototype.html')
  await expect(page.getByRole('group', { name: 'Sua mão', exact: true }).getByRole('button')).toHaveCount(5)
  await expect.poll(() => held).toBe(2)
  await page.getByRole('button', { name: 'Modo acessível', exact: true }).click()
  await expect(page.getByRole('region', { name: 'Sua mão', exact: true })).toBeVisible()
  release()
  await expect.poll(() => finished + cancelled).toBe(2)
  expect(finished).toBeGreaterThan(0)
  const callbacks = await page.evaluate<number>(`(async () => {
    const original = window.requestAnimationFrame
    let count = 0
    window.requestAnimationFrame = callback => { count++; return original.call(window, callback) }
    await new Promise(resolve => setTimeout(resolve, 500))
    window.requestAnimationFrame = original
    return count
  })()`)
  expect(callbacks).toBe(0)
  expect(await page.evaluate('window.workerCreations')).toEqual([])
  expect(errors).toEqual([])
  await page.getByRole('button', { name: 'Voltar à mesa 3D', exact: true }).click()
  await expect(page.getByRole('group', { name: 'Sua mão', exact: true }).getByRole('button')).toHaveCount(5)
  await expect.poll(async () => {
    await page.getByRole('button', { name: 'Medir renderização' }).click()
    return JSON.parse((await page.locator('.prototype-metrics').getAttribute('data-measurements'))!).compressedMaterials
  }).toBe(2)
  expect(errors).toEqual([])
})
