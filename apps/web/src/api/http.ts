import {
  isErrorResponse,
  type ErrorResponse,
} from '../contracts/identity-access.generated'

export type ApiTransportErrorCode =
  | 'NETWORK_UNAVAILABLE'
  | 'REQUEST_CANCELLED'
  | 'REQUEST_TIMEOUT'
  | 'UNEXPECTED_RESPONSE'

export class ApiTransportError extends Error {
  readonly code: ApiTransportErrorCode

  constructor(code: ApiTransportErrorCode, cause?: unknown) {
    super(code, { cause })
    this.name = 'ApiTransportError'
    this.code = code
  }
}

interface RequestJsonOptions {
  timeoutMs?: number
}

export interface JsonHttpResponse {
  body: unknown
  response: Response
}

const defaultTimeoutMs = 10_000

export async function requestJson(
  input: RequestInfo | URL,
  init: RequestInit = {},
  options: RequestJsonOptions = {},
): Promise<JsonHttpResponse> {
  const externalSignal = init.signal
  const controller = new AbortController()
  let timedOut = false
  const timeout = window.setTimeout(() => {
    timedOut = true
    controller.abort(new DOMException('Request timed out', 'TimeoutError'))
  }, options.timeoutMs ?? defaultTimeoutMs)
  const cancelFromCaller = () => controller.abort(externalSignal?.reason)
  externalSignal?.addEventListener('abort', cancelFromCaller, { once: true })
  if (externalSignal?.aborted) {
    cancelFromCaller()
  }

  try {
    const response = await fetch(input, {
      cache: 'no-store',
      credentials: 'same-origin',
      ...init,
      signal: controller.signal,
    })
    let body: unknown
    try {
      body = await response.json()
    } catch (error) {
      throw new ApiTransportError('UNEXPECTED_RESPONSE', error)
    }
    return { body, response }
  } catch (error) {
    if (error instanceof ApiTransportError) {
      throw error
    }
    if (timedOut) {
      throw new ApiTransportError('REQUEST_TIMEOUT', error)
    }
    if (externalSignal?.aborted) {
      throw new ApiTransportError('REQUEST_CANCELLED', error)
    }
    throw new ApiTransportError('NETWORK_UNAVAILABLE', error)
  } finally {
    window.clearTimeout(timeout)
    externalSignal?.removeEventListener('abort', cancelFromCaller)
  }
}

export function apiError(value: unknown): ErrorResponse['error'] | null {
  return isErrorResponse(value) ? value.error : null
}

export function transportErrorCode(error: unknown): ApiTransportErrorCode {
  return error instanceof ApiTransportError ? error.code : 'NETWORK_UNAVAILABLE'
}

export function isUncertainTransportFailure(code: string | null): boolean {
  return (
    code === 'NETWORK_UNAVAILABLE' ||
    code === 'REQUEST_TIMEOUT' ||
    code === 'REQUEST_CANCELLED' ||
    code === 'UNEXPECTED_RESPONSE'
  )
}
