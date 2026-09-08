// Real PostgreSQL base backup + archived WAL, isolated from the development DB.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { randomBytes, randomUUID } from 'node:crypto'
import { chmod, mkdir, mkdtemp, open, readFile, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { setTimeout as delay } from 'node:timers/promises'

const id = `hogwarts-dr-${randomUUID()}`
const privateDirectory = await mkdtemp(`${tmpdir()}/hp-dr-`)
await chmod(privateDirectory, 0o700)
await mkdir(resolve('.scratch'), { recursive: true })
const source = `${id}-source`
const target = `${id}-target`
const volumes = ['source', 'backup', 'wal', 'target'].map((name) => `${id}-${name}-data`)
const children = new Set()
const containers = new Set()
const createdVolumes = new Set()
let networkCreated = false
const sourceLogin = 'dr_source'
const targetLogin = 'dr_target'
const sourcePassword = randomBytes(24).toString('hex')
const targetPassword = randomBytes(24).toString('hex')
const sourceKey = randomBytes(16).toString('hex')
const targetKey = randomBytes(16).toString('hex')
const ledgerKey = randomBytes(16).toString('hex')
const sourceEpoch = randomUUID()
const targetEpoch = randomUUID()
const ledgerDirectory = `${privateDirectory}/ledger`
const sourceSocket = `${privateDirectory}/source-socket`
const targetSocket = `${privateDirectory}/target-socket`
const origin = 'http://127.0.0.1:5173'
const reportPath = resolve('.scratch/restore-profile.json')
let image
let directoryOwner
let sourceApp
let sourceWorker
let targetApp
let targetWorker

async function run(command, args, { input, env, allowFailure = false, timeout = 120_000 } = {}) {
  return new Promise((resolvePromise, reject) => {
    const process = spawn(command, args, { env: env ?? processEnv(), stdio: ['pipe', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    process.stdout.on('data', (data) => { stdout += data })
    process.stderr.on('data', (data) => { stderr += data })
    const timer = setTimeout(() => process.kill('SIGKILL'), timeout)
    process.on('error', reject)
    process.on('close', (code) => {
      clearTimeout(timer)
      if (code !== 0 && !allowFailure) {
        // Full diagnostics remain local and private; never echo credentials/SQL.
        writeFile(`${privateDirectory}/command-error.txt`, stderr, { mode: 0o600 })
          .then(() => reject(new Error(`${command} failed with exit ${code}; private diagnostics: ${privateDirectory}`)), reject)
      } else resolvePromise({ stdout: stdout.trim(), code })
    })
    process.stdin.on('error', () => {})
    process.stdin.end(input)
  })
}
function processEnv() { return process.env }
async function docker(...args) { return (await run('docker', args)).stdout }
async function sql(container, statement, login = sourceLogin) {
  return (await run('docker', ['exec', '-i', '-e', `PGPASSWORD=${login === sourceLogin ? sourcePassword : targetPassword}`, container, 'psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '-U', login, '-d', 'hogwarts', '-f', '-'], { input: statement })).stdout
}
function literal(value) { return `'${value.replaceAll("'", "''")}'` }
async function until(label, condition, milliseconds = 60_000) {
  const deadline = Date.now() + milliseconds
  while (Date.now() < deadline) {
    if (await condition()) return
    await delay(200)
  }
  throw new Error(`Timed out: ${label}`)
}
async function port() {
  const listener = createServer()
  await new Promise((done) => listener.listen(0, '127.0.0.1', done))
  const value = listener.address().port
  await new Promise((done) => listener.close(done))
  return value
}
async function launch(binary, env) {
  const log = await open(`${privateDirectory}/${binary}-${randomUUID()}.log`, 'w', 0o600)
  const child = spawn(resolve('target/debug', binary), [], { env: { ...process.env, ...env }, stdio: ['ignore', log.fd, log.fd] })
  await log.close()
  children.add(child)
  child.once('exit', () => children.delete(child))
  return child
}
async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  const exited = new Promise((done) => child.once('exit', done))
  child.kill('SIGTERM')
  const timer = setTimeout(() => child.kill('SIGKILL'), 10_000)
  await exited
  clearTimeout(timer)
}
function environment(url, epoch, key) {
  return { DATABASE_URL: url, DEPLOYMENT_EPOCH: epoch, SESSION_TOKEN_KEY: key,
    TOMBSTONE_HMAC_KEY: ledgerKey, TOMBSTONE_LOCAL_DIRECTORY: ledgerDirectory, APPLICATION_ORIGIN: origin }
}
async function request(base, method, path, payload, cookie, expected = 200) {
  const response = await fetch(`${base}${path}`, {
    method, headers: { origin, 'x-csrf-protection': '1', 'content-type': 'application/json',
      'idempotency-key': randomUUID(), ...(cookie ? { cookie } : {}) },
    ...(payload ? { body: JSON.stringify(payload) } : {}), signal: AbortSignal.timeout(10_000),
  })
  assert.equal(response.status, expected, `${method} ${path.replace(/[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{8}/g, ':room')} status`)
  const body = response.status === 503 ? null : await response.json()
  return { body, cookie: response.headers.get('set-cookie')?.split(';')[0] }
}
async function health(base) {
  try { return (await fetch(`${base}/health/ready`, { signal: AbortSignal.timeout(1000) })).status } catch { return 0 }
}
async function createGame(base, manifest) {
  const host = await request(base, 'POST', '/api/rooms', { display_name: 'Fixture host', recovery_password: 'a long uncommon recovery drill passphrase' }, null, 201)
  const code = host.body.room.code
  assert.match(code, /^[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{8}$/)
  await request(base, 'PUT', '/api/session/hero', { hero_id: 'harry' }, host.cookie)
  const guest = await request(base, 'POST', `/api/rooms/${code}/participants`, { display_name: 'Fixture guest', hero_id: 'hermione' }, null, 201)
  for (const player of [host, guest]) await request(base, 'PUT', '/api/session/readiness', { ready: true }, player.cookie)
  await request(base, 'POST', '/api/games', { adventure_id: 'adventure:001', manifest_digest: manifest, ruleset_version: 'game-one-v1' }, host.cookie, 201)
  return { code, players: [host, guest] }
}
async function advance(base, game) {
  let projection = (await request(base, 'GET', '/api/session', null, game.players[0].cookie)).body
  const position = projection.choice.status === 'pending' ? projection.choice.responsible_position : projection.turn.active_position
  const cookie = game.players[position - 1].cookie
  projection = (await request(base, 'GET', '/api/session', null, cookie)).body
  const intent = projection.choice.status === 'pending'
    ? { type: 'resolve_choice', choice_id: projection.choice.id, selected_options: projection.choice.options.slice(0, projection.choice.min) }
    : { type: 'end_hero_actions' }
  return (await request(base, 'POST', '/api/games/current/commands', {
    command_id: randomUUID(), expected_state_version: projection.snapshot.state_version, ...intent,
  }, cookie)).body.projection
}
async function gameState(container, game) {
  return JSON.parse(await sql(container, `SELECT json_build_object('snapshot', snapshot, 'digest', state_digest, 'seed', encode(prng_seed, 'hex'), 'counter', prng_counter, 'expires', expires_at::TEXT, 'last_action', last_game_action_at::TEXT, 'sequence', sequence, 'events', (SELECT count(*) FROM game_events WHERE game_id = games.id), 'receipts', (SELECT count(*) FROM game_command_receipts WHERE game_id = games.id)) FROM games WHERE room_id = (SELECT id FROM rooms WHERE code = ${literal(game.code)});`))
}

async function exercise() {
  const config = JSON.parse(await docker('compose', 'config', '--format', 'json'))
  image = config.services.postgres.image
  await docker('image', 'inspect', image)
  directoryOwner = await docker('run', '--rm', '--network', 'none', '-v', `${privateDirectory}:/drill`, image, 'stat', '-c', '%u:%g', '/drill')
  assert.match(directoryOwner, /^\d+:\d+$/)
  await docker('network', 'create', '--internal', id)
  networkCreated = true
  for (const volume of volumes) { await docker('volume', 'create', volume); createdVolumes.add(volume) }
  for (const socket of [sourceSocket, targetSocket]) { await mkdir(socket, { mode: 0o777 }); await chmod(socket, 0o777) }
  containers.add(source)
  await docker('run', '-d', '--name', source, '--network', id,
    '-e', `POSTGRES_USER=${sourceLogin}`, '-e', `POSTGRES_PASSWORD=${sourcePassword}`, '-e', 'POSTGRES_DB=hogwarts', '-e', `PGPASSWORD=${sourcePassword}`, '-e', 'POSTGRES_INITDB_ARGS=--auth-local=scram-sha-256 --auth-host=scram-sha-256',
    '-v', `${sourceSocket}:/var/run/postgresql`, '-v', `${volumes[0]}:/var/lib/postgresql`, '-v', `${volumes[1]}:/backup`, '-v', `${volumes[2]}:/archive`,
    image, 'postgres', '-c', 'archive_mode=on', '-c', 'archive_timeout=60s',
    '-c', 'archive_command=test ! -f /archive/%f && cp %p /archive/%f',
    '-c', 'log_error_verbosity=terse', '-c', 'log_min_error_statement=panic', '-c', 'log_statement=none')
  await docker('exec', '-u', 'root', source, 'chown', 'postgres:postgres', '/backup', '/archive')
  await until('source PostgreSQL', async () => (await run('docker', ['exec', source, 'pg_isready', '-U', sourceLogin, '-d', 'hogwarts'], { allowFailure: true })).code === 0)
  const sourceUrl = `postgres://${sourceLogin}:${sourcePassword}@localhost/hogwarts?host=${encodeURIComponent(sourceSocket)}`
  const sourceEnv = environment(sourceUrl, sourceEpoch, sourceKey)
  const sourceHttpPort = await port()
  const sourceBase = `http://127.0.0.1:${sourceHttpPort}`
  sourceApp = await launch('harry-potter-server', { ...sourceEnv, BIND_ADDRESS: `127.0.0.1:${sourceHttpPort}` })
  await until('source application', async () => await health(sourceBase) === 200)
  await run(resolve('target/debug/source-bootstrap'), [], { env: { ...process.env, ...sourceEnv } })
  sourceWorker = await launch('lifecycle-worker', sourceEnv)
  await until('ledger binding', async () => await sql(source, 'SELECT ledger_key_fingerprint IS NOT NULL FROM runtime_deployment;') === 't')
  const manifest = await sql(source, "SELECT digest FROM content_manifests WHERE ruleset_version = 'game-one-v1' AND playable;")
  const games = []
  for (let index = 0; index < 3; index++) games.push(await createGame(sourceBase, manifest))
  console.log('restore drill: created three games through HTTP; taking a physical base backup')
  await docker('exec', source, 'pg_basebackup', '-U', sourceLogin, '-D', '/backup/base', '-X', 'stream', '--checkpoint=fast')
  await stop(sourceWorker)
  // All official commands below are absent from the base backup and require WAL replay.
  for (const game of games) await advance(sourceBase, game)
  // Controlled time boundary: it is still valid at the recovery target, expired by restore.
  await sql(source, `UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '7 days', expires_at = clock_timestamp() + INTERVAL '3 seconds' WHERE room_id = (SELECT id FROM rooms WHERE code = ${literal(games[1].code)});`)
  const expected = await gameState(source, games[2])
  await sql(source, "UPDATE runtime_deployment SET checkpoint_at = clock_timestamp(); SELECT pg_create_restore_point('issue33_recovery_target');")
  const restorePointMs = Number(await sql(source, 'SELECT (extract(epoch FROM clock_timestamp()) * 1000)::BIGINT;'))
  // A post-target action and a post-target deletion must not be replayed as live state.
  await advance(sourceBase, games[2])
  await sql(source, `UPDATE games SET last_game_action_at = clock_timestamp() - INTERVAL '8 days', expires_at = clock_timestamp() - INTERVAL '1 second' WHERE room_id = (SELECT id FROM rooms WHERE code = ${literal(games[0].code)});`)
  sourceWorker = await launch('lifecycle-worker', sourceEnv)
  await until('post-backup source purge', async () => await sql(source, `SELECT count(*) FROM games WHERE room_id = (SELECT id FROM rooms WHERE code = ${literal(games[0].code)});`) === '0')
  await stop(sourceWorker)
  const archivedWal = await sql(source, 'SELECT pg_walfile_name(pg_current_wal_lsn());')
  await sql(source, 'SELECT pg_switch_wal();')
  await until('durable archive', async () => (await run('docker', ['exec', source, 'test', '-f', `/archive/${archivedWal}`], { allowFailure: true })).code === 0)
  const disasterMs = Date.now()
  const recoveryStarted = performance.now()
  await stop(sourceApp)
  await docker('stop', source)
  console.log('restore drill: source stopped; restoring base backup plus archived WAL in isolation')
  await docker('run', '--rm', '--network', 'none', '-v', `${volumes[1]}:/backup:ro`, '-v', `${volumes[3]}:/restored`, image,
    'sh', '-ec', 'mkdir -p /restored/18/docker; cp -a /backup/base/. /restored/18/docker/; touch /restored/18/docker/recovery.signal; chown -R postgres:postgres /restored/18/docker; chmod 700 /restored/18/docker')
  containers.add(target)
  await docker('run', '-d', '--name', target, '--network', id,
    '-v', `${targetSocket}:/var/run/postgresql`, '-v', `${volumes[3]}:/var/lib/postgresql`, '-v', `${volumes[2]}:/archive:ro`, '-e', `PGPASSWORD=${sourcePassword}`, image,
    'postgres', '-c', 'archive_mode=off', '-c', 'restore_command=cp /archive/%f %p',
    '-c', 'recovery_target_name=issue33_recovery_target', '-c', 'recovery_target_action=promote',
    '-c', 'log_error_verbosity=terse', '-c', 'log_min_error_statement=panic', '-c', 'log_statement=none')
  await until('PITR target promotion', async () => {
    const result = await run('docker', ['exec', target, 'psql', '-X', '-qAt', '-U', sourceLogin, '-d', 'hogwarts', '-c', 'SELECT NOT pg_is_in_recovery();'], { allowFailure: true })
    return result.code === 0 && result.stdout === 't'
  })
  assert.equal(await docker('network', 'inspect', '--format', '{{.Internal}}', id), 'true')
  assert.deepEqual(JSON.parse(await docker('inspect', target))[0].HostConfig.PortBindings ?? {}, {})
  assert.equal(await sql(target, 'SELECT count(*) FROM games;'), '3', 'the deleted root must actually be present in the recovered backup')
  const recovered = await gameState(target, games[2])
  assert.deepEqual(recovered, expected, 'WAL must recover exactly the target snapshot, seed, receipts and history')
  assert.equal(recovered.events, 1, 'the base backup had no official command')
  const rpoSeconds = (disasterMs - Date.parse(recovered.last_action)) / 1000
  assert.ok(rpoSeconds >= 0 && rpoSeconds <= 300, `RPO exceeded: ${rpoSeconds}`)
  await sql(target, `CREATE ROLE ${targetLogin} LOGIN SUPERUSER PASSWORD ${literal(targetPassword)}; ALTER ROLE ${sourceLogin} NOLOGIN;`)
  const targetUrl = `postgres://${targetLogin}:${targetPassword}@localhost/hogwarts?host=${encodeURIComponent(targetSocket)}`
  const targetEnv = environment(targetUrl, targetEpoch, targetKey)
  const targetHttpPort = await port()
  const targetBase = `http://127.0.0.1:${targetHttpPort}`
  targetApp = await launch('harry-potter-server', { ...targetEnv, BIND_ADDRESS: `127.0.0.1:${targetHttpPort}` })
  await until('closed target readiness', async () => await health(targetBase) === 503)
  await request(targetBase, 'GET', '/api/session', null, games[2].players[0].cookie, 503)
  const accessFile = `${privateDirectory}/fresh-access.json`
  await run(resolve('target/debug/restore-reconcile'), [], { env: { ...process.env, ...targetEnv,
    RESTORE_POINT_MS: String(restorePointMs), RESTORE_ACCESS_FILE: accessFile } })
  await until('reconciled readiness', async () => await health(targetBase) === 200)
  for (const game of games) {
    for (const player of game.players) {
      await request(targetBase, 'GET', '/api/session', null, player.cookie, 401)
      await request(targetBase, 'POST', '/api/games/current/commands', { type: 'end_hero_actions', command_id: randomUUID(), expected_state_version: 2 }, player.cookie, 401)
      await request(targetBase, 'POST', '/api/session/recover', { recovery_token: player.body.recovery_token,
        recovery_password: 'a long uncommon recovery drill passphrase', recovery_attempt_id: randomUUID() }, null, 401)
    }
  }
  assert.equal(await sql(target, 'SELECT count(*) FROM games WHERE access_expired_at IS NOT NULL;', targetLogin), '2')
  targetWorker = await launch('lifecycle-worker', targetEnv)
  await until('restored purge completion', async () => await sql(target, 'SELECT count(*) FROM games;', targetLogin) === '1'
    && await sql(target, 'SELECT count(*) FROM lifecycle_purge_jobs;', targetLogin) === '0')
  const grants = JSON.parse(await readFile(accessFile, 'utf8'))
  assert.equal(grants.length, 2, 'only surviving participants receive new access')
  const players = []
  for (const grant of grants.sort((left, right) => left.position - right.position)) {
    assert.equal(grant.room_code, games[2].code)
    players.push(await request(targetBase, 'POST', '/api/session/recover', { recovery_token: grant.recovery_token,
      recovery_password: grant.recovery_password, recovery_attempt_id: randomUUID() }))
  }
  const resumed = await advance(targetBase, { players })
  assert.equal(resumed.snapshot.sequence, 2, 'the original game continues with fresh credentials')
  const rtoSeconds = (performance.now() - recoveryStarted) / 1000
  assert.ok(rtoSeconds <= 3600, `RTO exceeded: ${rtoSeconds}`)
  console.log(`restore drill: RPO ${rpoSeconds.toFixed(3)} s; RTO ${rtoSeconds.toFixed(3)} s; old access rejected and surviving game resumed`)
  return { exercised_at: new Date().toISOString(), environment: 'isolated-local-docker', postgres_image: image,
    method: 'pg_basebackup + archived WAL + named recovery target', recovered_games: 3, purged_games: 2,
    surviving_games_resumed: 1, rpo_seconds: rpoSeconds, rto_seconds: rtoSeconds,
    targets: { rpo_seconds: 300, rto_seconds: 3600, restorable_window_days: 7 } }
}

async function cleanup() {
  for (const child of [...children]) await stop(child)
  for (const container of containers) await run('docker', ['rm', '-f', container])
  for (const volume of createdVolumes) await docker('volume', 'rm', volume)
  if (networkCreated) await docker('network', 'rm', id)
  if (directoryOwner) await docker('run', '--rm', '--network', 'none', '-v', `${privateDirectory}:/drill`, image, 'chown', '-R', directoryOwner, '/drill')
}
let report
try {
  report = await exercise()
} finally {
  await cleanup()
  if (report) await rm(privateDirectory, { recursive: true, force: true })
}
report.backup_artifacts_removed = true
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 })
console.log(`restore drill: sanitized evidence written to ${reportPath}`)
