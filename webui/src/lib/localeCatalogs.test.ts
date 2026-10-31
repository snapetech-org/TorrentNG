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
const categoriesStorageTrackerSummaryMessage = 'Categories, storage roots, and tracker summaries'
const filesystemHoleCheckMessage = [
  'Ask the filesystem which parts of each file were never written (holes, or preallocated space with no data) and re-verify any piece that claims to be there.',
  'Reads metadata only, so it is fast. It only triggers re-hashing; it never declares a piece corrupt by itself.',
].join(' ')
const immediateTorrentChangesMessage = 'Changes take effect immediately, for torrents that start or finish from now on.'
const weakUnknownStorageMessage = 'Check harder on weak or unknown storage'
const publicWebUiSetupMessage = 'Choose a WebUI username and password. Public binds require a password of at least 16 characters. API tokens stay enabled for automation and can also be entered in either login field.'
const chooseOrDropTorrentFilesMessage = 'Choose or drop torrent files'
const clearCategoryMessage = 'Clear category'
const chooseDockedDetailsPanelMessage = 'Choose where the details panel is docked'
const classifyEachSaveLocationMessage = 'Classify each save location'
const cleanShutdownMessage = 'Clean shutdown'
const closeEscapeActionsMessage = 'Close dialog, details, or clear selection'
const liveDownloadRateMessage = 'Collecting a live download rate'
const commandIndexUnavailableMessage = 'Command index unavailable'
const communityReleaseDiscussionMessage = 'Community support and release discussion'
const configOverlayMessage = 'Config overlay'
const copiedMessage = 'Copied'
const copyMessage = 'Copy'
const copySettingCurrentValueMessage = 'Copy {setting} current value'
const copyChangeSummaryMessage = 'Copy change summary'
const copyCommandMessage = 'Copy command'
const copyNameMessage = 'Copy name'
const copyVisibleCommandsMessage = 'Copy visible commands'
const fullRecheckCostMessage = 'Cost: Full recheck reads the whole library and is slow at scale.'
const sampleFullIoCostMessage = 'Cost: Sample re-reads a few percent; Full doubles the read I/O of a download.'
const sessionFileWriteCostMessage = 'Cost: a tiny file in the session folder, rewritten once a minute.'
const couldNotCheckFolderMessage = 'Could not check that folder:'
const couldNotReachApiMessage = 'Could not reach the API.'
const couldNotRestoreCredentialsMessage = 'Could not restore credentials from config.toml.'
const couldNotSaveLoginCredentialsMessage = 'Could not save login credentials.'
const credentialHiddenMessage = 'Credential hidden — select Show to reveal'
const createRuleBeforeApplyMessage = 'Create a rule above, then use match test to check titles before applying.'
const createWorkflowBeforeRunMessage = 'Create a workflow above, then preview it against the current torrent list before running.'
const credentialsRestoredMessage = 'Credentials restored from config.toml.'
const currentUsernameMessage = 'Current username:'
const customLinesMessage = 'Custom lines'
const customLinesCountMessage = 'Custom lines ({count})'
const customTorrentConfigLinesMessage = 'Custom rTorrent configuration lines'
const customTorrentLinesMessage = 'Custom rTorrent lines'
const customTorrentLinesEditedMessage = 'Custom rTorrent lines edited'
const dhtPeerDiscoveryControlsMessage = 'DHT, PEX, and UDP tracker discovery controls.'
const defaultActionMessage = 'Default'
const defaultsActionMessage = 'Defaults'
const deleteSelectedTorrentConfirmationMessage = 'Delete the selected torrent (with confirmation)'
const descendingMessage = 'Descending'
const detailsPanelPositionMessage = 'Details panel position'
const failedTorrentActionMessage = 'Failed to {action} torrent'

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
    expect(hi[immediateTorrentChangesMessage]).toContain('अब से शुरू या पूरे होने वाले टोरेंट')
    expect(ar[immediateTorrentChangesMessage]).toContain('التورنتات التي تبدأ أو تكتمل')
    expect(ar[weakUnknownStorageMessage]).toContain('شدّد الفحص')
    expect(id[weakUnknownStorageMessage]).toContain('pemeriksaan lebih ketat')
    expect(zhHans[publicWebUiSetupMessage]).toContain('绑定到公网地址时')
    expect(de[publicWebUiSetupMessage]).toContain('in eines der beiden Anmeldefelder')
    expect(ja[publicWebUiSetupMessage]).toContain('自動化用に引き続き有効')
    expect(fr[publicWebUiSetupMessage]).toContain('Une écoute sur une adresse publique')
    expect(ko[publicWebUiSetupMessage]).not.toContain('Public binds')
    expect(ptBR[publicWebUiSetupMessage]).toContain('qualquer um dos dois campos')
    expect(es[chooseOrDropTorrentFilesMessage]).toContain('arrastra y suelta')
    expect(hi[chooseOrDropTorrentFilesMessage]).toContain('खींचकर छोड़ें')
    expect(ja[chooseOrDropTorrentFilesMessage]).toContain('ドラッグ＆ドロップ')
    expect(ko[chooseOrDropTorrentFilesMessage]).toContain('끌어다 놓으세요')
    expect(id[chooseOrDropTorrentFilesMessage]).toContain('seret dan lepas')
    expect(hi[chooseDockedDetailsPanelMessage]).toBe('विवरण पैनल कहाँ रखना है, चुनें')
    expect(id[chooseDockedDetailsPanelMessage]).toBe('Pilih di mana panel detail ditempatkan')
    expect(hi[classifyEachSaveLocationMessage]).toContain('सहेजने की हर जगह')
    expect(hi[cleanShutdownMessage]).toBe('सामान्य शटडाउन')
    expect(ar[cleanShutdownMessage]).toBe('إغلاق سليم')
    expect(hi[liveDownloadRateMessage]).toBe('वर्तमान डाउनलोड गति मापी जा रही है')
    expect(id.Clear).toBe('Bersihkan')
    expect(de[clearCategoryMessage]).toBe('Keine Kategorie')
    expect(fr[clearCategoryMessage]).toBe('Aucune catégorie')
    expect(id[clearCategoryMessage]).toBe('Tanpa kategori')
    expect(vi.Categories).toBe('Danh mục')
    expect(vi[clearCategoryMessage]).toBe('Bỏ danh mục')
    expect(zhHans.Columns).toBe('列')
    expect(ja.Columns).toBe('列')
    expect(hi[commandIndexUnavailableMessage]).toBe('कमांड इंडेक्स उपलब्ध नहीं है')
    expect(ko[commandIndexUnavailableMessage]).not.toContain('unavailable')
    expect(hi[communityReleaseDiscussionMessage]).toContain('रिलीज़ पर चर्चा')
    expect(vi[configOverlayMessage]).toBe('Lớp phủ cấu hình')
    expect(ar[copiedMessage]).toBe('تم النسخ')
    expect(ar[copyMessage]).toBe('انسخ')
    expect(hi[copyMessage]).toBe('कॉपी करें')
    expect(de[copyMessage]).toBe('Kopieren')
    expect(ptBR[copyMessage]).toBe('Copiar')
    expect(id[copyMessage]).toBe('Salin')
    expect(ar[copyChangeSummaryMessage]).toBe('نسخ ملخص التغييرات')
    expect(hi[copyChangeSummaryMessage]).toBe('बदलावों का सारांश कॉपी करें')
    expect(ptBR[copyChangeSummaryMessage]).toBe('Copiar resumo das alterações')
    expect(de[copyCommandMessage]).toBe('Befehl kopieren')
    expect(ja[copyCommandMessage]).toBe('コマンドをコピー')
    expect(hi[copyCommandMessage]).toBe('कमांड कॉपी करें')
    expect(ko[copyCommandMessage]).toBe('명령 복사')
    expect(hi[copyNameMessage]).toBe('नाम कॉपी करें')
    expect(ja[copyNameMessage]).toBe('名前をコピー')
    expect(ko[copyNameMessage]).toBe('이름 복사')
    expect(ar[copyVisibleCommandsMessage]).toBe('نسخ الأوامر الظاهرة')
    expect(ja[copyVisibleCommandsMessage]).toBe('表示されているコマンドをコピー')
    expect(de[copyVisibleCommandsMessage]).toBe('Sichtbare Befehle kopieren')
    expect(ar[copySettingCurrentValueMessage]).toContain('الحالية لـ {setting}')
    expect(hi[copySettingCurrentValueMessage]).toContain('{setting} का वर्तमान मान')
    expect(es[fullRecheckCostMessage]).toContain('bibliotecas grandes')
    expect(hi[fullRecheckCostMessage]).toContain('पूर्ण पुनःजाँच')
    expect(ptBR[fullRecheckCostMessage]).toContain('bibliotecas grandes')
    expect(zhHans[sampleFullIoCostMessage]).not.toContain('Full')
    expect(id[sampleFullIoCostMessage]).toContain('pemeriksaan penuh')
    expect(ar[sessionFileWriteCostMessage]).toContain('يُعاد كتابته')
    expect(ar[couldNotCheckFolderMessage]).toContain('المجلد')
    expect(es[couldNotReachApiMessage]).toBe('No se pudo acceder a la API.')
    expect(zhHans[couldNotReachApiMessage]).not.toContain(' 。')
    expect(ptBR[couldNotReachApiMessage]).toContain('Não foi possível acessar')
    expect(ko[couldNotReachApiMessage]).toContain('연결할 수 없습니다')
    expect(id[couldNotReachApiMessage]).toContain('terhubung ke API')
    expect(zhHans[couldNotRestoreCredentialsMessage]).toContain('恢复凭据')
    expect(ar[couldNotRestoreCredentialsMessage]).toContain('بيانات الاعتماد')
    expect(ko[couldNotRestoreCredentialsMessage]).toContain('자격 증명')
    expect(es[couldNotSaveLoginCredentialsMessage]).toContain('No se pudieron guardar')
    expect(hi[couldNotSaveLoginCredentialsMessage]).toContain('सहेजे नहीं जा सके')
    expect(ar[createRuleBeforeApplyMessage]).toContain('قبل تطبيقها')
    expect(hi[createWorkflowBeforeRunMessage]).toContain('सूची से मिलाकर')
    expect(zhHans[credentialHiddenMessage]).toContain('选择“显示”即可查看')
    expect(zhHant[credentialHiddenMessage]).toContain('登入資訊已隱藏')
    expect(fr[credentialHiddenMessage]).toContain('Identifiants masqués')
    expect(ko[credentialHiddenMessage]).toContain('숨겨져 있습니다')
    expect(vi[credentialHiddenMessage]).toContain('đang ẩn')
    expect(id[credentialHiddenMessage]).toContain('untuk melihatnya')
    expect(ko[credentialsRestoredMessage]).not.toContain('Credentials')
    expect(zhHans[currentUsernameMessage]).not.toContain(' :')
    expect(zhHans[customLinesMessage]).toContain('配置行')
    expect(zhHans[customLinesCountMessage]).toContain('配置行')
    expect(ja[customLinesMessage]).toBe('カスタム設定行')
    expect(id[customLinesMessage]).toBe('Baris konfigurasi khusus')
    expect(hi[customTorrentConfigLinesMessage]).toContain('rTorrent की कस्टम कॉन्फ़िगरेशन लाइनें')
    expect(ptBR[customTorrentConfigLinesMessage]).toContain('do rTorrent')
    expect(ja[customTorrentLinesMessage]).toBe('カスタム rTorrent 設定行')
    expect(ko[customTorrentLinesEditedMessage]).toContain('구성 줄을 편집했습니다')
    expect(de[dhtPeerDiscoveryControlsMessage]).toContain('Peer-Erkennung')
    expect(fr[dhtPeerDiscoveryControlsMessage]).toContain('découverte des pairs')
    expect(ptBR[dhtPeerDiscoveryControlsMessage]).toContain('descoberta de peers')
    expect(ar[dhtPeerDiscoveryControlsMessage]).toContain('اكتشاف النظراء')
    expect(zhHans[dhtPeerDiscoveryControlsMessage]).toContain('对等端发现')
    expect(ar[defaultActionMessage]).toBe('افتراضي')
    expect(fr[defaultActionMessage]).toBe('Par défaut')
    expect(id[defaultActionMessage]).toBe('Bawaan')
    expect(hi[defaultsActionMessage]).toBe('डिफ़ॉल्ट लागू करें')
    expect(id[defaultsActionMessage]).toBe('Terapkan nilai bawaan')
    expect(ar[deleteSelectedTorrentConfirmationMessage]).toContain('التورنت المحدد')
    expect(zhHans[deleteSelectedTorrentConfirmationMessage]).not.toContain('torrent')
    expect(ja[deleteSelectedTorrentConfirmationMessage]).not.toContain('torrent')
    expect(fr[descendingMessage]).toBe('Décroissant')
    expect(zhHans[detailsPanelPositionMessage]).toBe('详情面板位置')
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
    expect(zhHans[categoriesStorageTrackerSummaryMessage]).toContain('追踪器摘要')
    expect(zhHans[categoriesStorageTrackerSummaryMessage]).not.toContain('tracker')
    expect(fr[categoriesStorageTrackerSummaryMessage]).toContain('répertoires racines')
    expect(ja[categoriesStorageTrackerSummaryMessage]).toContain('ルートディレクトリ')
    expect(id[categoriesStorageTrackerSummaryMessage]).toContain('direktori akar')
    expect(hi['Category for selected torrents']).toBe('चयनित टोरेंट के लिए श्रेणी')
    expect(de['Category path:']).toBe('Kategoriepfad:')

    for (const [locale, catalog] of Object.entries(catalogs)) {
      expect(catalog[crashSafetyPathMessage], `${locale}: settings label`).toContain(catalog.Settings)
      expect(catalog[crashSafetyPathMessage], `${locale}: backend label`).toContain(catalog.Backend)
      expect(catalog[crashSafetyPathMessage], `${locale}: crash safety label`).toContain(catalog['Crash safety'])
    }

    const keyboardHelp = 'Click a row to select it. Hold Control or Command to add or remove rows. Hold Shift to select a range. Use Arrow keys to move and Space to select.'
    expect(es[closeEscapeActionsMessage]).toContain('panel de detalles')
    expect(hi[closeEscapeActionsMessage]).toContain('विवरण पैनल')
    expect(ja[closeEscapeActionsMessage]).toContain('詳細パネル')
    expect(vi[closeEscapeActionsMessage]).toContain('bảng chi tiết')
    expect(id[closeEscapeActionsMessage]).toContain('batalkan pilihan')
    expect(hi[keyboardHelp]).not.toContain('Arrow keys')
    expect(hi[keyboardHelp]).toContain('स्पेसबार')
    expect(ja[keyboardHelp]).not.toContain('Space キー')
    expect(ja[keyboardHelp]).toContain('スペースキー')
    for (const catalog of Object.values(catalogs)) {
      const translation = catalog[keyboardHelp]
      expect(translation).not.toContain('Hold Shift')
      expect(translation).not.toContain('Grab Shift')
      expect(translation).not.toContain('space to select')
    }
  })

  it('keeps reviewed action, status, and dynamic failure wording in context', () => {
    expect(ar['Different settings for specific folders']).toBe('إعدادات مختلفة لمجلدات محددة')
    expect(de['Detect host crashes']).toBe('Host-Abstürze erkennen')
    expect(de.Disk).toBe('Speicherplatz')
    expect(id['Discord support']).toBe('Bantuan di Discord')
    expect(ko['Dry run']).toBe('모의 실행')
    expect(vi['Dry run']).toBe('Chạy thử')
    expect(ko.Exclude).toBe('제외')
    expect(ja.Expand).toBe('展開する')
    expect(hi.Expand).toBe('खोलें')
    expect(ko['File details unavailable.']).toBe('파일 정보를 사용할 수 없습니다.')
    expect(ko.Folder).toBe('폴더')
    expect(id['Files ({count})']).toBe('Berkas ({count})')
    expect(fr.Free).toBe('Disponible')
    expect(zhHans.Free).toBe('可用')
    expect(ar.Free).toBe('المساحة المتاحة')
    expect(de.Go).toBe('Filtern')
    expect(ja.Go).toBe('絞り込む')
    expect(zhHans['Filter trackers']).toBe('筛选追踪器')
    expect(vi['Filter commands']).toBe('Lọc lệnh')
    expect(es.Hash).toBe('Hash')
    expect(ko.Hash).toBe('해시')
    expect(zhHant['Global download slots']).toBe('全域同時下載數上限')
    expect(ko['Loading authentication settings…']).toBe('인증 설정을 불러오는 중…')
    expect(vi['Keeps a power cut or crash from marking half-written files as complete.']).toContain('chỉ mới được ghi một phần')
    expect(ko.Lean).toBe('경량')
    expect(ar.Left).toBe('يسار')
    expect(zhHans.Manual).toBe('手动')
    expect(de.Manual).toBe('Manuell')
    expect(ja.Manual).toBe('手動')
    expect(id.Manual).toBe('Manual')
    expect(hi['Log kind']).toBe('लॉग का प्रकार')
    expect(ar['Log kind']).toBe('نوع السجل')
    expect(es['Logs unavailable']).toBe('No se pueden consultar los registros')
    expect(zhHans['Logs unavailable']).toBe('日志不可用')
    expect(ko['Filter commands']).toBe('명령 필터링')
    expect(zhHans['Global']).toBe('全局')
    expect(vi['Global']).toBe('Toàn cục')
    expect(ar[failedTorrentActionMessage]).toBe('تعذّر تنفيذ الإجراء «{action}» على التورنت')
    expect(zhHans[failedTorrentActionMessage]).toBe('无法对种子执行“{action}”操作')
    expect(ptBR[failedTorrentActionMessage]).toBe('Não foi possível executar a ação “{action}” no torrent')
  })
})
