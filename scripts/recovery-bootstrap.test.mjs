import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import test from 'node:test'
import vm from 'node:vm'

const repositoryRoot = resolve(import.meta.dirname, '..')

test('recovery bootstrap removes the fragment before the application entrypoint', async () => {
  const [document, bootstrap] = await Promise.all([
    readFile(resolve(repositoryRoot, 'apps/web/index.html'), 'utf8'),
    readFile(resolve(repositoryRoot, 'apps/web/public/recovery-bootstrap.js'), 'utf8'),
  ])
  const bootstrapPosition = document.indexOf('<script src="/recovery-bootstrap.js"></script>')
  const manifestPosition = document.indexOf('<link rel="manifest"')
  const applicationPosition = document.indexOf('<script type="module" src="/src/main.ts"></script>')
  assert.ok(bootstrapPosition >= 0)
  assert.equal(document.indexOf('<script'), bootstrapPosition)
  assert.match(document, /<head>\s*<script src="\/recovery-bootstrap\.js"><\/script>/)
  assert.ok(bootstrapPosition < manifestPosition)
  assert.ok(bootstrapPosition < applicationPosition)

  const token = 'a'.repeat(64)
  const replacements = []
  const window = {
    history: {
      replaceState: (...values) => replacements.push(values),
      state: { preserved: true },
    },
    location: {
      hash: `#recovery=${token}`,
      pathname: '/play',
      search: '?language=pt-BR',
    },
  }
  vm.runInNewContext(bootstrap, { URLSearchParams, window })

  assert.equal(window.__HOGWARTS_RECOVERY_TOKEN__, token)
  assert.deepEqual(replacements, [
    [{ preserved: true }, '', '/play?language=pt-BR'],
  ])
})
