export type TableQuality = 'basic' | 'balanced' | 'high'
export type QualityPreference = 'auto' | TableQuality
export const qualityProfiles = {
  basic: { label: 'Básico', fps: 30, frameBudget: 35, pixelRatio: 1, shadows: false, particles: 0 },
  balanced: { label: 'Equilibrado', fps: 60, frameBudget: 22, pixelRatio: 1.5, shadows: true, particles: 8 },
  high: { label: 'Alto', fps: 60, frameBudget: 22, pixelRatio: 2, shadows: true, particles: 16 },
} as const

/** Five-second observation windows, two bad windows down, thirty stable seconds up. */
export function createTableQuality() {
  const levels: TableQuality[] = ['basic', 'balanced', 'high']
  let level = 1
  let preference: QualityPreference = 'auto'
  let windowStart = 0
  let badWindows = 0
  let stableMilliseconds = 0
  const samples: number[] = []
  function resetWindow(now: number) {
    windowStart = now
    samples.length = 0
    badWindows = 0
    stableMilliseconds = 0
  }
  return {
    current: () => levels[level]!,
    resetWindow,
    select(value: QualityPreference, now: number) {
      preference = value
      if (value !== 'auto') level = levels.indexOf(value)
      resetWindow(now)
    },
    frame(now: number, interval: number) {
      if (preference !== 'auto' || !Number.isFinite(interval) || interval <= 0) return
      if (samples.length < 2048) samples.push(interval)
      if (now - windowStart < 5000) return
      const sorted = samples.sort((a, b) => a - b)
      const profile = qualityProfiles[levels[level]!]
      const bad = (sorted[Math.ceil(sorted.length * .95) - 1] ?? Infinity) > profile.frameBudget
        || (sorted[Math.floor(sorted.length / 2)] ?? Infinity) > 1000 / (profile.fps === 60 ? 58 : 29)
      badWindows = bad ? badWindows + 1 : 0
      stableMilliseconds = bad ? 0 : stableMilliseconds + now - windowStart
      if (badWindows >= 2 && level > 0) { level--; badWindows = 0; stableMilliseconds = 0 }
      if (stableMilliseconds >= 30000 && level < 2) { level++; stableMilliseconds = 0; badWindows = 0 }
      samples.length = 0
      windowStart = now
    },
  }
}
