import { afterEach, expect, it, vi } from 'vitest'

import { initializeTelemetry, observe, startJourney } from './telemetry'

let stop: (() => void) | undefined
afterEach(() => {
  stop?.()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('sends only anonymous numeric observations and completes each journey once', async () => {
  const requests = vi.fn().mockResolvedValue(new Response(null, { status: 204 }))
  vi.stubGlobal('fetch', requests)
  stop = initializeTelemetry()
  const finish = startJourney('recovery')
  finish('success')
  finish('abandoned')
  await Promise.resolve()
  const bodies = requests.mock.calls.map(([, options]) => JSON.parse(options.body))
  expect(bodies).toContainEqual({ metric: 'recovery_journeys', outcome: 'started', value: 1 })
  expect(bodies).toContainEqual({ metric: 'recovery_journeys', outcome: 'success', value: 1 })
  expect(bodies).toContainEqual({
    metric: 'recovery_human_seconds', outcome: 'success', value: expect.any(Number),
  })
  expect(bodies).toHaveLength(3)
  for (const [url, options] of requests.mock.calls) {
    expect(url).toBe('/api/telemetry')
    expect(options.credentials).toBe('omit')
    expect(options.referrerPolicy).toBe('no-referrer')
    expect(options.headers['x-csrf-protection']).toBe('1')
  }
})

it('records abandonment at page exit and bounds failed transport without retrying', async () => {
  const requests = vi.fn().mockRejectedValue(new Error('private transport error'))
  vi.stubGlobal('fetch', requests)
  stop = initializeTelemetry()
  startJourney('recovery')
  window.dispatchEvent(new Event('pagehide'))
  observe('web_lcp_seconds', Number.NaN)
  await Promise.resolve()
  expect(requests.mock.calls.map(([, options]) => JSON.parse(options.body))).toEqual([
    { metric: 'recovery_journeys', outcome: 'started', value: 1 },
    { metric: 'recovery_journeys', outcome: 'abandoned', value: 1 },
    { metric: 'recovery_human_seconds', outcome: 'abandoned', value: expect.any(Number) },
  ])
})
