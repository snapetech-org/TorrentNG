import { LocalizedErrorText, LocalizedText } from '../lib/LocalizedText'
import { useInterfaceLanguage } from '../lib/interfaceLocale'
import { getRegionalLocale } from '../lib/localeFormat'
import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../api/client'
import { useDialogFocus } from '../hooks/useDialogFocus'

interface Props {
  hashes: string[]
  onClose: () => void
}

const INPUT: React.CSSProperties = {
  width: '100%', background: 'var(--bg)', border: '1px solid var(--border-strong)', borderRadius: 5,
  color: 'var(--text)', padding: '6px 8px', fontSize: 12, outline: 'none', boxSizing: 'border-box',
}

export function BulkEditDialog({ hashes, onClose }: Props) {
  const qc = useQueryClient()
  const { t, textProps } = useInterfaceLanguage()
  const [category, setCategory] = useState('')
  const [tags, setTags] = useState('')
  const [location, setLocation] = useState('')
  const [ratioLimit, setRatioLimit] = useState('')
  const [seedMinutes, setSeedMinutes] = useState('')
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const [messageTone, setMessageTone] = useState<'ok' | 'error'>('ok')
  const dialogRef = useDialogFocus(onClose)

  const { data: categories = [] } = useQuery({ queryKey: ['categories'], queryFn: api.categories.list })
  const { data: allTags = [] } = useQuery({ queryKey: ['tags'], queryFn: api.tags.list })

  async function apply(label: string, fn: () => Promise<void>) {
    setBusy(true)
    setMessage('')
    setMessageTone('ok')
    try {
      await fn()
      setMessage(label)
      qc.invalidateQueries({ queryKey: ['torrents'], exact: false })
      qc.invalidateQueries({ queryKey: ['categories'] })
      qc.invalidateQueries({ queryKey: ['tags'] })
      qc.invalidateQueries({ queryKey: ['tracker-health'] })
    } catch (err) {
      setMessage(err instanceof Error ? err.message : String(err))
      setMessageTone('error')
    } finally {
      setBusy(false)
    }
  }

  const tagList = tags.split(',').map(tag => tag.trim()).filter(Boolean)

  return (
    <div className="tng-modal-backdrop" style={{
      position: 'fixed', inset: 0, background: 'rgba(2,6,23,0.72)', zIndex: 1200,
      display: 'grid', placeItems: 'center', padding: 22,
    }} onClick={e => { if (!busy && e.target === e.currentTarget) onClose() }}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="bulk-edit-title" aria-describedby="bulk-edit-description" aria-busy={busy} tabIndex={-1} className="tng-modal" style={{
        width: 'min(620px, 100%)', maxHeight: '88vh', overflowY: 'auto',
        background: 'var(--panel)', border: '1px solid var(--border-strong)', borderRadius: 8,
        boxShadow: '0 24px 60px var(--shadow)',
      }} onClick={e => e.stopPropagation()}>
        <header style={{ padding: '13px 15px', borderBottom: '1px solid var(--border)', display: 'flex', gap: 12, alignItems: 'center' }}>
          <div style={{ flex: 1 }}>
            <h2 id="bulk-edit-title" style={{ margin: 0, color: 'var(--text)', fontWeight: 700, fontSize: 15 }}><LocalizedText source="Edit selected torrents" /></h2>
            <div id="bulk-edit-description" style={{ color: 'var(--faint)', fontSize: 12 }}>
              <LocalizedText source="{count} selected" parameters={{ count: hashes.length.toLocaleString(getRegionalLocale()) }} />
            </div>
          </div>
          {busy && <span role="status" aria-live="polite" style={{
            color: 'var(--accent-text)', background: 'var(--accent-soft)', border: '1px solid var(--accent)',
            borderRadius: 999, padding: '2px 8px', fontSize: 11, fontWeight: 700,
          }}><LocalizedText source="Applying" /></span>}
          <button type="button" onClick={onClose} disabled={busy} style={smallButton('var(--muted)', busy)}><LocalizedText source="Close" /></button>
        </header>

        <div style={{ padding: 15, display: 'grid', gap: 14 }}>
          <Field source="Category">
            <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 8 }}>
              <select aria-label={t('Category')} {...textProps('Category')} value={category} onChange={e => setCategory(e.target.value)} disabled={busy} style={INPUT}>
                <option value=""><LocalizedText source="Clear category" /></option>
                {categories.map(cat => <option key={cat.name} value={cat.name}>{cat.name}</option>)}
              </select>
              <button type="button" disabled={busy} onClick={() => apply('Category applied', async () => { await api.bulk('set-category', hashes, false, { category }) })} style={smallButton('#93c5fd', busy)}><LocalizedText source="Apply" /></button>
            </div>
          </Field>

          <Field source="Tags">
            <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 8 }}>
              <input aria-label={t('Tags')} {...textProps('Tags')} value={tags} onChange={e => setTags(e.target.value)} disabled={busy} placeholder={t('Comma-separated tags')} style={INPUT} {...textProps('Comma-separated tags')} />
              <button type="button" disabled={busy} onClick={() => apply('Tags applied', () => api.torrents.setTags(hashes, tagList))} style={smallButton('#93c5fd', busy)}><LocalizedText source="Set tags" /></button>
            </div>
            {allTags.length > 0 && (
              <div style={{ display: 'flex', gap: 5, flexWrap: 'wrap', marginTop: 7 }}>
                {allTags.map(tag => (
                  <button
                    type="button"
                    key={tag}
                    className="tng-tag-chip"
                    data-active={tagList.includes(tag) ? 'true' : 'false'}
                    disabled={busy}
                    onClick={() => setTags(Array.from(new Set([...tagList, tag])).join(','))}
                    style={{ ...chipButton, opacity: busy ? 0.55 : 1, cursor: busy ? 'not-allowed' : 'pointer' }}
                  >{tag}</button>
                ))}
              </div>
            )}
          </Field>

          <Field source="Location">
            <div style={{ display: 'grid', gridTemplateColumns: '1fr auto auto', gap: 8 }}>
              <input aria-label={t('Location')} {...textProps('Location')} value={location} onChange={e => setLocation(e.target.value)} disabled={busy} placeholder="/downloads/category" style={{ ...INPUT, fontFamily: 'monospace' }} />
              <button type="button" disabled={busy || !location.trim()} onClick={() => apply('Location previewed', async () => { await api.bulk('set-location', hashes, true, { save_path: location.trim() }) })} style={smallButton('var(--muted)', busy || !location.trim())}><LocalizedText source="Preview" /></button>
              <button type="button" disabled={busy || !location.trim()} onClick={() => apply('Location applied', () => api.torrents.setLocation(hashes, location.trim()))} style={smallButton('#93c5fd', busy || !location.trim())}><LocalizedText source="Move" /></button>
            </div>
          </Field>

          <Field source="Share limits">
            <div className="tng-action-grid-3" style={{ display: 'grid', gridTemplateColumns: '1fr 1fr auto', gap: 8 }}>
              <input aria-label={t('Ratio limit')} {...textProps('Ratio limit')} value={ratioLimit} onChange={e => setRatioLimit(e.target.value)} disabled={busy} placeholder={t('Ratio -2 default, -1 unlimited')} style={INPUT} />
              <input aria-label={t('Seed time limit')} {...textProps('Seed time limit')} value={seedMinutes} onChange={e => setSeedMinutes(e.target.value)} disabled={busy} placeholder={t('Minutes -2 default, -1 unlimited')} style={INPUT} />
              <button type="button" disabled={busy} onClick={() => apply('Share limits applied', () => api.torrents.setShareLimits(
                hashes,
                ratioLimit.trim() ? Number(ratioLimit) : -2,
                seedMinutes.trim() ? Number(seedMinutes) : -2,
              ))} style={smallButton('#93c5fd', busy)}><LocalizedText source="Apply" /></button>
            </div>
          </Field>

          <div className="tng-form-card" style={{
            display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10,
            background: 'var(--surface)', border: '1px solid var(--border)', borderRadius: 7, padding: 10,
          }}>
            <div>
              <div style={{ color: 'var(--text)', fontSize: 12, fontWeight: 700 }}><LocalizedText source="Playback order" /></div>
              <div style={{ color: 'var(--muted)', fontSize: 11, marginTop: 2 }}><LocalizedText source="Toggle sequential mode for every selected torrent." /></div>
            </div>
            <button type="button" disabled={busy} onClick={() => apply('Sequential toggled', () => api.torrents.toggleSequential(hashes))} style={{ ...smallButton('#f59e0b', busy), color: 'var(--text)' }}><LocalizedText source="Toggle sequential" /></button>
          </div>

          {message && <div role={messageTone === 'error' ? 'alert' : 'status'} aria-live={messageTone === 'error' ? 'assertive' : 'polite'} style={{
            color: messageTone === 'error' ? 'var(--danger)' : 'var(--success)', fontSize: 12,
            background: messageTone === 'error' ? 'color-mix(in srgb, var(--danger) 9%, var(--surface))' : 'color-mix(in srgb, var(--success) 8%, var(--surface))',
            border: '1px solid ' + (messageTone === 'error' ? 'color-mix(in srgb, var(--danger) 45%, var(--border))' : 'color-mix(in srgb, var(--success) 40%, var(--border))'),
            borderRadius: 6, padding: '8px 9px', overflowWrap: 'anywhere',
          }}>{messageTone === 'error' ? <LocalizedErrorText message={message} /> : <LocalizedText source={message} />}</div>}
        </div>
      </div>
    </div>
  )
}

