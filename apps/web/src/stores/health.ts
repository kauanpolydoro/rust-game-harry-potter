import { defineStore } from 'pinia'

import { healthStatuses, type HealthResponse } from '../contracts/health.generated'

export type Availability = 'checking' | 'ready' | 'unavailable'

function isHealthResponse(value: unknown): value is HealthResponse {
  if (typeof value !== 'object' || value === null || !("status" in value)) {
    return false
  }

  return healthStatuses.some((status) => status === value.status)
}

export const useHealthStore = defineStore('health', {
  state: (): { availability: Availability } => ({
    availability: 'checking',
  }),
  actions: {
    async check(): Promise<void> {
      this.availability = 'checking'

      try {
        const response = await fetch('/health/ready', {
          cache: 'no-store',
          credentials: 'same-origin',
          headers: { Accept: 'application/json' },
        })
        const health: unknown = await response.json()
        this.availability =
          response.ok && isHealthResponse(health) && health.status === 'ready'
            ? 'ready'
            : 'unavailable'
      } catch {
        this.availability = 'unavailable'
      }
    },
  },
})
