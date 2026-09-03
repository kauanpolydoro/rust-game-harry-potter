import assert from 'node:assert/strict'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'

import { checkVueAccessibility } from './check-vue-a11y.mjs'

const repositoryRoot = resolve(import.meta.dirname, '..')

test('rejects static DOM event attributes', () => {
  const fixture = '<template><button onclick="retry()">Run</button></template>'
  assert.throws(
    () => checkVueAccessibility(fixture, 'fixture.vue'),
    /forbidden static event attribute onclick/,
  )
})

test('rejects invalid static ARIA values', () => {
  for (const [attribute, expectedError] of [
    ['aria-busy="yes"', /invalid aria-busy value yes/],
    ['aria-live="polit"', /invalid aria-live value polit/],
  ]) {
    assert.throws(
      () => checkVueAccessibility(`<template><main ${attribute}></main></template>`, 'fixture.vue'),
      expectedError,
    )
  }
})

test('rejects missing and duplicate accessibility targets', () => {
  assert.throws(
    () =>
      checkVueAccessibility(
        '<template><main aria-labelledby="missing"></main></template>',
        'fixture.vue',
      ),
    /references missing id missing from aria-labelledby/,
  )
  assert.throws(
    () =>
      checkVueAccessibility(
        '<template><div id="same"></div><div id="same"></div></template>',
        'fixture.vue',
      ),
    /invalid or duplicate id same/,
  )
})

test('accepts child components, slots, and framework-typed bindings', () => {
  const fixture = `<script setup lang="ts">
import ChildPanel from './ChildPanel.vue'
const busy = true
</script>
<template><ChildPanel :busy="busy"><slot /></ChildPanel></template>
`
  assert.doesNotThrow(() => checkVueAccessibility(fixture, 'fixture.vue'))
})

test('the official Vue checker rejects script and template type errors', async () => {
  const sourceRoot = resolve(repositoryRoot, 'apps/web/src')
  const fixturePath = resolve(sourceRoot, `FixtureTypecheck-${process.pid}.vue`)

  try {
    await writeFile(
      fixturePath,
      `<script setup lang="ts">
const count: number = 'not a number'
</script>
<template>{{ missingName }} {{ count }}</template>
`,
    )
    const result = spawnSync('npm', ['run', 'typecheck', '--workspace', '@hogwarts/web'], {
      cwd: repositoryRoot,
      encoding: 'utf8',
    })

    assert.notEqual(result.status, 0)
    assert.match(`${result.stdout}${result.stderr}`, /not assignable to type 'number'/)
    assert.match(`${result.stdout}${result.stderr}`, /missingName/)
  } finally {
    await rm(fixturePath, { force: true })
  }
})

test('the accessibility CLI checks every Vue file without generated snapshots', async () => {
  const temporaryDirectory = await mkdtemp(resolve(tmpdir(), 'hogwarts-vue-a11y-'))
  const environment = { ...process.env, VUE_A11Y_SOURCE_ROOT: temporaryDirectory }

  try {
    await writeFile(
      resolve(temporaryDirectory, 'Fixture.vue'),
      '<template><section aria-describedby="missing"></section></template>',
    )
    const result = spawnSync(process.execPath, [resolve(repositoryRoot, 'scripts/check-vue-a11y.mjs')], {
      encoding: 'utf8',
      env: environment,
    })
    assert.notEqual(result.status, 0)
    assert.match(`${result.stdout}${result.stderr}`, /references missing id missing/)
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true })
  }
})
