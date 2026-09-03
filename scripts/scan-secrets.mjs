import { readdir, readFile } from 'node:fs/promises'
import { relative, resolve } from 'node:path'

const repositoryRoot = resolve(import.meta.dirname, '..')
const ignoredDirectories = new Set([
  '.git',
  'dist',
  'node_modules',
  'playwright-report',
  'target',
  'test-results',
])
const signatures = [
  new RegExp(['AK', 'IA[0-9A-Z]{16}'].join('')),
  new RegExp(['gh', '[pousr]_[A-Za-z0-9]{30,}'].join('')),
  new RegExp(['sk-', 'proj-[A-Za-z0-9_-]{20,}'].join('')),
  new RegExp(['-----BEGIN ', '(?:RSA |EC |OPENSSH )?PRIVATE KEY-----'].join('')),
]

async function filesUnder(directory) {
  const entries = await readdir(directory, { withFileTypes: true })
  const files = []

  for (const entry of entries) {
    if (entry.isDirectory() && ignoredDirectories.has(entry.name)) {
      continue
    }

    const path = resolve(directory, entry.name)
    if (entry.isDirectory()) {
      files.push(...(await filesUnder(path)))
    } else if (entry.isFile()) {
      files.push(path)
    }
  }

  return files
}

const findings = []
for (const path of await filesUnder(repositoryRoot)) {
  const contents = await readFile(path).catch(() => Buffer.alloc(0))
  if (contents.includes(0)) {
    continue
  }

  const text = contents.toString('utf8')
  if (signatures.some((signature) => signature.test(text))) {
    findings.push(relative(repositoryRoot, path))
  }
}

if (findings.length > 0) {
  console.error(`Potential secrets found in: ${findings.join(', ')}`)
  process.exitCode = 1
}
