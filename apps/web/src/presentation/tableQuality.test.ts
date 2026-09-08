import { describe, expect, it } from 'vitest'
import { createTableQuality } from './tableQuality'

describe('table quality policy', () => {
  it('drops after two bad five-second windows, and rises only after thirty stable seconds', () => {
    const quality = createTableQuality()
    let now = 0
    function run(milliseconds: number, frameMs: number) {
      const until = now + milliseconds
      while (now < until) { now += frameMs; quality.frame(now, frameMs) }
    }
    run(5000, 40)
    expect(quality.current()).toBe('balanced')
    run(5000, 40)
    expect(quality.current()).toBe('basic')
    run(25000, 1000 / 30)
    expect(quality.current()).toBe('basic')
    run(5500, 1000 / 30)
    expect(quality.current()).toBe('balanced')
  })
  it('honors manual selection and excludes time spent in the background from hysteresis', () => {
    const quality = createTableQuality()
    quality.select('high', 0)
    for (let now = 40; now <= 15000; now += 40) quality.frame(now, 40)
    expect(quality.current()).toBe('high')
    quality.select('auto', 15000)
    quality.resetWindow(75000)
    for (let now = 75040; now <= 80000; now += 40) quality.frame(now, 40)
    expect(quality.current()).toBe('high')
    for (let now = 80040; now <= 85000; now += 40) quality.frame(now, 40)
    expect(quality.current()).toBe('balanced')
  })
})
