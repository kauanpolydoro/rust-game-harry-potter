import assert from 'node:assert/strict'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'

import { compileVueForTypecheck } from './generate-vue-types.mjs'

const repositoryRoot = resolve(import.meta.dirname, '..')
const sourceRoot = resolve(repositoryRoot, 'apps/web/src')

function runTypeScript(generatedPath) {
  return spawnSync(
    resolve(repositoryRoot, 'node_modules/.bin/tsc'),
    [
      '--noEmit',
      '--strict',
      '--skipLibCheck',
      '--target',
      'ES2024',
      '--module',
      'ESNext',
      '--moduleResolution',
      'Bundler',
      '--lib',
      'ES2024,DOM,DOM.Iterable',
      generatedPath,
    ],
    { encoding: 'utf8' },
  )
}

test('makes the type gate reject a semantic script error inside a Vue SFC', async () => {
  const temporaryDirectory = await mkdtemp(resolve(sourceRoot, '.vue-typecheck-'))

  try {
    const fixture = `<script setup lang="ts">\nconst count: number = 'not a number'\n</script>\n<template>{{ count }}</template>\n`
    const generated = compileVueForTypecheck(fixture, 'fixture.vue')
    const generatedPath = resolve(temporaryDirectory, 'fixture.vue.typecheck.generated.ts')
    await writeFile(generatedPath, generated)

    const result = runTypeScript(generatedPath)

    assert.notEqual(result.status, 0)
    assert.match(`${result.stdout}${result.stderr}`, /not assignable to type 'number'/)
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true })
  }
})

test('makes the type gate reject an unknown template binding', async () => {
  const temporaryDirectory = await mkdtemp(resolve(sourceRoot, '.vue-typecheck-'))

  try {
    const fixture = `<script setup lang="ts">\nconst count = 1\n</script>\n<template>{{ missingName }} {{ count }}</template>\n`
    const generated = compileVueForTypecheck(fixture, 'fixture.vue')
    const generatedPath = resolve(temporaryDirectory, 'fixture.vue.typecheck.generated.ts')
    await writeFile(generatedPath, generated)

    const result = runTypeScript(generatedPath)

    assert.notEqual(result.status, 0)
    assert.match(`${result.stdout}${result.stderr}`, /Property 'missingName' does not exist/)
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true })
  }
})

test('rejects a direct DOM handler reference whose event contract cannot be checked', () => {
  const fixture = `<script setup lang="ts">\nfunction handler(value: number): void { void value }\n</script>\n<template><button @click="handler">Run</button></template>\n`

  assert.throws(
    () => compileVueForTypecheck(fixture, 'fixture.vue'),
    /event @click must call a typed handler inline without \$event/,
  )
})

test('rejects static DOM event attributes', () => {
  const fixture = `<script setup lang="ts">\nfunction retry(): void {}\n</script>\n<template><button onclick="retry()">Run</button></template>\n`

  assert.throws(
    () => compileVueForTypecheck(fixture, 'fixture.vue'),
    /forbidden static event attribute onclick/,
  )
})

test('makes the type gate reject object, function, and invalid string ARIA values', async () => {
  const invalidValues = [
    ['const value = { busy: true }', 'value'],
    ['const value = () => true', 'value'],
    ["const value = 'yes' as const", 'value'],
  ]

  for (const [declaration, expression] of invalidValues) {
    const temporaryDirectory = await mkdtemp(resolve(sourceRoot, '.vue-typecheck-'))

    try {
      const fixture = `<script setup lang="ts">\n${declaration}\n</script>\n<template><section :aria-busy="${expression}"></section></template>\n`
      const generated = compileVueForTypecheck(fixture, 'fixture.vue')
      const generatedPath = resolve(temporaryDirectory, 'fixture.vue.typecheck.generated.ts')
      await writeFile(generatedPath, generated)

      const result = runTypeScript(generatedPath)

      assert.notEqual(result.status, 0, `expected ${declaration} to fail the ARIA type check`)
      assert.match(`${result.stdout}${result.stderr}`, /does not satisfy/)
    } finally {
      await rm(temporaryDirectory, { force: true, recursive: true })
    }
  }
})

test('rejects invalid static ARIA values and roles', () => {
  const invalidAttributes = [
    ['aria-busy="yes"', /invalid aria-busy value yes/],
    ['aria-live="polit"', /invalid aria-live value polit/],
    ['role="wizard"', /unsupported role wizard/],
  ]

  for (const [attribute, expectedError] of invalidAttributes) {
    const fixture = `<script setup lang="ts">\nconst ready = true\n</script>\n<template><main ${attribute}>{{ ready }}</main></template>\n`
    assert.throws(() => compileVueForTypecheck(fixture, 'fixture.vue'), expectedError)
  }
})

test('rejects a missing aria-labelledby target', () => {
  const fixture = `<script setup lang="ts">\nconst ready = true\n</script>\n<template><main aria-labelledby="missing">{{ ready }}</main></template>\n`

  assert.throws(
    () => compileVueForTypecheck(fixture, 'fixture.vue'),
    /references missing id missing from aria-labelledby/,
  )
})

test('rejects child component props until the official TypeScript 7 checker supports them', () => {
  const fixture = `<script setup lang="ts">\nconst Child = {}\n</script>\n<template><Child :count="'wrong'" /></template>\n`

  assert.throws(
    () => compileVueForTypecheck(fixture, 'fixture.vue'),
    /child components and slots require the official TypeScript 7 Vue checker/,
  )
})

test('the public typecheck command rejects a stale SFC snapshot', async () => {
  const temporaryDirectory = await mkdtemp(resolve(tmpdir(), 'hogwarts-vue-typecheck-'))
  const sourcePath = resolve(temporaryDirectory, 'Fixture.vue')
  const environment = { ...process.env, VUE_TYPECHECK_SOURCE_ROOT: temporaryDirectory }

  try {
    await writeFile(sourcePath, '<script setup lang="ts">\nconst value = 1\n</script>\n<template>{{ value }}</template>\n')
    const generated = spawnSync(
      process.execPath,
      [resolve(repositoryRoot, 'scripts/generate-vue-types.mjs')],
      { encoding: 'utf8', env: environment },
    )
    assert.equal(generated.status, 0, `${generated.stdout}${generated.stderr}`)

    await writeFile(sourcePath, '<script setup lang="ts">\nconst value = 2\n</script>\n<template>{{ value }}</template>\n')
    const typecheck = spawnSync(
      'npm',
      ['run', 'typecheck', '--workspace', '@hogwarts/web'],
      { cwd: repositoryRoot, encoding: 'utf8', env: environment },
    )

    assert.notEqual(typecheck.status, 0)
    assert.match(`${typecheck.stdout}${typecheck.stderr}`, /Fixture\.vue\.typecheck\.generated\.ts is stale/)
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true })
  }
})
