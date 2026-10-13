import type { ThemeMode } from '../themes'
import { REGIONAL_FORMAT_OPTIONS } from '../lib/localeFormat'
import { INTERFACE_LANGUAGE_OPTIONS, useInterfaceLanguage, type InterfaceLanguage } from '../lib/interfaceLocale'

export type MediaInferenceMode = 'full' | 'suffix' | 'hints' | 'off'

interface Props {
  mediaInference: MediaInferenceMode
  onMediaInference: (mode: MediaInferenceMode) => void
  themes?: Array<{ id: string; label: string; dark: Record<string, string>; light: Record<string, string> }>
  themeId?: string
  themeMode?: ThemeMode
  onTheme?: (id: string) => void
  onThemeMode?: (mode: ThemeMode) => void
  regionalFormat: string
  onRegionalFormat: (value: string) => void
}

const MODES: Array<{ value: MediaInferenceMode; label: string; description: string }> = [
  { value: 'full', label: 'Full', description: 'Use suffixes plus name, category, tag, and path hints.' },
  { value: 'suffix', label: 'Suffix only', description: 'Classify only from visible file extensions.' },
  { value: 'hints', label: 'Hints only', description: 'Classify from names, categories, tags, and paths without suffix parsing.' },
  { value: 'off', label: 'Disabled', description: 'Show a neutral type icon for every torrent.' },
]

