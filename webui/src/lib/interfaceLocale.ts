import { useSyncExternalStore } from 'react'
import { UI_LABELS } from './ui-labels'

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
const STORAGE_KEY = 'torrentng.interface-language'
const listeners = new Set<() => void>()

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

function applyDocumentLanguage(_language: InterfaceLanguage) {
  if (typeof document === 'undefined') return
  document.documentElement.lang = 'en'
  document.documentElement.dir = 'ltr'
}

let activeLanguage = readLanguage()

function notify() {
  listeners.forEach(listener => listener())
}

if (typeof window !== 'undefined') {
  window.addEventListener('storage', event => {
    if (event.key !== STORAGE_KEY && event.key !== null) return
    activeLanguage = readLanguage()
    applyDocumentLanguage(activeLanguage)
    notify()
  })
}

export function applyInitialInterfaceLanguage() {
  activeLanguage = readLanguage()
  applyDocumentLanguage(activeLanguage)
}

export function useInterfaceLanguage() {
  const language = useSyncExternalStore<InterfaceLanguage>(
    listener => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => activeLanguage,
    () => 'en',
  )
  const setLanguage = (next: InterfaceLanguage) => {
    activeLanguage = next
    try { localStorage.setItem(STORAGE_KEY, next) } catch { /* Keep the current tab usable without storage. */ }
    applyDocumentLanguage(next)
    notify()
  }
  const t = (source: string) => {
    if (language === 'en') return source
    const entry = (UI_LABELS as Record<string, Partial<Record<InterfaceLanguage, string>>>)[source]
    return entry?.[language] ?? source
  }
  return { language, setLanguage, t }
}
