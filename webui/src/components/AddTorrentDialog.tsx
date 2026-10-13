import { interfaceTextProps, translateInterface } from '../lib/interfaceLocale'
import { formatRegionalNumber, getRegionalLocale } from '../lib/localeFormat'
import { useInterfaceLanguage } from '../lib/interfaceLocale'
import { useState, useRef, useCallback } from 'react'
import { useQueryClient, useQuery } from '@tanstack/react-query'
import { api } from '../api/client'
import { useDialogFocus } from '../hooks/useDialogFocus'

interface Props {
  onClose: () => void
}

const INPUT: React.CSSProperties = {
  background: 'var(--bg)', border: '1px solid var(--border-strong)', borderRadius: 5,
  color: 'var(--text)', padding: '6px 10px', fontSize: 13, outline: 'none',
  width: '100%', boxSizing: 'border-box',
}

export function AddTorrentDialog({ onClose }: Props) {
  const qc = useQueryClient()
  const [url, setUrl] = useState('')
  const [savePath, setSavePath] = useState('')
  const [category, setCategory] = useState('')
  const [start, setStart] = useState(true)
  const [dragOver, setDragOver] = useState(false)
  const [files, setFiles] = useState<File[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)
  const dialogRef = useDialogFocus(onClose)
  const { t, textProps } = useInterfaceLanguage()

  const { data: categories = [] } = useQuery({ queryKey: ['categories'], queryFn: api.categories.list })
  const urlCount = url.split('\n').map(line => line.trim()).filter(Boolean).length
  const stagedFilesSource = files.length === 1 ? '{count} file staged' : '{count} files staged'
  const stagedUrlsSource = urlCount === 1 ? '{count} URL staged' : '{count} URLs staged'
  const stagedFilesCount = files.length.toLocaleString(getRegionalLocale())
  const stagedUrlsCount = urlCount.toLocaleString(getRegionalLocale())
  const canSubmit = !busy && (urlCount > 0 || files.length > 0)
  const selectedCategory = categories.find(c => c.name === category)
  const effectiveSavePath = savePath.trim() || selectedCategory?.save_path || ''

  const addFiles = useCallback((incoming: FileList | File[]) => {
    const all = Array.from(incoming)
    const valid = all.filter(f => f.name.toLowerCase().endsWith('.torrent'))
    if (valid.length !== all.length) {
      setError(t('Only .torrent files can be added from the file picker.'))
    } else {
      setError(null)
    }
    setFiles(prev => {
      const names = new Set(prev.map(f => f.name))
      return [...prev, ...valid.filter(f => !names.has(f.name))]
    })
  }, [t])

  function handleDrop(e: React.DragEvent) {
    e.preventDefault()
    setDragOver(false)
    addFiles(e.dataTransfer.files)
  }

  async function submit() {
    if (!url.trim() && files.length === 0) {
      setError(t('Enter a URL or drag a .torrent file.'))
      return
    }
    setBusy(true)
    setError(null)
    try {
      const effectiveSavePath = savePath.trim()

      // Upload .torrent files
      for (const file of files) {
        await api.torrents.addFile(file, effectiveSavePath, category, start)
      }

      // Load magnet/HTTP URLs
      for (const line of url.split('\n')) {
        const u = line.trim()
        if (!u) continue
        await api.torrents.addMagnet(u, effectiveSavePath, category, start)
      }

      qc.invalidateQueries({ queryKey: ['torrents'], exact: false })
      onClose()
    } catch {
      setError(t('Failed to add torrent. Check the URL or file and try again.'))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="tng-modal-backdrop" style={{
      position: 'fixed', inset: 0, background: 'rgba(2,6,23,0.72)', display: 'flex',
      alignItems: 'center', justifyContent: 'center', zIndex: 100,
    }} onClick={e => { if (!busy && e.target === e.currentTarget) onClose() }}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="add-torrent-title" aria-describedby="add-torrent-description" tabIndex={-1} className="tng-modal tng-add-dialog" style={{
        background: 'var(--panel)', border: '1px solid var(--border)', borderRadius: 10,
        width: 480, maxWidth: '95vw', padding: 24, display: 'flex', flexDirection: 'column', gap: 16,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
          <div>
              <h2 id="add-torrent-title" {...textProps('Add torrent')} style={{ margin: 0, fontWeight: 800, fontSize: 15, color: 'var(--text)' }}>{t('Add torrent')}</h2>
            <div id="add-torrent-description" {...textProps('Stage files, magnets, or HTTP torrent URLs')} style={{ color: 'var(--faint)', fontSize: 12, marginTop: 2 }}>{t('Stage files, magnets, or HTTP torrent URLs')}</div>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            aria-label={t('Close add torrent dialog')}
            {...textProps('Close add torrent dialog')}
            style={{
              background: 'none', border: 'none', color: 'var(--faint)', fontSize: 18,
              cursor: busy ? 'not-allowed' : 'pointer', opacity: busy ? 0.55 : 1,
            }}
          >✕</button>
        </div>

        {/* Drag-drop zone */}
        <div
          className="tng-dropzone"
          data-active={dragOver ? 'true' : 'false'}
          data-filled={files.length > 0 ? 'true' : 'false'}
          role={files.length > 0 ? 'group' : 'button'}
          tabIndex={files.length > 0 ? undefined : 0}
          aria-label={files.length > 0 ? t('Staged torrent files') : t('Choose or drop torrent files')}
          {...textProps(files.length > 0 ? 'Staged torrent files' : 'Choose or drop torrent files')}
          aria-keyshortcuts={files.length > 0 ? undefined : 'Enter Space'}
          data-dialog-initial-focus
          onDragOver={e => { e.preventDefault(); setDragOver(true) }}
          onDragLeave={() => setDragOver(false)}
          onDrop={handleDrop}
          onClick={() => fileRef.current?.click()}
          onKeyDown={e => {
            if (files.length === 0 && (e.key === 'Enter' || e.key === ' ')) {
              e.preventDefault()
              fileRef.current?.click()
            }
          }}
          style={{
            border: `2px dashed ${dragOver ? 'var(--accent)' : files.length ? 'var(--success)' : 'var(--border-strong)'}`,
            borderRadius: 8, padding: '20px 16px', textAlign: 'center',
            cursor: 'pointer', transition: 'border-color 0.15s',
            background: dragOver ? 'var(--accent-soft)' : files.length ? 'color-mix(in srgb, var(--success) 7%, transparent)' : 'transparent',
            boxShadow: dragOver ? '0 0 0 3px color-mix(in srgb, var(--accent) 16%, transparent)' : undefined,
          }}
        >
          <input
            ref={fileRef}
            type="file"
            accept=".torrent"
            multiple
            aria-label={translateInterface("Torrent files")}
            style={{ display: 'none' }}
            onChange={e => e.target.files && addFiles(e.target.files)}
          {...interfaceTextProps("Torrent files")} />
          {files.length === 0 ? (
            <div style={{ display: 'grid', gap: 5 }}>
              <span {...textProps(dragOver ? 'Drop to stage torrents' : 'Drop .torrent files here')} style={{ fontSize: 14, color: dragOver ? 'var(--accent-text)' : 'var(--muted)', fontWeight: 700 }}>
                {t(dragOver ? 'Drop to stage torrents' : 'Drop .torrent files here')}
              </span>
              <span {...textProps('or')} style={{ fontSize: 12, color: 'var(--faint)' }}>
                {t('or')} <span {...textProps('browse')} style={{ color: 'var(--accent)' }}>{t('browse')}</span>
              </span>
            </div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              {files.map(f => (
                <div key={f.name} className="tng-staged-file" style={{
                  display: 'flex', alignItems: 'center', gap: 8,
                  border: '1px solid var(--border)', borderRadius: 6, padding: '5px 7px',
                  background: 'var(--surface)',
                }}>
                  <span style={{ fontSize: 12, color: 'var(--muted)', flex: 1, textAlign: 'left', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>📄 {f.name}</span>
                  <span style={{ color: 'var(--faint)', fontSize: 10, fontVariantNumeric: 'tabular-nums' }}>{fmtSize(f.size)}</span>
                  <button type="button" aria-label={t('Remove {name}', { name: f.name })} {...textProps('Remove {name}')} onClick={e => { e.stopPropagation(); setFiles(p => p.filter(x => x !== f)) }}
                    style={{ background: 'none', border: 'none', color: '#ef4444', cursor: 'pointer', fontSize: 14, padding: '0 2px' }}>✕</button>
                </div>
              ))}
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8, flexWrap: 'wrap', marginTop: 4 }}>
                <span {...textProps(stagedFilesSource)} style={{ fontSize: 11, color: 'var(--success)' }}>
                  {t(stagedFilesSource, { count: stagedFilesCount })}
                </span>
                <button
                  type="button"
                  aria-label={t('Add more torrent files')}
                  {...textProps('Add more torrent files')}
                  onClick={e => { e.stopPropagation(); fileRef.current?.click() }}
                  style={{
                    background: 'var(--surface-2)', border: '1px solid var(--border-strong)', borderRadius: 4,
                    color: 'var(--accent-text)', padding: '2px 7px', fontSize: 10, cursor: 'pointer',
                  }}
                >{t('Select more files')}</button>
              </div>
            </div>
          )}
        </div>

        {/* URL input */}
        <div className="tng-form-card" style={{
          border: '1px solid var(--border)', borderRadius: 7, background: 'var(--surface)',
          padding: '9px 10px',
        }}>
          <label htmlFor="add-torrent-urls" {...textProps('Magnet links or URLs (one per line)')} style={{ fontSize: 11, color: 'var(--faint)', display: 'block', marginBottom: 4 }}>
            {t('Magnet links or URLs (one per line)')}
          </label>
          <textarea
            id="add-torrent-urls"
            value={url}
            onChange={e => setUrl(e.target.value)}
            placeholder="magnet:?xt=urn:btih:…"
            rows={3}
            style={{ ...INPUT, resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }}
          />
          {urlCount > 0 && (
            <div style={{ marginTop: 5, color: 'var(--accent-text)', fontSize: 11 }}>
              <span {...textProps(stagedUrlsSource)}>
                {t(stagedUrlsSource, { count: stagedUrlsCount })}
              </span>
            </div>
          )}
        </div>

        {/* Save path */}
        <div className="tng-form-card" style={{
          border: '1px solid var(--border)', borderRadius: 7, background: 'var(--surface)',
          padding: '9px 10px',
        }}>
          <label htmlFor="add-torrent-save-path" {...textProps('Save path')} style={{ fontSize: 11, color: 'var(--faint)', display: 'block', marginBottom: 4 }}>{t('Save path')}</label>
          <input
            id="add-torrent-save-path"
            value={savePath}
            onChange={e => setSavePath(e.target.value)}
            placeholder="/data/downloads"
            style={INPUT}
          />
        </div>

        {/* Category */}
        {categories.length > 0 && (
          <div className="tng-form-card" style={{
            border: '1px solid var(--border)', borderRadius: 7, background: 'var(--surface)',
            padding: '9px 10px',
          }}>
            <label htmlFor="add-torrent-category" {...textProps('Category')} style={{ fontSize: 11, color: 'var(--faint)', display: 'block', marginBottom: 4 }}>{t('Category')}</label>
            <select
              id="add-torrent-category"
              value={category}
              onChange={e => {
                setCategory(e.target.value)
                const cat = categories.find(c => c.name === e.target.value)
                if (cat?.save_path && !savePath) setSavePath(cat.save_path)
              }}
              style={{ ...INPUT, cursor: 'pointer' }}
            >
              <option value="">{t('None')}</option>
              {categories.map(c => <option key={c.name} value={c.name}>{c.name}</option>)}
            </select>
            {selectedCategory?.save_path && (
              <div style={{ marginTop: 5, color: 'var(--faint)', fontSize: 11, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                <span {...textProps('Category path:')}>{t('Category path:')}</span> <span style={{ color: 'var(--muted)' }}>{selectedCategory.save_path}</span>
              </div>
            )}
          </div>
        )}

        {/* Start toggle */}
        <label className="tng-form-card" data-start={start ? 'true' : 'false'} style={{
          display: 'flex', alignItems: 'center', gap: 8, cursor: busy ? 'not-allowed' : 'pointer',
          fontSize: 13, color: 'var(--muted)', border: '1px solid var(--border)', borderRadius: 7,
          background: 'var(--surface)', padding: '8px 10px',
        }}>
          <input type="checkbox" checked={start} disabled={busy} onChange={e => setStart(e.target.checked)} style={{ accentColor: 'var(--accent)' }} />
          <span {...textProps('Start immediately')}>{t('Start immediately')}</span>
        </label>

        <div className="tng-add-summary" data-ready={canSubmit ? 'true' : 'false'} data-start={start ? 'true' : 'false'} style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10,
          padding: '7px 9px', border: '1px solid var(--border)', borderRadius: 6,
          background: 'var(--surface)', color: 'var(--faint)', fontSize: 12,
        }}>
          <span style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            <span {...textProps(stagedFilesSource)}>{t(stagedFilesSource, { count: stagedFilesCount })}</span>
            {' · '}
            <span {...textProps(stagedUrlsSource)}>{t(stagedUrlsSource, { count: stagedUrlsCount })}</span>
            {effectiveSavePath ? ` · ${effectiveSavePath}` : ''}
          </span>
          <span style={{ color: start ? 'var(--success)' : 'var(--warning)' }}>
            {t(start ? 'will start' : 'paused on add')}
          </span>
        </div>

        {error && <div role="alert" aria-live="assertive" style={noticeStyle}>{error}</div>}

        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', flexWrap: 'wrap' }}>
          <button type="button" onClick={onClose} disabled={busy} style={{
            background: 'none', border: '1px solid var(--border-strong)', borderRadius: 5,
            color: 'var(--faint)', padding: '6px 16px', fontSize: 13, cursor: 'pointer',
            opacity: busy ? 0.5 : 1,
          }} {...textProps('Cancel')}>{t('Cancel')}</button>
          <button type="button" onClick={submit} disabled={!canSubmit} {...textProps(busy ? 'Adding…' : 'Add')} style={{
            background: 'var(--accent-soft)', border: '1px solid var(--accent)', borderRadius: 5,
            color: 'var(--accent-text)', padding: '6px 20px', fontSize: 13,
            cursor: canSubmit ? 'pointer' : 'not-allowed', opacity: canSubmit ? 1 : 0.55,
          }}>
            {t(busy ? 'Adding…' : 'Add')}
          </button>
        </div>
      </div>
    </div>
  )
}

function fmtSize(bytes: number): string {
  if (bytes >= 1e9) return formatRegionalNumber(bytes / 1e9, { minimumFractionDigits: 1, maximumFractionDigits: 1 }) + ' GB'
  if (bytes >= 1e6) return formatRegionalNumber(bytes / 1e6, { minimumFractionDigits: 1, maximumFractionDigits: 1 }) + ' MB'
  if (bytes >= 1e3) return formatRegionalNumber(bytes / 1e3, { maximumFractionDigits: 0 }) + ' KB'
  return formatRegionalNumber(bytes) + ' B'
}

const noticeStyle: React.CSSProperties = {
  fontSize: 12,
  color: 'var(--danger)',
  background: 'color-mix(in srgb, var(--danger) 9%, var(--surface))',
  border: '1px solid color-mix(in srgb, var(--danger) 45%, var(--border))',
  borderRadius: 6,
  padding: '8px 9px',
  overflowWrap: 'anywhere',
}
