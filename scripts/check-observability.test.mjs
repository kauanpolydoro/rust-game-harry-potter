import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const template = JSON.parse(
  await readFile(new URL('../ops/observability.cloudformation.json', import.meta.url), 'utf8'),
)
const resources = Object.values(template.Resources)

test('one observation of every specified P0 pages the configured responder within one period', () => {
  for (const metric of [
    'p0_state_divergence',
    'p0_ack_before_commit',
    'p0_authorization_after_access_end',
    'p0_secret_detected',
    'p0_restore_resurrection',
  ]) {
    const alarm = resources.find((resource) => resource.Properties.MetricName === metric)?.Properties
    assert.ok(alarm, `missing alarm for ${metric}`)
    assert.equal(alarm.Statistic, 'Sum')
    assert.equal(alarm.ComparisonOperator, 'GreaterThanOrEqualToThreshold')
    assert.equal(alarm.Threshold, 1)
    assert.equal(alarm.EvaluationPeriods, 1)
    assert.equal(alarm.DatapointsToAlarm, 1)
    assert.ok(alarm.Period <= 60)
    assert.deepEqual(alarm.AlarmActions, [{ Ref: 'NotificationTopicArn' }])
    assert.match(alarm.AlarmDescription, /ops\/runbooks\.md#/u)
    assert.deepEqual(alarm.Dimensions.map(({ Name }) => Name), ['environment', 'operation'])
  }
})

test('a stopped process or collector cannot look healthy because its heartbeat is missing', () => {
  for (const metric of ['server_heartbeat', 'worker_heartbeat']) {
    const alarm = resources.find((resource) => resource.Properties.MetricName === metric)?.Properties
    assert.ok(alarm)
    assert.equal(alarm.TreatMissingData, 'breaching')
    assert.ok(alarm.Period * alarm.EvaluationPeriods <= 180)
    assert.ok(alarm.AlarmActions.length > 0)
  }
})

test('both process log groups expire identifiable telemetry after seven days', () => {
  const groups = resources.filter(({ Type }) => Type === 'AWS::Logs::LogGroup')
  assert.equal(groups.length, 2)
  for (const { Properties } of groups) {
    assert.equal(Properties.RetentionInDays, 7)
  }
})

test('one overdue unfinished purge alerts even when completed jobs have healthy percentiles', () => {
  const alarm = resources.find(({ Properties }) => Properties.MetricName === 'lifecycle_overdue')?.Properties
  assert.ok(alarm)
  assert.equal(alarm.Threshold, 0)
  assert.equal(alarm.ComparisonOperator, 'GreaterThanThreshold')
  assert.equal(alarm.EvaluationPeriods, 1)
  assert.deepEqual(alarm.AlarmActions, [{ Ref: 'NotificationTopicArn' }])
})