function Field({ source, children }: { source: string; children: React.ReactNode }) {
  const { t, textProps } = useInterfaceLanguage()
  return <div className="tng-form-card" role="group" aria-label={t(source)} {...textProps(source)} style={{
    display: 'grid', gap: 6, color: 'var(--muted)', fontSize: 11, fontWeight: 700,
    textTransform: 'uppercase', background: 'var(--surface)', border: '1px solid var(--border)',
    borderRadius: 7, padding: 10,
    boxShadow: 'inset 0 1px 0 rgba(255,255,255,0.025)',
  }}>
    <span {...textProps(source)}>{t(source)}</span>
    {children}
  </div>
}

function smallButton(color: string, disabled = false): React.CSSProperties {
  return {
    background: 'var(--surface-2)', border: `1px solid color-mix(in srgb, ${color} 42%, var(--border))`, borderRadius: 5,
    color: `color-mix(in srgb, ${color} 78%, var(--text))`, padding: '6px 9px', fontSize: 12,
    cursor: disabled ? 'not-allowed' : 'pointer', opacity: disabled ? 0.55 : 1,
  }
}

const chipButton: React.CSSProperties = {
  background: 'var(--surface)',
  border: '1px solid var(--border-strong)',
  borderRadius: 12,
  color: 'var(--muted)',
  padding: '2px 7px',
  fontSize: 11,
  cursor: 'pointer',
}
