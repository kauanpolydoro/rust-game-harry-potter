import assert from 'node:assert/strict'
import test from 'node:test'

import { findUnapprovedDependencies } from './domain-boundaries.mjs'

test('rejects every dependency not explicitly approved for the domain', () => {
  const dependencies = [{ name: 'game-rules' }, { name: 'internal-postgres-client' }]

  assert.deepEqual(findUnapprovedDependencies(dependencies, ['game-rules']), [
    'internal-postgres-client',
  ])
})

test('accepts a domain crate with no dependencies', () => {
  assert.deepEqual(findUnapprovedDependencies([], []), [])
})
