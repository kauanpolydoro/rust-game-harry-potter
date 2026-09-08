/** Bounded distributions retain an hour-long run without retaining every frame. */
export function createDistribution() {
  const bins = new Uint32Array(4002)
  let count = 0, sum = 0, max = 0
  return {
    reset() { bins.fill(0); count = 0; sum = 0; max = 0 },
    add(value: number) {
      if (!Number.isFinite(value) || value < 0) return
      bins[Math.min(4001, Math.floor(value * 4))]!++
      count++; sum += value; max = Math.max(max, value)
    },
    read() {
      function percentile(fraction: number) {
        let seen = 0
        for (let index = 0; index < bins.length; index++) {
          seen += bins[index]!
          if (count && seen >= Math.ceil(count * fraction)) return index === 4001 ? max : (index + 1) / 4
        }
        return 0
      }
      return { count, meanMs: count ? sum / count : 0, medianMs: percentile(.5), p95Ms: percentile(.95), maxMs: max,
        binWidthMs: .25, overflowFromMs: 1000.25,
        bins: [...bins].flatMap((count, index) => count ? [[index, count]] : []) }
    },
  }
}

export function createTableTelemetry() {
  let startedAt = performance.now()
  const frames = createDistribution()
  let sceneSegments = 0
  const input = createDistribution()
  const command = createDistribution()
  const longTasks = createDistribution()
  let observer: PerformanceObserver | undefined
  try {
    if (PerformanceObserver.supportedEntryTypes.includes('longtask')) {
      observer = new PerformanceObserver(list => {
        for (const entry of list.getEntries()) if (!document.hidden && entry.startTime >= startedAt) longTasks.add(entry.duration)
      })
      observer.observe({ type: 'longtask' })
    }
  } catch { /* Unsupported metrics are exported as unavailable, never zero. */ }
  return {
    input, command, frames,
    sceneStarted() { sceneSegments++ },
    reset() { startedAt = performance.now(); input.reset(); command.reset(); longTasks.reset(); frames.reset(); sceneSegments = 1 },
    read: () => ({ elapsedMs: performance.now() - startedAt, sceneSegments, inputToAnimationFrameCallback: input.read(), inputToCommandHandler: command.read(),
      longTasks: observer ? longTasks.read() : null,
      heapBytes: (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize ?? null }),
    dispose: () => observer?.disconnect(),
  }
}
