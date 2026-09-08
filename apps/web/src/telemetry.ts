type Metric =
  | 'web_lcp_seconds' | 'web_inp_seconds' | 'web_cls_ratio'
  | 'recovery_journeys' | 'recovery_human_seconds' | 'reconnect_attempts'
  | 'replay_seconds' | 'snapshot_seconds' | 'long_task_seconds' | 'touch_feedback_seconds'
type Outcome = 'started' | 'success' | 'error' | 'abandoned'
type Journey = 'recovery' | 'reconnect'
type FinishJourney = (outcome: Exclude<Outcome, 'started'>, mode?: 'replay' | 'snapshot') => void

let enabled = false
let pending = 0
const journeys = new Set<FinishJourney>()

/** Deliberately accepts no labels, identifiers, DOM entries or error objects. */
export function observe(metric: Metric, value: number, outcome: Outcome = 'success'): void {
  if (!enabled || !Number.isFinite(value) || value < 0 || pending >= 8) return
  pending += 1
  void fetch('/api/telemetry', {
    method: 'POST', credentials: 'omit', referrerPolicy: 'no-referrer', keepalive: true,
    headers: { 'content-type': 'application/json', 'x-csrf-protection': '1' },
    body: JSON.stringify({ metric, value, outcome }),
  }).catch(() => {}).finally(() => { pending -= 1 })
}

export function startJourney(kind: Journey): FinishJourney {
  if (!enabled) return () => {}
  const started = performance.now()
  const counter = kind === 'recovery' ? 'recovery_journeys' : 'reconnect_attempts'
  observe(counter, 1, 'started')
  const finish: FinishJourney = (outcome, mode = 'replay') => {
    if (!journeys.delete(finish)) return
    observe(counter, 1, outcome)
    observe(kind === 'recovery' ? 'recovery_human_seconds' : `${mode}_seconds`,
      (performance.now() - started) / 1_000, outcome)
  }
  journeys.add(finish)
  return finish
}

export function initializeTelemetry(): () => void {
  if (enabled) return () => {}
  enabled = true
  const abandon = () => { for (const finish of journeys) finish('abandoned') }
  window.addEventListener('pagehide', abandon)
  const longTasks = observeLongTasks()
  const touch = (event: PointerEvent) => {
    if (event.pointerType !== 'touch') return
    const start = performance.now()
    requestAnimationFrame(() => requestAnimationFrame(() => {
      observe('touch_feedback_seconds', (performance.now() - start) / 1_000)
    }))
  }
  window.addEventListener('pointerdown', touch, { passive: true })
  // Standard build only: attribution entries and metric IDs never enter transport.
  void import('web-vitals').then(({ onLCP, onINP, onCLS }) => {
    if (!enabled) return
    onLCP(({ value }) => observe('web_lcp_seconds', value / 1_000))
    onINP(({ value }) => observe('web_inp_seconds', value / 1_000))
    onCLS(({ value }) => observe('web_cls_ratio', value))
  }).catch(() => {})
  return () => {
    abandon()
    enabled = false
    window.removeEventListener('pagehide', abandon)
    window.removeEventListener('pointerdown', touch)
    longTasks?.disconnect()
  }
}

function observeLongTasks(): PerformanceObserver | undefined {
  if (typeof PerformanceObserver === 'undefined' ||
    !PerformanceObserver.supportedEntryTypes?.includes('longtask')) return
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) observe('long_task_seconds', entry.duration / 1_000)
  })
  observer.observe({ type: 'longtask', buffered: true })
  return observer
}
