import { spawnSync } from 'node:child_process'

import { findUnapprovedDependencies } from './domain-boundaries.mjs'

const result = spawnSync('cargo', ['metadata', '--format-version=1', '--no-deps'], {
  encoding: 'utf8',
})

if (result.status !== 0) {
  process.stderr.write(result.stderr)
  process.exit(result.status ?? 1)
}

const metadata = JSON.parse(result.stdout)
const domain = metadata.packages.find((candidate) => candidate.name === 'game-domain')

if (!domain) {
  throw new Error('workspace must contain the game-domain crate')
}

const allowedDomainDependencies = []
const forbidden = findUnapprovedDependencies(domain.dependencies, allowedDomainDependencies)

if (forbidden.length > 0) {
  console.error(`game-domain imports unapproved dependencies: ${forbidden.join(', ')}`)
  process.exitCode = 1
}
