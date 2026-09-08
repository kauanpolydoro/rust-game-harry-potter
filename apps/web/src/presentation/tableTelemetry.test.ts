import { expect, it } from 'vitest'
import { createDistribution } from './tableTelemetry'

it('retains rare slow frames in bounded distributions and starts a fresh benchmark after warmup', () => {
  const distribution = createDistribution()
  for (let index = 0; index < 10_000; index++) distribution.add(index % 100 === 0 ? 220 : 16.67)
  distribution.add(5000)
  distribution.add(Number.NaN)
  expect(distribution.read()).toMatchObject({ count: 10_001, medianMs: 16.75, p95Ms: 16.75, maxMs: 5000 })
  expect(distribution.read().bins).toHaveLength(3)
  distribution.reset()
  expect(distribution.read()).toMatchObject({ count: 0, medianMs: 0, maxMs: 0, bins: [] })
})
