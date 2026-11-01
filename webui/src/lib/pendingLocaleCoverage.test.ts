import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { UI_LABELS as ar } from './locale-catalogs/ar'
import { UI_LABELS as de } from './locale-catalogs/de'
import { UI_LABELS as es } from './locale-catalogs/es'
import { UI_LABELS as fr } from './locale-catalogs/fr'
import { UI_LABELS as hi } from './locale-catalogs/hi'
import { UI_LABELS as id } from './locale-catalogs/id'
import { UI_LABELS as ja } from './locale-catalogs/ja'
import { UI_LABELS as ko } from './locale-catalogs/ko'
import { UI_LABELS as ptBR } from './locale-catalogs/pt-BR'
import { UI_LABELS as vi } from './locale-catalogs/vi'
import { UI_LABELS as zhHans } from './locale-catalogs/zh-Hans'
import { UI_LABELS as zhHant } from './locale-catalogs/zh-Hant'
import {
  PENDING_SERVER_MESSAGE_SOURCE_IDS,
  PENDING_UI_LOCALE_SOURCES,
  SERVER_MESSAGE_SOURCE_CALLER_LEDGER,
} from './pendingLocaleCoverage'

const catalogs = [es, zhHans, hi, de, ja, fr, ptBR, ar, ko, zhHant, vi, id]

describe('pending locale coverage ledger', () => {
  it('tracks 90 source-owned UI strings without filler translations', () => {
    const sources = PENDING_UI_LOCALE_SOURCES.map(entry => entry.source)
    expect(sources).toHaveLength(90)
    expect(new Set(sources).size).toBe(90)
    const callerSources = new Map<string, string>()
    for (const entry of PENDING_UI_LOCALE_SOURCES) {
      const callers = 'callers' in entry ? entry.callers : [entry.caller]
      expect(callers.length).toBeGreaterThan(0)
      for (const callerPath of callers) {
        const caller = callerSources.get(callerPath) ?? readFileSync(resolve(process.cwd(), '..', callerPath), 'utf8')
        callerSources.set(callerPath, caller)
        expect(caller, callerPath).toContain(entry.source)
      }
    }
    for (const catalog of catalogs) {
      for (const source of sources) expect(Object.hasOwn(catalog, source)).toBe(false)
    }
  })

  it('records the producer and render caller for all 195 server sources', () => {
    expect(SERVER_MESSAGE_SOURCE_CALLER_LEDGER).toHaveLength(195)
    expect(new Set(SERVER_MESSAGE_SOURCE_CALLER_LEDGER.map(entry => entry.id)).size).toBe(195)

    const pending = SERVER_MESSAGE_SOURCE_CALLER_LEDGER.filter(entry => entry.status === 'pending')
    const covered = SERVER_MESSAGE_SOURCE_CALLER_LEDGER.filter(entry => entry.status === 'covered')
    expect(pending).toHaveLength(191)
    expect(PENDING_SERVER_MESSAGE_SOURCE_IDS).toHaveLength(191)
    expect(covered.map(entry => entry.source).sort()).toEqual([
      'Tags updated',
      'Torrent added',
      'Torrent removed',
      'Torrent updated',
    ])
    expect(pending.every(entry => entry.producerPaths.length > 0 && entry.callerPaths.length === 2)).toBe(true)

    for (const catalog of catalogs) {
      for (const entry of pending) expect(Object.hasOwn(catalog, entry.source)).toBe(false)
      for (const entry of covered) expect(Object.hasOwn(catalog, entry.source)).toBe(true)
    }
    expect(pending.length * catalogs.length).toBe(2_292)
  })
})
