import { useSyncExternalStore } from 'react'

export const INTERFACE_LANGUAGE_OPTIONS = [
  { value: 'en', label: 'English' },
  { value: 'es', label: 'Español — Spanish' },
  { value: 'zh-Hans', label: '简体中文 — Simplified Chinese' },
  { value: 'hi', label: 'हिन्दी — Hindi' },
  { value: 'de', label: 'Deutsch — German' },
  { value: 'ja', label: '日本語 — Japanese' },
  { value: 'fr', label: 'Français — French' },
  { value: 'pt-BR', label: 'Português (Brasil) — Brazilian Portuguese' },
  { value: 'ar', label: 'العربية — Arabic' },
  { value: 'ko', label: '한국어 — Korean' },
  { value: 'zh-Hant', label: '繁體中文 — Traditional Chinese' },
  { value: 'vi', label: 'Tiếng Việt — Vietnamese' },
  { value: 'id', label: 'Bahasa Indonesia — Indonesian' },
] as const

export type InterfaceLanguage = (typeof INTERFACE_LANGUAGE_OPTIONS)[number]['value']
type TranslatedLanguage = Exclude<InterfaceLanguage, 'en'>
type LocaleCatalog = Record<string, string>

export function selectedTorrentCountSource(count: number) {
  return count === 1 ? '{count} selected torrent' : '{count} selected torrents'
}

const STORAGE_KEY = 'torrentng.interface-language'
const listeners = new Set<() => void>()
const catalogCache = new Map<TranslatedLanguage, LocaleCatalog>()
const pendingCatalogs = new Map<TranslatedLanguage, Promise<LocaleCatalog>>()
const catalogLoaders: Record<TranslatedLanguage, () => Promise<LocaleCatalog>> = {
  es: () => import('./locale-catalogs/es').then(module => module.UI_LABELS),
  'zh-Hans': () => import('./locale-catalogs/zh-Hans').then(module => module.UI_LABELS),
  hi: () => import('./locale-catalogs/hi').then(module => module.UI_LABELS),
  de: () => import('./locale-catalogs/de').then(module => module.UI_LABELS),
  ja: () => import('./locale-catalogs/ja').then(module => module.UI_LABELS),
  fr: () => import('./locale-catalogs/fr').then(module => module.UI_LABELS),
  'pt-BR': () => import('./locale-catalogs/pt-BR').then(module => module.UI_LABELS),
  ar: () => import('./locale-catalogs/ar').then(module => module.UI_LABELS),
  ko: () => import('./locale-catalogs/ko').then(module => module.UI_LABELS),
  'zh-Hant': () => import('./locale-catalogs/zh-Hant').then(module => module.UI_LABELS),
  vi: () => import('./locale-catalogs/vi').then(module => module.UI_LABELS),
  id: () => import('./locale-catalogs/id').then(module => module.UI_LABELS),
}

type LocaleSnapshot = { language: InterfaceLanguage; revision: number }
const serverSnapshot: LocaleSnapshot = { language: 'en', revision: 0 }

function isInterfaceLanguage(value: string | null): value is InterfaceLanguage {
  return INTERFACE_LANGUAGE_OPTIONS.some(option => option.value === value)
}

function browserLanguage(): InterfaceLanguage {
  if (typeof navigator === 'undefined') return 'en'
  const candidates = navigator.languages?.length ? navigator.languages : [navigator.language]
  for (const candidate of candidates) {
    const normalized = candidate.toLowerCase()
    const base = normalized.split(/[-_]/)[0]
    if (base === 'zh') return /(?:hant|tw|hk|mo)/.test(normalized) ? 'zh-Hant' : 'zh-Hans'
    if (base === 'pt') return 'pt-BR'
    if (base === 'es' || base === 'hi' || base === 'de' || base === 'ja' || base === 'fr' ||
        base === 'ar' || base === 'ko' || base === 'vi' || base === 'id') return base
  }
  return 'en'
}

function readLanguage(): InterfaceLanguage {
  try {
    const stored = typeof localStorage === 'undefined' ? null : localStorage.getItem(STORAGE_KEY)
    return isInterfaceLanguage(stored) ? stored : browserLanguage()
  } catch {
    return browserLanguage()
  }
}

let activeLanguage = readLanguage()
let currentSnapshot: LocaleSnapshot = { language: activeLanguage, revision: 0 }

function notify() {
  currentSnapshot = { language: activeLanguage, revision: currentSnapshot.revision + 1 }
  listeners.forEach(listener => listener())
}

function loadCatalog(language: InterfaceLanguage) {
  if (language === 'en' || catalogCache.has(language) || pendingCatalogs.has(language)) return

  const pending = catalogLoaders[language]()
  pendingCatalogs.set(language, pending)
  void pending.then(catalog => {
    pendingCatalogs.delete(language)
    catalogCache.set(language, catalog)
    if (activeLanguage === language) notify()
  }).catch(() => {
    pendingCatalogs.delete(language)
    if (activeLanguage === language) notify()
  })
}

function activateLanguage(language: InterfaceLanguage) {
  activeLanguage = language
  if (typeof document !== 'undefined') {
    document.documentElement.lang = language
    document.documentElement.dir = language === 'ar' ? 'rtl' : 'ltr'
  }
  loadCatalog(language)
  notify()
}

if (typeof window !== 'undefined') {
  window.addEventListener('storage', event => {
    if (event.key !== STORAGE_KEY && event.key !== null) return
    activateLanguage(readLanguage())
  })
}

export function applyInitialInterfaceLanguage() {
  activateLanguage(readLanguage())
}

function translatedValue(source: string, language: InterfaceLanguage) {
  if (language === 'en') return source
  return catalogCache.get(language)?.[source] ?? source
}

export function translateInterface(
  source: string,
  parameters: Record<string, string | number> = {},
  language: InterfaceLanguage = activeLanguage,
) {
  return translatedValue(source, language).replace(/\{([a-zA-Z][a-zA-Z0-9_]*)\}/g, (placeholder, name: string) => {
    const value = parameters[name]
    return typeof value === 'string' || typeof value === 'number' ? String(value) : placeholder
  })
}

export function interfaceTextProps(source: string, language: InterfaceLanguage = activeLanguage): { lang: string; dir: 'ltr' | 'rtl' } {
  const translatedValue = language === 'en' ? undefined : catalogCache.get(language)?.[source]
  const translated = typeof translatedValue === 'string' && translatedValue !== source
  return { lang: translated ? language : 'en', dir: translated && language === 'ar' ? 'rtl' : 'ltr' }
}

export function useInterfaceLanguage() {
  const snapshot = useSyncExternalStore(
    listener => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => currentSnapshot,
    () => serverSnapshot,
  )
  const setLanguage = (next: InterfaceLanguage) => {
    try { localStorage.setItem(STORAGE_KEY, next) } catch { /* Keep the current tab usable without storage. */ }
    activateLanguage(next)
  }
  const language = snapshot.language
  const t = (source: string, parameters: Record<string, string | number> = {}) => translateInterface(source, parameters, language)
  const textProps = (source: string) => interfaceTextProps(source, language)
  return { language, setLanguage, t, textProps }
}

loadCatalog(activeLanguage)
