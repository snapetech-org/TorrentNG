export type ServerMessageRecord = {
  id: string
  source: string
  parameters: { name: string }[]
  patterns: string[]
}

export type ServerMessageMatch = {
  id: string
  source: string
  tokens: Record<string, string>
}

type CompiledPattern = {
  id: string
  source: string
  names: string[]
  parts: string[]
  specificity: number
}

export type ServerMessageMatchers = {
  exact: Map<string, CompiledPattern>
  templates: CompiledPattern[]
}

const MAX_MESSAGE_LENGTH = 8192

function compilePattern(pattern: string, parameters: string[]): Omit<CompiledPattern, 'id' | 'source'> {
  const parts: string[] = []
  const names: string[] = []
  let cursor = 0
  const tokens = new RegExp(
    parameters.length ? `\\{(${parameters.join('|')})\\}` : '(?!)',
    'g',
  )

  for (const match of pattern.matchAll(tokens)) {
    parts.push(pattern.slice(cursor, match.index))
    names.push(match[1])
    cursor = match.index + match[0].length
  }

  parts.push(pattern.slice(cursor))

  if (names.length !== parameters.length || new Set(names).size !== names.length) {
    throw new Error('Server-message pattern must contain every parameter once')
  }

  if (parts.slice(1, -1).some(part => !part)) {
    throw new Error('Adjacent server-message parameters are ambiguous')
  }

  if (!parts.some(part => /[A-Za-z]/.test(part))) {
    throw new Error('Server-message patterns require fixed application wording')
  }

  return { parts, names, specificity: parts.join('').length }
}

export function compileServerMessageMatchers(records: ServerMessageRecord[]): ServerMessageMatchers {
  const exact = new Map<string, CompiledPattern>()
  const templates: CompiledPattern[] = []
  const ids = new Set<string>()

  for (const record of records) {
    if (!record || typeof record.id !== 'string' || !record.id ||
        typeof record.source !== 'string' || !record.source ||
        !Array.isArray(record.parameters) || !Array.isArray(record.patterns) ||
        !record.patterns.length || ids.has(record.id)) {
      throw new Error('Invalid or duplicate server-message record')
    }

    ids.add(record.id)
    const parameters = record.parameters.map(parameter => parameter.name)

    if (parameters.some(name => !/^value\d+$/.test(name)) ||
        new Set(parameters).size !== parameters.length) {
      throw new Error('Invalid server-message parameters')
    }

    for (const pattern of record.patterns) {
      if (typeof pattern !== 'string' || !pattern) {
        throw new Error('Invalid server-message pattern')
      }

      const compiled: CompiledPattern = {
        ...compilePattern(pattern, parameters),
        id: record.id,
        source: record.source,
      }

      if (!parameters.length) {
        const previous = exact.get(pattern)
        if (previous && previous.id !== record.id) {
          throw new Error('Conflicting exact server-message patterns')
        }
        exact.set(pattern, compiled)
      } else {
        templates.push(compiled)
      }
    }
  }

  templates.sort((left, right) => right.specificity - left.specificity)
  return { exact, templates }
}

function matchTemplate(message: string, template: CompiledPattern): Record<string, string> | null {
  const { parts, names } = template

  if (!message.startsWith(parts[0]) || !message.endsWith(parts[parts.length - 1])) {
    return null
  }

  const tokens: Record<string, string> = {}
  let cursor = parts[0].length

  for (let index = 0; index < names.length; index++) {
    const nextPart = parts[index + 1]
    const last = index === names.length - 1
    const boundary = last ? message.length - nextPart.length : message.indexOf(nextPart, cursor)

    if (boundary < cursor) {
      return null
    }

    // Repeated interior delimiters make token boundaries ambiguous.
    if (!last && message.indexOf(nextPart, boundary + nextPart.length) !== -1) {
      return null
    }

    tokens[names[index]] = message.slice(cursor, boundary)
    cursor = boundary + nextPart.length
  }

  return cursor === message.length ? tokens : null
}

function sameTokens(left: Record<string, string>, right: Record<string, string>): boolean {
  const leftKeys = Object.keys(left)
  return leftKeys.length === Object.keys(right).length &&
    leftKeys.every(key => left[key] === right[key])
}

export function matchServerMessage(message: unknown, matchers: ServerMessageMatchers): ServerMessageMatch | null {
  if (typeof message !== 'string' || !message || message.length > MAX_MESSAGE_LENGTH) {
    return null
  }

  const exact = matchers.exact.get(message)
  if (exact) {
    return { id: exact.id, source: exact.source, tokens: {} }
  }

  let result: ServerMessageMatch | null = null
  let specificity = -1

  for (const template of matchers.templates) {
    if (template.specificity < specificity) {
      break
    }

    const tokens = matchTemplate(message, template)
    if (!tokens) {
      continue
    }

    if (result && (result.id !== template.id || !sameTokens(result.tokens, tokens))) {
      return null
    }

    result = { id: template.id, source: template.source, tokens }
    specificity = template.specificity
  }

  return result
}
