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

const catalogs = { es, zhHans, hi, de, ja, fr, ptBR, ar, ko, zhHant, vi, id }
const referenceKeys = Object.keys(es).sort()

function placeholders(value: string) {
  return [...value.matchAll(/\{([a-zA-Z][a-zA-Z0-9_]*)\}/g)]
    .map(match => match[1])
    .sort()
}

describe('interface locale catalogs', () => {
  it('keeps all twelve catalogs in parity with the source-message catalog', () => {
    expect(referenceKeys).toHaveLength(1142)
    for (const [locale, catalog] of Object.entries(catalogs)) {
      expect(Object.keys(catalog).sort(), locale).toEqual(referenceKeys)
    }
  })

  it('keeps every message non-empty and preserves named placeholders', () => {
    for (const [locale, catalog] of Object.entries(catalogs)) {
      for (const [source, translation] of Object.entries(catalog)) {
        expect(translation.trim(), `${locale}: ${source}`).not.toBe('')
        expect(placeholders(translation), `${locale}: ${source}`).toEqual(placeholders(source))
      }
    }
  })
})
