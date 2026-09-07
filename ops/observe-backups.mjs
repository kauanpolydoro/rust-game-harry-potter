import { execFileSync } from 'node:child_process'
import { pathToFileURL } from 'node:url'

/** Only projected RDS metadata reaches this boundary; no DB identity is emitted. */
export function backupObservations(metadata, manualSnapshots, now = Date.now()) {
  const earliest = Date.parse(metadata?.earliest)
  const latest = Date.parse(metadata?.latest)
  const retention = metadata?.retention
  if (!Number.isFinite(earliest) || !Number.isFinite(latest) ||
      earliest > latest || latest > now ||
      !Number.isInteger(retention) || retention < 1 || retention > 35 ||
      !Number.isInteger(manualSnapshots) || manualSnapshots < 0) {
    return [['backup_measurements', 1, 'unavailable']]
  }
  return [
    ['backup_rpo_seconds', (now - latest) / 1_000, 'success'],
    ['backup_window_seconds', (now - earliest) / 1_000, 'success'],
    ['backup_retention_days', retention, 'success'],
    ['backup_manual_snapshots', manualSnapshots, 'success'],
    ['backup_measurements', 1, 'success'],
  ]
}

function writeObservations(observations) {
  const environment = process.env.TELEMETRY_ENVIRONMENT
  if (!['development', 'staging', 'production'].includes(environment)) {
    process.stderr.write('TELEMETRY_ENVIRONMENT must be development, staging or production\n')
    process.exitCode = 1
    return
  }
  for (const [metric, value, outcome] of observations) {
    process.stdout.write(`${JSON.stringify({
      environment, operation: 'backup', outcome, metric, value, [metric]: value,
      _aws: { Timestamp: Date.now(), CloudWatchMetrics: [{
        Namespace: 'Hogwarts',
        Dimensions: [['environment', 'operation'], ['environment', 'operation', 'outcome']],
        Metrics: [{ Name: metric, Unit: metric.endsWith('_seconds') ? 'Seconds' : 'Count' }],
      }] },
    })}\n`)
  }
}

function describe(command, query, extra = []) {
  return JSON.parse(execFileSync('aws', [
    'rds', command, '--db-instance-identifier', process.env.RDS_DB_INSTANCE_ID,
    '--query', query, '--output', 'json', '--no-cli-pager', ...extra,
  ], { encoding: 'utf8', timeout: 10_000, maxBuffer: 64 * 1024, stdio: ['ignore', 'pipe', 'pipe'] }))
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  let observations
  try {
    if (!process.env.RDS_DB_INSTANCE_ID) throw new Error('missing configuration')
    const metadata = describe('describe-db-instance-automated-backups',
      "DBInstanceAutomatedBackups[?Status=='active'] | [0].{earliest:RestoreWindow.EarliestTime,latest:RestoreWindow.LatestTime,retention:BackupRetentionPeriod}")
    const manual = describe('describe-db-snapshots', 'length(DBSnapshots)', ['--snapshot-type', 'manual'])
    observations = backupObservations(metadata, manual)
  } catch {
    // AWS errors may include resource names, endpoints and credential details.
    observations = [['backup_measurements', 1, 'unavailable']]
  }
  writeObservations(observations)
  if (observations[0][2] === 'unavailable') process.exitCode = 1
}
