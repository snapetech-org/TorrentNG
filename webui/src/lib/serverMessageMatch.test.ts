import { describe, expect, it } from 'vitest'
import serverMessages from './server-messages.json'
import {
  compileServerMessageMatchers,
  matchServerMessage,
  type ServerMessageRecord,
} from './serverMessageMatch'
import { SERVER_EVENT_KIND_LABELS } from './serverEventKinds'
import { UI_LABELS as existingCatalog } from './locale-catalogs/es'

function record(id: string, source: string, parameters: string[] = [], patterns = [source]): ServerMessageRecord {
  return {
    id,
    source,
    parameters: parameters.map(name => ({ name })),
    patterns,
  }
}

describe('server message matching', () => {
  it('matches fixed messages only as complete strings', () => {
    const matchers = compileServerMessageMatchers([record('fixed', 'A category is recommended')])

    expect(matchServerMessage('A category is recommended', matchers)).toEqual({
      id: 'fixed', source: 'A category is recommended', tokens: {},
    })
    expect(matchServerMessage('prefix A category is recommended', matchers)).toBeNull()
    expect(matchServerMessage('A category is recommended suffix', matchers)).toBeNull()
  })

  it('preserves captured values byte for byte', () => {
    const matchers = compileServerMessageMatchers([
      record('size', 'Received {value0} bytes; maximum is {value1}', ['value0', 'value1']),
    ])
    const raw = '0012 / عربى / {protocol-token}'

    expect(matchServerMessage(`Received ${raw} bytes; maximum is 8`, matchers)).toEqual({
      id: 'size',
      source: 'Received {value0} bytes; maximum is {value1}',
      tokens: { value0: raw, value1: '8' },
    })
  })

  it('prefers the most specific fixed wording', () => {
    const matchers = compileServerMessageMatchers([
      record('broad', 'Unable to connect: {value0}', ['value0']),
      record('specific', 'Unable to connect: import list {value0}', ['value0']),
    ])

    expect(matchServerMessage('Unable to connect: import list external', matchers)?.id).toBe('specific')
  })

  it('leaves equally specific collisions unchanged', () => {
    const matchers = compileServerMessageMatchers([
      record('left', 'A{value0}B', ['value0']),
      record('right', 'A{value0}B', ['value0']),
    ])

    expect(matchServerMessage('AexternalB', matchers)).toBeNull()
  })

  it('rejects repeated interior delimiters with ambiguous captures', () => {
    const matchers = compileServerMessageMatchers([
      record('two', "From '{value0}' to '{value1}'", ['value0', 'value1']),
    ])

    expect(matchServerMessage("From 'a' to 'b' to 'c'", matchers)).toBeNull()
    expect(matchServerMessage("From 'a' to 'b'", matchers)?.tokens).toEqual({ value0: 'a', value1: 'b' })
  })

  it('treats undeclared protocol braces as literal text', () => {
    const source = 'Only a literal hc:{id} protocol token is supported'
    const matchers = compileServerMessageMatchers([record('literal', source)])

    expect(matchServerMessage(source, matchers)?.id).toBe('literal')
    expect(matchServerMessage(source.replace('{id}', '123'), matchers)).toBeNull()
  })

  it('rejects invalid and oversized input', () => {
    const matchers = compileServerMessageMatchers([record('fixed', 'A fixed source message')])

    for (const value of [undefined, null, 1, {}, [], '']) {
      expect(matchServerMessage(value, matchers)).toBeNull()
    }
    expect(matchServerMessage(`Error: ${'a'.repeat(8192)}`, matchers)).toBeNull()
  })

  it('refuses missing, repeated, unsafe, or adjacent token declarations', () => {
    expect(() => compileServerMessageMatchers([
      record('missing', 'Message {value0}', ['value0', 'value1']),
    ])).toThrow(/every parameter once/)
    expect(() => compileServerMessageMatchers([
      record('repeated', 'Message {value0} {value0}', ['value0']),
    ])).toThrow(/every parameter once/)
    expect(() => compileServerMessageMatchers([
      record('unsafe', 'Message {value0}', ['value0|.*']),
    ])).toThrow(/Invalid server-message parameters/)
    expect(() => compileServerMessageMatchers([
      record('adjacent', 'Message {value0}{value1}', ['value0', 'value1']),
    ])).toThrow(/Adjacent/)
    expect(() => compileServerMessageMatchers([
      record('dynamic', '{value0}: {value1}', ['value0', 'value1']),
    ])).toThrow(/fixed application wording/)
  })

  it('rejects duplicate IDs and conflicting exact patterns', () => {
    expect(() => compileServerMessageMatchers([
      record('same', 'First'), record('same', 'Second'),
    ])).toThrow(/duplicate/)
    expect(() => compileServerMessageMatchers([
      record('first', 'Same'), record('second', 'Same'),
    ])).toThrow(/Conflicting/)
  })

  it('round-trips every reviewed server source pattern without changing dynamic values', () => {
    const matchers = compileServerMessageMatchers(serverMessages)

    for (const sourceRecord of serverMessages) {
      for (const pattern of sourceRecord.patterns) {
        const expectedTokens: Record<string, string> = {}
        const message = pattern.replace(/\{(value\d+)\}/g, (_placeholder, name: string) => {
          const value = `dynamic-${sourceRecord.id}-${name}`
          expectedTokens[name] = value
          return value
        })
        expect(matchServerMessage(message, matchers), sourceRecord.id).toEqual({
          id: sourceRecord.id,
          source: sourceRecord.source,
          tokens: expectedTokens,
        })
      }
    }
  })

  it('maps every persisted event kind to an existing or reviewed catalog source', () => {
    const sources = new Set([
      ...Object.keys(existingCatalog),
      ...serverMessages.map(sourceRecord => sourceRecord.source),
    ])

    expect(Object.keys(SERVER_EVENT_KIND_LABELS)).toHaveLength(23)
    for (const source of Object.values(SERVER_EVENT_KIND_LABELS)) {
      expect(sources.has(source), source).toBe(true)
    }
  })
})
