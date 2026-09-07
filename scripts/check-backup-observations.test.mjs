import assert from 'node:assert/strict'
import test from 'node:test'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, delimiter } from 'node:path'
import { fileURLToPath } from 'node:url'
import { backupObservations } from '../ops/observe-backups.mjs'

test('RDS observations expose lag and oldest backup age, including a violated window', () => {
  const samples = backupObservations({
    earliest: '2026-08-30T12:00:00Z', latest: '2026-09-07T11:54:00Z', retention: 8,
  }, 1, Date.parse('2026-09-07T12:00:00Z'))
  assert.deepEqual(samples, [
    ['backup_rpo_seconds', 360, 'success'],
    ['backup_window_seconds', 691_200, 'success'],
    ['backup_retention_days', 8, 'success'],
    ['backup_manual_snapshots', 1, 'success'],
    ['backup_measurements', 1, 'success'],
  ])
})

test('missing PITR evidence or a future clock cannot produce a healthy RPO', () => {
  for (const metadata of [null, {}, {
    earliest: '2026-09-07T12:00:00Z', latest: '2026-09-07T12:01:00Z', retention: 7,
  }]) {
    assert.deepEqual(backupObservations(metadata, 0, Date.parse('2026-09-07T12:00:00Z')),
      [['backup_measurements', 1, 'unavailable']])
  }
})

test('the operational probe calls the PITR API and never forwards private CLI errors', () => {
  const directory = mkdtempSync(join(tmpdir(), 'hogwarts-backup-probe-'))
  const script = fileURLToPath(new URL('../ops/observe-backups.mjs', import.meta.url))
  try {
    writeFileSync(join(directory, 'aws'), `#!/usr/bin/env node
if (process.env.PROBE_FAIL) {
  process.stderr.write('private-credential-canary'); process.exit(1)
}
const args = process.argv.slice(2)
if (args[0] !== 'rds' || !args.includes('--db-instance-identifier')) process.exit(2)
if (args[1] === 'describe-db-instance-automated-backups') {
  if (!args.some(value => value.includes('RestoreWindow.EarliestTime'))) process.exit(3)
  process.stdout.write(JSON.stringify({earliest:new Date(Date.now()-600000).toISOString(), latest:new Date(Date.now()-60000).toISOString(), retention:7}))
} else if (args[1] === 'describe-db-snapshots') {
  process.stdout.write('0')
} else { process.exit(4) }
`, { mode: 0o700 })
    const env = { ...process.env, PATH: `${directory}${delimiter}${process.env.PATH}`, TELEMETRY_ENVIRONMENT: 'staging', RDS_DB_INSTANCE_ID: 'private-database-canary' }
    const healthy = spawnSync(process.execPath, [script], { env, encoding: 'utf8' })
    assert.equal(healthy.status, 0)
    assert.equal(healthy.stderr, '')
    assert.ok(healthy.stdout.split('\n').filter(Boolean).map(JSON.parse).some(({ metric }) => metric === 'backup_window_seconds'))
    assert.ok(!healthy.stdout.includes('private-database-canary'))
    const failed = spawnSync(process.execPath, [script], { env: { ...env, PROBE_FAIL: '1' }, encoding: 'utf8' })
    assert.equal(failed.status, 1)
    assert.equal(failed.stderr, '')
    const observation = JSON.parse(failed.stdout)
    assert.equal(observation.metric, 'backup_measurements')
    assert.equal(observation.outcome, 'unavailable')
    assert.ok(!failed.stdout.includes('private-credential-canary'))
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
