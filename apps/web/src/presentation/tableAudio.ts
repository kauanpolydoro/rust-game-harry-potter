import type { TableVisualGroup } from './tableTimeline'
import { visualManifest } from './visualManifest'

const sounds = ['card', 'damage', 'heal', 'resource', 'victory', 'defeat', 'ambient'] as const
type Sound = typeof sounds[number]
export interface TableAudioSettings { enabled: boolean; effects: number; ambient: number; vibration: boolean }
export function loadAudioSettings(): TableAudioSettings {
  const fallback = { enabled: false, effects: .5, ambient: .15, vibration: false }
  try {
    const saved = JSON.parse(localStorage.getItem('hogwarts.table-audio') ?? 'null')
    if (!saved) return fallback
    return { enabled: saved.enabled === true, vibration: saved.vibration === true,
      effects: typeof saved.effects === 'number' && saved.effects >= 0 && saved.effects <= 1 ? saved.effects : .5,
      ambient: typeof saved.ambient === 'number' && saved.ambient >= 0 && saved.ambient <= 1 ? saved.ambient : .15 }
  } catch { return fallback }
}

/** Owns audio resources independently of WebGL; never queues missed sound effects. */
export function createTableAudio(initial: TableAudioSettings) {
  let settings = { ...initial }
  let context: AudioContext | null = null
  let effectsGain: GainNode | null = null
  let ambientGain: GainNode | null = null
  let ambient: AudioBufferSourceNode | null = null
  let disposed = false
  let suspended = false
  let unlocked = false
  let soundsPlayed = 0
  const buffers = new Map<Sound, AudioBuffer>()
  const active = new Set<AudioBufferSourceNode>()
  const abort = new AbortController()

  function discardEffects() {
    for (const source of active) { source.stop(); source.disconnect() }
    active.clear()
  }
  function stop() {
    discardEffects()
    ambient?.stop()
    ambient?.disconnect()
    ambient = null
  }
  function startAmbient() {
    if (!context || ambient || !settings.enabled || !settings.ambient || suspended || disposed || context.state !== 'running') return
    const buffer = buffers.get('ambient')
    if (!buffer || !ambientGain) return
    ambient = context.createBufferSource()
    ambient.buffer = buffer
    ambient.loop = true
    ambient.connect(ambientGain)
    ambient.start()
  }
  async function unlock(): Promise<boolean> {
    if (disposed) return false
    try {
      if (!context) {
        context = new AudioContext()
        effectsGain = context.createGain()
        ambientGain = context.createGain()
        effectsGain.connect(context.destination)
        ambientGain.connect(context.destination)
        effectsGain.gain.value = settings.effects
        ambientGain.gain.value = settings.ambient
        const target = context
        for (const sound of sounds) {
          void fetch(visualManifest.audio[sound].url, { signal: abort.signal }).then(async response => {
            if (!response.ok) return
            const buffer = await target.decodeAudioData(await response.arrayBuffer())
            if (disposed) return
            buffers.set(sound, buffer)
            if (sound === 'ambient') startAmbient()
          }).catch(() => { /* Visual feedback remains complete if an audio asset fails. */ })
        }
      }
      await context.resume()
      if (disposed || suspended) { if (!disposed) await context.suspend(); return false }
      unlocked = context.state === 'running'
      startAmbient()
      return unlocked
    } catch { return false }
  }
  return {
    unlock,
    discardEffects,
    configure(next: TableAudioSettings) {
      settings = { ...next }
      if (effectsGain) effectsGain.gain.value = settings.effects
      if (ambientGain) ambientGain.gain.value = settings.ambient
      if (!settings.enabled) stop()
      else startAmbient()
      try { localStorage.setItem('hogwarts.table-audio', JSON.stringify(settings)) } catch { /* Retain the live settings. */ }
    },
    play(group: TableVisualGroup) {
      if (disposed || suspended || document.hidden || group.summarized) return
      if (settings.vibration && group.cues.some(cue => cue.kind === 'damage' || cue.kind === 'stun')) navigator.vibrate?.(20)
      if (!settings.enabled || !unlocked || context?.state !== 'running' || !effectsGain) return
      const groupSounds = new Set<Sound>()
      for (const cue of group.cues) {
        if (cue.kind === 'phase') continue
        groupSounds.add(cue.kind === 'damage' || cue.kind === 'stun' ? 'damage' : cue.kind === 'heal' ? 'heal'
          : cue.kind === 'won' || cue.kind === 'villain_defeated' ? 'victory'
            : cue.kind === 'lost' || cue.kind === 'location_lost' ? 'defeat' : cue.kind === 'resource' ? 'resource' : 'card')
      }
      for (const sound of groupSounds) {
        const buffer = buffers.get(sound)
        if (!buffer || active.size >= 8) continue
        const source = context.createBufferSource()
        source.buffer = buffer
        source.connect(effectsGain)
        source.onended = () => { active.delete(source); source.disconnect() }
        active.add(source)
        soundsPlayed++
        source.start()
      }
    },
    pause(value: boolean) {
      suspended = value
      stop()
      if (value) { navigator.vibrate?.(0); void context?.suspend().catch(() => {}) }
      else if (unlocked && settings.enabled) void unlock()
    },
    measurements: () => ({ unlocked, state: context?.state ?? 'locked', soundsPlayed,
      activeSounds: active.size, bufferedBytes: [...buffers.values()].reduce((size, buffer) => size + buffer.length * buffer.numberOfChannels * 4, 0) }),
    dispose() {
      disposed = true
      abort.abort()
      stop()
      buffers.clear()
      effectsGain?.disconnect()
      ambientGain?.disconnect()
      void context?.close().catch(() => {})
      context = null
    },
  }
}
