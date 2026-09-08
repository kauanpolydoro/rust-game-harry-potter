import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { promisify } from 'node:util'
import { pathToFileURL } from 'node:url'

const execute = promisify(execFile)
const day = 86_400_000
const instant = (value) => typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value * 1000
  : typeof value === 'string' && Number.isFinite(Date.parse(value)) ? Date.parse(value) : NaN

// Input preserves native AWS response shapes, plus an observation timestamp.
// Every collection is mandatory: missing inventory is not proof of absence.
export function auditBackupInventory(inventory, now = Date.now()) {
  const findings = []
  const fail = (message) => findings.push(message)
  if (!Number.isFinite(now) || !Number.isFinite(instant(inventory.observed_at))
    || now - instant(inventory.observed_at) > 60_000 || instant(inventory.observed_at) > now) fail('backup inventory is stale or undated')
  for (const collection of ['DBInstances', 'DBSnapshots', 'DBInstanceAutomatedBackups', 'RecoveryPoints']) {
    if (!Array.isArray(inventory[collection])) fail(`missing ${collection} inventory`)
  }
  if (findings.length) return findings
  if (inventory.DBInstances.length !== 1) fail('exactly one source instance is required')
  for (const instance of inventory.DBInstances) {
    if (!Number.isInteger(instance.BackupRetentionPeriod) || instance.BackupRetentionPeriod < 1 || instance.BackupRetentionPeriod > 7) fail('RDS retention must be enabled and at most seven days')
    if (instance.DBInstanceStatus !== 'available') fail('source must be available; stopped time extends RDS retention')
    checkWindow(instance, now, fail)
  }
  for (const snapshot of inventory.DBSnapshots) {
    if (snapshot.SnapshotType !== 'automated' && (snapshot.SnapshotType !== 'awsbackup' || !snapshot.DBSnapshotArn
      || !inventory.RecoveryPoints.some((point) => point.RecoveryPointArn === snapshot.DBSnapshotArn))) {
      fail('manual or untracked snapshot is prohibited; a tag is not a deletion controller')
      continue
    }
    const original = instant(snapshot.SnapshotDatabaseTime ?? snapshot.OriginalSnapshotCreateTime ?? snapshot.SnapshotCreateTime)
    if (!Number.isFinite(original) || original < now - 7 * day || original > now) fail('snapshot copy contains data older than the allowed restore window')
  }
  for (const backup of inventory.DBInstanceAutomatedBackups) {
    if (!Number.isInteger(backup.BackupRetentionPeriod) || backup.BackupRetentionPeriod < 1 || backup.BackupRetentionPeriod > 7) fail('retained automated backup exceeds seven days or has no retention')
    checkWindow({ EarliestRestorableTime: backup.RestoreWindow?.EarliestTime,
      LatestRestorableTime: backup.RestoreWindow?.LatestTime }, now, fail, false)
  }
  for (const recovery of inventory.RecoveryPoints) {
    const created = instant(recovery.CreationDate)
    const deletion = instant(recovery.CalculatedLifecycle?.DeleteAt)
    const retention = recovery.Lifecycle?.DeleteAfterDays
    if (!Number.isInteger(retention) || retention < 1 || retention > 7
      || !Number.isFinite(created) || created > now || created < now - 7 * day
      || !Number.isFinite(deletion) || deletion <= now || deletion > created + 7 * day
      || recovery.Status !== 'COMPLETED') fail('AWS Backup recovery point is expired, immutable beyond seven days or lacks bounded deletion')
  }
  return [...new Set(findings)]
}
function checkWindow(window, now, fail, checkRpo = true) {
  const earliest = instant(window.EarliestRestorableTime)
  const latest = instant(window.LatestRestorableTime)
  if (!Number.isFinite(earliest) || !Number.isFinite(latest) || earliest < now - 7 * day
    || earliest > latest || latest > now) fail('advertised PITR window exceeds seven days or is unavailable')
  if (checkRpo && (!Number.isFinite(latest) || now - latest > 300_000)) fail('latest restorable time exceeds five-minute RPO')
}
async function aws(...args) {
  try {
    const result = await execute('aws', [...args, '--output', 'json'], { maxBuffer: 16 * 1024 * 1024 })
    return JSON.parse(result.stdout)
  } catch { throw new Error('AWS backup inventory unavailable; policy cannot be certified') }
}
async function collect(identifier) {
  const { DBInstances } = await aws('rds', 'describe-db-instances', '--db-instance-identifier', identifier)
  if (DBInstances.length !== 1) throw new Error('source instance is ambiguous')
  const instance = DBInstances[0]
  const [snapshots, backupSnapshots, automated, vaults] = await Promise.allSettled([
    aws('rds', 'describe-db-snapshots', '--db-instance-identifier', identifier),
    aws('rds', 'describe-db-snapshots', '--db-instance-identifier', identifier, '--snapshot-type', 'awsbackup'),
    aws('rds', 'describe-db-instance-automated-backups', '--dbi-resource-id', instance.DbiResourceId),
    aws('backup', 'list-backup-vaults'),
  ])
  for (const result of [snapshots, backupSnapshots, automated, vaults]) if (result.status === 'rejected') throw result.reason
  const points = []
  for (const vault of vaults.value.BackupVaultList) {
    const listing = await aws('backup', 'list-recovery-points-by-backup-vault', '--backup-vault-name', vault.BackupVaultName,
      '--by-resource-arn', instance.DBInstanceArn)
    points.push(...listing.RecoveryPoints)
  }
  return { observed_at: new Date().toISOString(), DBInstances,
    DBSnapshots: [...snapshots.value.DBSnapshots, ...backupSnapshots.value.DBSnapshots],
    DBInstanceAutomatedBackups: automated.value.DBInstanceAutomatedBackups, RecoveryPoints: points }
}
async function main() {
  const [mode, value] = process.argv.slice(2)
  const inventory = mode === '--rds-instance' ? await collect(value)
    : mode === '--inventory' ? JSON.parse(await readFile(value, 'utf8')) : null
  if (!inventory) throw new Error('use --rds-instance ID or --inventory PATH')
  const findings = auditBackupInventory(inventory)
  console.log(JSON.stringify({ source_inventory_compliant: findings.length === 0, scope: 'specified source in configured AWS account and region', external_copies_verified: false, findings }))
  if (findings.length) process.exitCode = 1
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main()
