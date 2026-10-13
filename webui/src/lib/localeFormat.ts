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

export function getRegionalLocale(): string | undefined {
  try {
    const value = localStorage.getItem(REGIONAL_FORMAT_KEY)
    return value && supportedRegionalFormats.has(value) && value !== 'system' ? value : undefined
  } catch {
    return undefined
  }
}
