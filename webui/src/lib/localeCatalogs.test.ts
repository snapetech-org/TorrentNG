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
const restoreApiTokensMessage = [
  "API tokens are configured in {tokenConfig} or {tokenFileConfig}. On Unraid, find the token in the container's {unraidSetting} setting.",
  'The existing-client profile also accepts {tngApiTokens} or {rtngApiTokens}. Runtime credential changes are saved in the service state directory.',
  'Select Restore to load values from {configFile} again.',
].join(' ')
const crashSafetyPathMessage = [
  'A torrent shown as Finalizing has downloaded every piece but is being held until its data is confirmed on disk, so a power cut cannot leave a half-written file marked complete for Sonarr, Radarr or a tracker.',
  'It normally clears within a second. If it stays, storage cannot be synced; see Settings, Backend, Crash safety.',
].join(' ')
const advancedOverridesMessage = 'Advanced overrides are imported after managed settings and usually need a restart.'
const weakMountChecksMessage = 'After a power loss, Weak mounts are fully rechecked and Unknown mounts get at least the recent-writes check. Weak mounts also get a sampled read-back after every download.'
const appliesToTorrentsMessage = 'Applies to torrents saved in this folder or anywhere below it.'
const applyDefaultsCountMessage = 'Apply defaults to {count} settings in this group? Autosave will apply shortly.'
const applyOneErrorMessage = 'Apply: {matched} matched, {applied} applied, {count} error'
const locationPoliciesLimitMessage = 'At most {count} location policies are allowed.'
const cacheLimitsMessage = 'Cache, file/socket ceilings, hashing, and session persistence.'
const filesystemHoleCheckMessage = [
  'Ask the filesystem which parts of each file were never written (holes, or preallocated space with no data) and re-verify any piece that claims to be there.',
  'Reads metadata only, so it is fast. It only triggers re-hashing; it never declares a piece corrupt by itself.',
].join(' ')

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

  it('keeps torrent state, action, glossary, and keyboard-help translations in context', () => {
    expect(es.Checking).toBe('Comprobando')
    expect(es['Check…']).toBe('Comprobando…')
    expect(es.Command).toBe('Comando')
    expect(es['Affected torrents']).toBe('Torrents afectados')
    expect(ja['Check…']).toBe('チェック中…')
    expect(ko['Checking…']).toBe('확인 중…')
    expect(fr['Affected torrents']).toBe('Torrents concernés')
    expect(zhHans['Check']).toBe('检查')
    expect(vi.Command).toBe('Lệnh')
    expect(hi.Command).toBe('कमांड')
    expect(de['Add a torrent to populate the table.']).toContain('die Tabelle')
    expect(ar['Add folder policy']).toBe('إضافة سياسة للمجلد')
    expect(ar['Add or remove a torrent from the selection']).toContain('إضافة تورنت')
    expect(ja['Add more torrent files']).toBe('トレントファイルをさらに追加')
    expect(fr['API-token login:']).toBe('Connexion avec un jeton API :')
    expect(hi[restoreApiTokensMessage]).toContain('“पुनर्स्थापित करें”')
    expect(id['. Live-safe controls are applied immediately; controls marked restart are saved for the next daemon start.']).toContain('saat layanan berjalan')
    expect(id.Backend).toBe('Backend')
    expect(ko['Crash safety']).toBe('시스템 충돌 보호')
    expect(ja.Advanced).toBe('高度な設定')
    expect(de[advancedOverridesMessage]).toContain('Überschreibungen')
    expect(fr[advancedOverridesMessage]).toContain('paramètres de remplacement avancés')
    expect(fr['Adding…']).toBe('Ajout en cours…')
    expect(es[weakMountChecksMessage]).toContain('puntos de montaje débiles')
    expect(zhHans[weakMountChecksMessage]).toContain('弱挂载上的数据')
    expect(hi[weakMountChecksMessage]).toContain('हाल की लिखाइयों')
    expect(ar[weakMountChecksMessage]).toContain('نقاط التحميل')
    expect(ptBR[weakMountChecksMessage]).toContain('pontos de montagem')
    expect(ko[weakMountChecksMessage]).toContain('최근 쓰기')
    expect(id[weakMountChecksMessage]).toContain('titik mount')
    expect(es[appliesToTorrentsMessage]).toContain('torrents guardados')
    expect(ptBR[appliesToTorrentsMessage]).toContain('torrents salvos')
    expect(de[appliesToTorrentsMessage]).toContain('diesem Ordner')
    expect(zhHans[appliesToTorrentsMessage]).not.toContain(' 。')
    expect(ar[appliesToTorrentsMessage]).not.toContain('。')
    expect(ar['Apply category']).toBe('تطبيق الفئة')
    expect(ko['Apply category']).toBe('카테고리 적용')
    expect(de[applyDefaultsCountMessage]).toContain('{count} Einstellungen')
    expect(fr[applyDefaultsCountMessage]).toContain('aux {count} paramètres')
    expect(vi[applyDefaultsCountMessage]).toContain('cho {count} cài đặt')
    expect(es[applyOneErrorMessage]).toContain('{matched} coincidencias')
    expect(fr[applyOneErrorMessage]).toContain('{matched} correspondances')
    expect(ptBR[applyOneErrorMessage]).toContain('{matched} correspondências')
    expect(de[locationPoliciesLimitMessage]).toContain('höchstens {count} Standortregeln')
    expect(ar[locationPoliciesLimitMessage]).toContain('بحد أقصى {count}')
    expect(zhHans['Authentication settings are unavailable.']).not.toContain(' 。')
    expect(hi['Autosaves now; effective after restart']).not.toContain('Autosaves')
    expect(ko[filesystemHoleCheckMessage]).toContain('손상된 것으로')
    expect(id[filesystemHoleCheckMessage]).toContain('suatu bagian rusak')
    expect(ar[filesystemHoleCheckMessage]).toContain('لا يحكم بمفرده')
    expect(ja['Bulk torrent actions']).toBe('トレントの一括操作')
    expect(vi['CA path']).toBe('Đường dẫn CA')
    expect(id['CA path']).toBe('Jalur CA')
    expect(ptBR[cacheLimitsMessage]).toContain('cálculo de hashes')
    expect(id[cacheLimitsMessage]).toContain('penghitungan hash')

    for (const [locale, catalog] of Object.entries(catalogs)) {
      expect(catalog[crashSafetyPathMessage], `${locale}: settings label`).toContain(catalog.Settings)
      expect(catalog[crashSafetyPathMessage], `${locale}: backend label`).toContain(catalog.Backend)
      expect(catalog[crashSafetyPathMessage], `${locale}: crash safety label`).toContain(catalog['Crash safety'])
    }

    const keyboardHelp = 'Click a row to select it. Hold Control or Command to add or remove rows. Hold Shift to select a range. Use Arrow keys to move and Space to select.'
    for (const catalog of Object.values(catalogs)) {
      const translation = catalog[keyboardHelp]
      expect(translation).not.toContain('Hold Shift')
      expect(translation).not.toContain('Grab Shift')
      expect(translation).not.toContain('space to select')
    }
  })
})