export function AppearancePanel({ mediaInference, onMediaInference, themes = [], themeId, themeMode = 'dark', onTheme, onThemeMode, regionalFormat, onRegionalFormat }: Props) {
  const { language, setLanguage, t, textProps } = useInterfaceLanguage()
  const alternateMode: ThemeMode = themeMode === 'dark' ? 'light' : 'dark'
  const localized = (source: string, parameters?: Record<string, string | number>) => (
    <span {...textProps(source)}>{t(source, parameters)}</span>
  )

  return (
    <section aria-labelledby="appearance-title" style={{ padding: 18 }}>
      <h2 id="appearance-title" {...textProps('Appearance')} style={{ margin: '0 0 4px', fontSize: 14, fontWeight: 700, color: 'var(--text)' }}>{t("Appearance")}</h2>
      <div {...textProps('Configure presentation-only behavior for this browser.')} style={{ fontSize: 12, color: 'var(--faint)', marginBottom: 14 }}>
        {t('Configure presentation-only behavior for this browser.')}
      </div>

      <div className="tng-card tng-appearance-panel" style={{ ...panelStyle, marginBottom: 12 }}>
        <label htmlFor="interface-language" {...textProps('Interface language')} style={{ display: 'grid', gap: 6, color: 'var(--text)', fontSize: 12, fontWeight: 700, marginBottom: 12 }}>
          {localized('Interface language')}
          <select id="interface-language" value={language} onChange={event => setLanguage(event.currentTarget.value as InterfaceLanguage)} style={{ color: 'var(--text)', background: 'var(--surface)', border: '1px solid var(--border)', borderRadius: 5, padding: '8px 9px', font: 'inherit' }}>
            {INTERFACE_LANGUAGE_OPTIONS.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
          </select>
        </label>
        <label htmlFor="regional-format" {...textProps('Date, time, and number format')} style={{ display: 'grid', gap: 6, color: 'var(--text)', fontSize: 12, fontWeight: 700 }}>
          {localized('Date, time, and number format')}
          <select
            id="regional-format"
            value={regionalFormat}
            onChange={event => onRegionalFormat(event.currentTarget.value)}
            style={{ color: 'var(--text)', background: 'var(--surface)', border: '1px solid var(--border)', borderRadius: 5, padding: '8px 9px', font: 'inherit' }}
          >
            {REGIONAL_FORMAT_OPTIONS.map(option => (
              <option key={option.value} value={option.value}>
                {option.value === 'system' ? t(option.label) : option.label}
              </option>
            ))}
          </select>
        </label>
        <div {...textProps('Choose how dates, times, and numbers are displayed. This setting is separate from interface language.')} style={{ fontSize: 11, color: 'var(--faint)', marginTop: 5 }}>
          {t("Choose how dates, times, and numbers are displayed. This setting is separate from interface language.")}
        </div>
      </div>

      {themes.length > 0 && (
        <div className="tng-card tng-appearance-panel" style={{ ...panelStyle, marginBottom: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10, marginBottom: 10 }}>
            <div>
              <h3 {...textProps('Theme palette')} style={{ margin: 0, fontSize: 12, color: 'var(--text)', fontWeight: 800 }}>{t('Theme palette')}</h3>
              <div {...textProps('Preview the full shell colors used by each theme.')} style={{ fontSize: 11, color: 'var(--faint)', marginTop: 2 }}>{t('Preview the full shell colors used by each theme.')}</div>
            </div>
            <button
              type="button"
              onClick={() => onThemeMode?.(alternateMode)}
              aria-label={t('Switch to {mode} theme', { mode: t(alternateMode === 'dark' ? 'Dark' : 'Light') })}
              {...textProps('Switch to {mode} theme')}
              aria-pressed={themeMode === 'dark'}
              style={{
                color: 'var(--accent-text)', background: 'var(--accent-soft)', border: '1px solid var(--accent)',
                borderRadius: 999, padding: '3px 9px', fontSize: 11, fontWeight: 800, cursor: 'pointer',
              }}
            >
              <span {...textProps(themeMode === 'dark' ? 'Dark' : 'Light')}>{t(themeMode === 'dark' ? 'Dark' : 'Light')}</span>
            </button>
          </div>

          <div className="tng-theme-gallery">
            {themes.map(theme => {
              const tokens = theme[themeMode]
              const active = theme.id === themeId
              return (
                <button
                  key={theme.id}
                  type="button"
                  className="tng-theme-card"
                  data-active={active ? 'true' : 'false'}
                  aria-label={t(active ? 'Active theme {name}' : 'Select theme {name}', { name: theme.label })}
                  {...textProps(active ? 'Active theme {name}' : 'Select theme {name}')}
                  aria-pressed={active}
                  onClick={() => onTheme?.(theme.id)}
                  style={{
                    ['--preview-bg' as string]: tokens.bg,
                    ['--preview-panel' as string]: tokens.panel,
                    ['--preview-surface' as string]: tokens.surface,
                    ['--preview-row' as string]: tokens.row,
                    ['--preview-alt' as string]: tokens.rowAlt,
                    ['--preview-accent' as string]: tokens.accent,
                    ['--preview-border' as string]: tokens.borderStrong,
                    ['--preview-text' as string]: tokens.text,
                    ['--preview-muted' as string]: tokens.muted,
                    ['--preview-shadow' as string]: tokens.shadow,
                  }}
                >
                  <span className="tng-theme-card-preview" aria-hidden="true">
                    <span className="tng-theme-card-bar" />
                    <span className="tng-theme-card-row" />
                    <span className="tng-theme-card-row" />
                    <span className="tng-theme-card-row" />
                  </span>
                  <span className="tng-theme-card-label">
                    <span {...textProps('Select theme {name}')}>{theme.label}</span>
                    {active && <span {...textProps('Active')}>{t('Active')}</span>}
                  </span>
                </button>
              )
            })}
          </div>
        </div>
      )}

      <div className="tng-card tng-appearance-panel" style={panelStyle}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10, marginBottom: 10 }}>
          <div>
              <h3 {...textProps('Media type inference')} style={{ margin: 0, fontSize: 12, color: 'var(--text)', fontWeight: 800 }}>{t('Media type inference')}</h3>
              <div {...textProps('Controls the icon and type filters only.')} style={{ fontSize: 11, color: 'var(--faint)', marginTop: 2 }}>{t('Controls the icon and type filters only.')}</div>
          </div>
          <span style={{
            color: 'var(--accent-text)', background: 'var(--accent-soft)', border: '1px solid var(--accent)',
            borderRadius: 999, padding: '2px 8px', fontSize: 11, fontWeight: 800,
          }} {...textProps(MODES.find(mode => mode.value === mediaInference)?.label ?? 'Full')}>{t(MODES.find(mode => mode.value === mediaInference)?.label ?? 'Full')}</span>
        </div>
        <div role="radiogroup" aria-label={t('Media type inference mode')} {...textProps('Media type inference mode')}>
        {MODES.map(mode => (
          <label key={mode.value} className="tng-appearance-option" data-active={mediaInference === mode.value ? 'true' : 'false'} style={{
            display: 'grid', gridTemplateColumns: 'auto 1fr auto', gap: 10, alignItems: 'start',
            border: '1px solid ' + (mediaInference === mode.value ? 'var(--accent)' : 'var(--border)'),
            borderRadius: 6, padding: 10, background: mediaInference === mode.value ? 'var(--accent-soft)' : 'var(--surface)',
            cursor: 'pointer', marginTop: 8,
          }}>
            <input
              type="radio"
              name="mediaInference"
              aria-label={t(mode.label)}
              {...textProps(mode.label)}
              checked={mediaInference === mode.value}
              onChange={() => onMediaInference(mode.value)}
              style={{ accentColor: 'var(--accent)', marginTop: 2 }}
            />
            <span>
              <span {...textProps(mode.label)} style={{ display: 'block', color: 'var(--text)', fontSize: 13, fontWeight: 600 }}>{t(mode.label)}</span>
              <span {...textProps(mode.description)} style={{ display: 'block', color: 'var(--faint)', fontSize: 12, marginTop: 2 }}>{t(mode.description)}</span>
            </span>
            {mediaInference === mode.value && (
              <span style={{
                color: 'var(--accent-text)', background: 'var(--accent-soft)', border: '1px solid var(--accent)',
                borderRadius: 999, padding: '1px 7px', fontSize: 10, fontWeight: 700,
              }} {...textProps('Active')}>{t('Active')}</span>
            )}
          </label>
        ))}
        </div>
      </div>
    </section>
  )
}

const panelStyle: React.CSSProperties = {
  display: 'grid',
  gap: 0,
  maxWidth: 760,
  background: 'color-mix(in srgb, var(--surface) 84%, var(--bg))',
  border: '1px solid var(--border)',
  borderRadius: 8,
  padding: 12,
  boxShadow: 'inset 0 1px 0 rgba(255,255,255,0.03)',
}
