import assert from 'node:assert/strict'
import test from 'node:test'
import { execFile } from 'node:child_process'
import { mkdtemp, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, delimiter } from 'node:path'
import { promisify } from 'node:util'
import { auditBackupInventory } from './audit-backup-policy.mjs'
const now = Date.parse('2026-09-07T15:00:00Z')
function valid() {
  return { observed_at: '2026-09-07T15:00:00Z', DBInstances: [{ BackupRetentionPeriod: 7, DBInstanceStatus: 'available',
    EarliestRestorableTime: '2026-08-31T15:00:00Z', LatestRestorableTime: '2026-09-07T14:55:00Z' }],
    DBSnapshots: [], DBInstanceAutomatedBackups: [], RecoveryPoints: [] }
}
test('accepts the exact seven-day window and five-minute RPO boundaries', () => {
  assert.deepEqual(auditBackupInventory(valid(), now), [])
})
test('fails closed on omitted inventories, stale observations and unavailable restore windows', () => {
  for (const field of ['observed_at', 'DBInstances', 'DBSnapshots', 'DBInstanceAutomatedBackups', 'RecoveryPoints']) {
    const inventory = valid(); delete inventory[field]
    assert.ok(auditBackupInventory(inventory, now).length > 0)
  }
  assert.ok(auditBackupInventory(valid(), now + 60_001).length > 0)
  const inventory = valid(); delete inventory.DBInstances[0].EarliestRestorableTime
  assert.ok(auditBackupInventory(inventory, now).length > 0)
})
test('rejects stale PITR, disabled or long retention, and stopped instances', () => {
  for (const change of [{ BackupRetentionPeriod: 0 }, { BackupRetentionPeriod: 8 }, { DBInstanceStatus: 'stopped' },
    { EarliestRestorableTime: '2026-08-31T14:59:59.999Z' }, { LatestRestorableTime: '2026-09-07T14:54:59.999Z' }]) {
    const inventory = valid(); Object.assign(inventory.DBInstances[0], change)
    assert.ok(auditBackupInventory(inventory, now).length > 0)
  }
})
test('includes retained automated backups and AWS Backup outside the instance settings', () => {
  const inventory = valid()
  inventory.DBInstanceAutomatedBackups.push({ BackupRetentionPeriod: 35 })
  assert.ok(auditBackupInventory(inventory, now).length > 0)
  inventory.DBInstanceAutomatedBackups = []
  inventory.RecoveryPoints.push({ Status: 'COMPLETED', CreationDate: '2026-09-07T00:00:00Z', Lifecycle: { DeleteAfterDays: 35 } })
  assert.ok(auditBackupInventory(inventory, now).length > 0)
})
test('a tag alone never certifies actual deletion of a manual snapshot', () => {
  const inventory = valid()
  inventory.DBSnapshots.push({ SnapshotType: 'manual', SnapshotCreateTime: '2026-09-07T00:00:00Z',
    TagList: [{ Key: 'delete-after', Value: '2026-09-08T00:00:00Z' }] })
  assert.ok(auditBackupInventory(inventory, now).length > 0)
})
test('automated snapshots remain independently restorable even when the PITR window is compliant', () => {
  const inventory = valid()
  inventory.DBSnapshots.push({ SnapshotType: 'automated', SnapshotCreateTime: '2026-08-31T15:00:00Z' })
  assert.deepEqual(auditBackupInventory(inventory, now), [])
  inventory.DBSnapshots[0].SnapshotCreateTime = '2026-08-30T15:00:00Z'
  assert.ok(auditBackupInventory(inventory, now).includes('snapshot copy contains data older than the allowed restore window'))
})
test('live collection checks original data age in snapshots managed by AWS Backup', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'backup-audit-'))
  try {
    // Model the native CLI boundary: its default snapshot query omits awsbackup.
    await writeFile(join(directory, 'aws'), `#!/usr/bin/env node
const args = process.argv.slice(2)
const now = Date.now()
const ago = (days) => new Date(now - days * 86400000).toISOString()
const snapshot = { SnapshotType: 'awsbackup', DBSnapshotArn: 'fixture-snapshot', SnapshotCreateTime: ago(1), SnapshotDatabaseTime: ago(8) }
const responses = {
  'describe-db-instances': { DBInstances: [{ BackupRetentionPeriod: 7, DBInstanceStatus: 'available', EarliestRestorableTime: ago(6), LatestRestorableTime: ago(0), DbiResourceId: 'fixture-db', DBInstanceArn: 'fixture-source' }] },
  'describe-db-snapshots': { DBSnapshots: args.includes('awsbackup') ? [snapshot] : [] },
  'describe-db-instance-automated-backups': { DBInstanceAutomatedBackups: [] },
  'list-backup-vaults': { BackupVaultList: [{ BackupVaultName: 'fixture-vault' }] },
  'list-recovery-points-by-backup-vault': { RecoveryPoints: [{ RecoveryPointArn: 'fixture-snapshot', Status: 'COMPLETED', CreationDate: ago(1), Lifecycle: { DeleteAfterDays: 7 }, CalculatedLifecycle: { DeleteAt: ago(-6) } }] },
}
if (!responses[args[1]]) process.exit(2)
console.log(JSON.stringify(responses[args[1]]))
`, { mode: 0o700 })
    const result = await promisify(execFile)(process.execPath,
      ['scripts/audit-backup-policy.mjs', '--rds-instance', 'fixture-source'],
      { env: { ...process.env, PATH: directory + delimiter + process.env.PATH } })
      .catch((error) => { assert.equal(error.code, 1); return error })
    const report = JSON.parse(result.stdout)
    assert.equal(report.source_inventory_compliant, false)
    assert.ok(report.findings.includes('snapshot copy contains data older than the allowed restore window'))
  } finally { await rm(directory, { recursive: true, force: true }) }
})
