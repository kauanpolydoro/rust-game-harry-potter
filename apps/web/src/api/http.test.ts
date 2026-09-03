import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  ApiTransportError,
  apiError,
  isUncertainTransportFailure,
  requestJson,
} from './http'

describe('HTTP boundary', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('returns the status and validated JSON body', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ status: 'ready' }), {
          headers: { 'Content-Type': 'application/json' },
          status: 200,
        }),
      ),
    )

    const result = await requestJson('/health/ready')

    expect(result.response.status).toBe(200)
    expect(result.body).toEqual({ status: 'ready' })
  })

  it('aborts a request at the shared timeout boundary', async () => {
    vi.useFakeTimers()
    vi.stubGlobal(
      'fetch',
      vi.fn((_input: RequestInfo | URL, init?: RequestInit) =>
        new Promise((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => reject(init.signal?.reason), { once: true })
        }),
      ),
    )

    const request = expect(requestJson('/slow', {}, { timeoutMs: 50 })).rejects.toMatchObject({
      code: 'REQUEST_TIMEOUT',
    })
    await vi.advanceTimersByTimeAsync(50)

    await request
  })

  it('keeps caller cancellation distinct from a network failure', async () => {
    const controller = new AbortController()
    vi.stubGlobal(
      'fetch',
      vi.fn((_input: RequestInfo | URL, init?: RequestInit) =>
        new Promise((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => reject(init.signal?.reason), { once: true })
        }),
      ),
    )

    const request = requestJson('/cancelled', { signal: controller.signal })
    controller.abort()

    await expect(request).rejects.toMatchObject({ code: 'REQUEST_CANCELLED' })
  })

  it('rejects malformed JSON and incomplete API errors', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response('<html>gateway failure</html>', {
          headers: { 'Content-Type': 'text/html' },
          status: 502,
        }),
      ),
    )

    await expect(requestJson('/malformed')).rejects.toMatchObject({
      code: 'UNEXPECTED_RESPONSE',
    } satisfies Partial<ApiTransportError>)
    expect(apiError({ error: { code: 'PARTIAL' } })).toBeNull()
  })

  it('classifies every transport outcome with an uncertain commit result', () => {
    expect(isUncertainTransportFailure('NETWORK_UNAVAILABLE')).toBe(true)
    expect(isUncertainTransportFailure('REQUEST_TIMEOUT')).toBe(true)
    expect(isUncertainTransportFailure('REQUEST_CANCELLED')).toBe(true)
    expect(isUncertainTransportFailure('UNEXPECTED_RESPONSE')).toBe(true)
    expect(isUncertainTransportFailure('ROOM_FULL')).toBe(false)
    expect(isUncertainTransportFailure(null)).toBe(false)
  })
})
