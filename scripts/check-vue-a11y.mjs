import { readdir, readFile } from 'node:fs/promises'
import { dirname, relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { parse } from '@vue/compiler-sfc'
import { ElementTypes, NodeTypes } from '@vue/compiler-dom'

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const sourceRoot = process.env.VUE_A11Y_SOURCE_ROOT
  ? resolve(process.env.VUE_A11Y_SOURCE_ROOT)
  : resolve(repositoryRoot, 'apps/web/src')

const booleanishAriaAttributes = new Set([
  'aria-atomic',
  'aria-busy',
  'aria-disabled',
  'aria-hidden',
])
const enumeratedAriaAttributes = new Map([['aria-live', new Set(['assertive', 'off', 'polite'])]])
const referenceAttributes = new Map([
  ['aria-controls', 'controls'],
  ['aria-describedby', 'describedBy'],
  ['aria-labelledby', 'labelledBy'],
])
const validId = /^[A-Za-z][\w:.-]*$/

async function vueFilesUnder(directory) {
  const entries = await readdir(directory, { withFileTypes: true })
  const files = []

  for (const entry of entries) {
    const path = resolve(directory, entry.name)
    if (entry.isDirectory()) {
      files.push(...(await vueFilesUnder(path)))
    } else if (entry.isFile() && entry.name.endsWith('.vue')) {
      files.push(path)
    }
  }

  return files.sort()
}

export function checkVueAccessibility(source, filename) {
  const { descriptor, errors } = parse(source, { filename })
  if (errors.length > 0) {
    throw new AggregateError(errors, `Vue parsing failed for ${filename}`)
  }
  if (!descriptor.template?.ast) {
    return
  }

  const contract = {
    controls: [],
    describedBy: [],
    formReferences: [],
    ids: new Set(),
    labelledBy: [],
  }
  visitTemplate(descriptor.template.ast, filename, contract)
  for (const [kind, references] of [
    ['aria-controls', contract.controls],
    ['aria-describedby', contract.describedBy],
    ['aria-labelledby', contract.labelledBy],
    ['form', contract.formReferences],
  ]) {
    for (const reference of references) {
      if (!contract.ids.has(reference)) {
        throw new Error(`${filename} references missing id ${reference} from ${kind}`)
      }
    }
  }
}

function visitTemplate(node, filename, contract) {
  if (node.type === NodeTypes.ELEMENT && node.tagType === ElementTypes.ELEMENT) {
    for (const property of node.props) {
      if (property.type === NodeTypes.ATTRIBUTE) {
        validateStaticAttribute(node.tag, property, filename, contract)
      }
    }
  }

  for (const child of node.children ?? []) {
    visitTemplate(child, filename, contract)
  }
}

function validateStaticAttribute(tag, attribute, filename, contract) {
  const name = attribute.name.toLowerCase()
  const value = attribute.value?.content ?? ''

  if (name.startsWith('on')) {
    throw new Error(`${filename} uses forbidden static event attribute ${attribute.name}`)
  }
  if (booleanishAriaAttributes.has(name) && !['false', 'true'].includes(value)) {
    throw new Error(`${filename} uses invalid ${name} value ${value || '[empty]'}`)
  }
  const allowedValues = enumeratedAriaAttributes.get(name)
  if (allowedValues && !allowedValues.has(value)) {
    throw new Error(`${filename} uses invalid ${name} value ${value || '[empty]'}`)
  }
  const referenceKind = referenceAttributes.get(name)
  if (referenceKind) {
    const references = value.split(/\s+/).filter(Boolean)
    if (references.length === 0 || references.some((reference) => !validId.test(reference))) {
      throw new Error(`${filename} uses invalid ${name} value ${value || '[empty]'}`)
    }
    contract[referenceKind].push(...references)
  }
  if (name === 'id') {
    if (!validId.test(value) || contract.ids.has(value)) {
      throw new Error(`${filename} uses invalid or duplicate id ${value || '[empty]'}`)
    }
    contract.ids.add(value)
  }
  if (name === 'form' && tag === 'button') {
    if (!validId.test(value)) {
      throw new Error(`${filename} uses invalid form reference ${value || '[empty]'}`)
    }
    contract.formReferences.push(value)
  }
  if (name === 'for' && tag === 'label') {
    if (!validId.test(value)) {
      throw new Error(`${filename} uses invalid label target ${value || '[empty]'}`)
    }
    contract.controls.push(value)
  }
}

async function checkAll() {
  for (const sourcePath of await vueFilesUnder(sourceRoot)) {
    const source = await readFile(sourcePath, 'utf8')
    checkVueAccessibility(source, relative(repositoryRoot, sourcePath))
  }
}

const invokedPath = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : ''
if (import.meta.url === invokedPath) {
  await checkAll()
}
