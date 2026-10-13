import { useSyncExternalStore } from 'react'

export const REGIONAL_FORMAT_KEY = 'tng.regionalFormat'

export const REGIONAL_FORMAT_OPTIONS = [
  { value: 'system', label: 'Use browser settings' },
  { value: 'es', label: 'Español — Spanish' },
  { value: 'zh-CN', label: '简体中文 — Chinese (Simplified)' },
  { value: 'hi-IN', label: 'हिन्दी — Hindi' },
  { value: 'de', label: 'Deutsch — German' },
  { value: 'ja', label: '日本語 — Japanese' },
  { value: 'fr', label: 'Français — French' },
  { value: 'pt-BR', label: 'Português (Brasil) — Brazilian Portuguese' },
  { value: 'ar', label: 'العربية — Arabic' },
  { value: 'ko', label: '한국어 — Korean' },
  { value: 'zh-TW', label: '繁體中文 — Chinese (Traditional)' },
  { value: 'vi', label: 'Tiếng Việt — Vietnamese' },
  { value: 'id', label: 'Bahasa Indonesia — Indonesian' },
] as const

const supportedRegionalFormats = new Set<string>(REGIONAL_FORMAT_OPTIONS.map(option => option.value))
const listeners = new Set<() => void>()

function readRegionalFormat(): string {
  try {
    const value = localStorage.getItem(REGIONAL_FORMAT_KEY)
    return value && supportedRegionalFormats.has(value) ? value : 'system'
  } catch {
    return 'system'
  }
}

let activeRegionalFormat = readRegionalFormat()

function notifyRegionalFormatChange() {
  listeners.forEach(listener => listener())
}

if (typeof window !== 'undefined') {
  window.addEventListener('storage', event => {
    if (event.key !== REGIONAL_FORMAT_KEY && event.key !== null) return
    activeRegionalFormat = readRegionalFormat()
    notifyRegionalFormatChange()
  })
}

export function setRegionalFormatPreference(value: string) {
  if (!supportedRegionalFormats.has(value)) return
  activeRegionalFormat = value
  try {
    localStorage.setItem(REGIONAL_FORMAT_KEY, value)
  } catch {
    // Keep the selected format active for this tab without browser storage.
  }
  notifyRegionalFormatChange()
}

export function useRegionalFormat(): string {
  return useSyncExternalStore(
    listener => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => activeRegionalFormat,
    () => 'system',
  )
}

export function getRegionalFormat(): string {
  return activeRegionalFormat
}

export function getRegionalLocale(): string | undefined {
  return activeRegionalFormat === 'system' ? undefined : activeRegionalFormat
}

export function formatRegionalNumber(value: number, options?: Intl.NumberFormatOptions): string {
  return new Intl.NumberFormat(getRegionalLocale(), options).format(value)
}
