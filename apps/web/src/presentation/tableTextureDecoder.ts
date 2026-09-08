import { KhronosTextureContainer2 } from '@babylonjs/core/Misc/khronosTextureContainer2'
import { AutoReleaseWorkerPool } from '@babylonjs/core/Misc/workerPool'
import '@babylonjs/core/Materials/Textures/Loaders/ktxTextureLoader'

export function acquireTableTextureDecoder(): () => void {
  const workers = new Set<Worker>()
  // The two compressed materials initialize independently. Posting init first is
  // ordered before decode by the Worker message queue. Keep the pool promise
  // fulfilled so a chunk failure reaches Babylon's texture error listener.
  const pool = new AutoReleaseWorkerPool(2, () => {
    const worker = new Worker(new URL('./ktx2.worker.ts', import.meta.url), { type: 'module' })
    workers.add(worker)
    worker.addEventListener('error', event => event.preventDefault())
    worker.postMessage({ action: 'init' })
    return Promise.resolve(worker)
  }, { idleTimeElapsedBeforeRelease: 1000 })
  KhronosTextureContainer2.WorkerPool = pool
  // Shipped UASTC textures stay compressed on ASTC/BC7 GPUs; other GPUs use the
  // small RGBA decoder instead of downloading an unneeded general transcoder.
  KhronosTextureContainer2.DefaultDecoderOptions.useRGBAIfASTCBC7NotAvailableWhenUASTC = true
  return () => {
    // Texture responses can arrive after scene disposal. Keep a closed entry
    // point until the next mount replaces it, preventing Babylon's blob worker
    // fallback and settling late decode promises without creating new workers.
    pool.push = () => { throw new Error('The table texture decoder was disposed') }
    pool.dispose()
    for (const worker of workers) worker.terminate()
    workers.clear()
  }
}
