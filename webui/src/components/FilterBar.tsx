import { getRegionalLocale } from '../lib/localeFormat'
import { useInterfaceLanguage } from '../lib/interfaceLocale'
import { LocalizedText } from '../lib/LocalizedText'
import { useEffect, useId, useState } from 'react'
import type { ListParams } from '../api/client'
import { maskAnnounceUrl } from '../lib/maskUrl'

const SORT_COLUMNS: Record<string, string> = {
  name: 'Name', status: 'Status', size: 'Size', progress: 'Progress', remaining: 'Left',
  speed_down: 'Download speed', speed_up: 'Upload speed', seeds: 'Seeds', peers: 'Peers',
  ratio: 'Ratio', added: 'Added', completed: 'Completed',
}
const STATUS_LABELS: Record<string, string> = {
  downloading: 'Downloading', seeding: 'Seeding', completed: 'Completed', running: 'Running',
  queued: 'Queued', stopped: 'Stopped', active: 'Active', inactive: 'Inactive', stalled: 'Stalled',
  stalled_uploading: 'Stalled Uploading', stalled_downloading: 'Stalled Downloading', checking: 'Checking',
  moving: 'Moving', error: 'Errored', tracker_error: 'Tracker Errors',
}
const MEDIA_TYPE_LABELS: Record<string, string> = {
  ebook: 'Ebooks', tv: 'TV', video: 'Video', audio: 'Audio', iso: 'ISO/Image', game: 'Game',
  software: 'Software/Archive', other: 'Other',
}

function ownLabel(labels: Record<string, string>, value: string): string | undefined {
  return Object.prototype.hasOwnProperty.call(labels, value) ? labels[value] : undefined
}

interface Props {
  params: ListParams
  onChange: (p: Partial<ListParams>) => void
}

export function FilterBar({ params, onChange }: Props) {
  const { language, t, textProps } = useInterfaceLanguage()
  const sortDescriptionId = useId()
  const localized = (source: string, parameters?: Record<string, string | number>) => (
    <span {...textProps(source)}>{t(source, parameters)}</span>
  )
  const [search, setSearch] = useState(params.filter ?? '')
  const hasSidebarFilters = Boolean(params.status || params.category || params.tag || params.tracker || params.media_type)
  const activeCount = [params.filter || search, params.status, params.category, params.tag, params.tracker, params.media_type].filter(Boolean).length
  const sortColumnSource = params.sort ?? 'name'
  const sortColumn = params.sort ? ownLabel(SORT_COLUMNS, params.sort) : 'Name'
  const knownSortColumn = !params.sort || sortColumn !== undefined
  const sortColumnText = sortColumn ? t(sortColumn) : params.sort ?? t('Name')
  const sortDirection = params.dir === 'desc' ? 'Descending' : 'Ascending'
  const sortLabelSource = 'Sort {column} {direction}'
  const sortLabelColumn = language === 'en' ? sortColumnSource : sortColumnText
  const sortLabelDirection = language === 'en' ? (params.dir === 'desc' ? 'desc' : 'asc') : t(sortDirection)
  const sortDescriptionSource = 'Sorted by {column}, {direction}'
  const sortDescriptionColumn = knownSortColumn ? sortColumnText : `\u2068${params.sort}\u2069`
  const sortDescriptionDirection = t(sortDirection)
  const sortDescription = t(sortDescriptionSource, { column: sortDescriptionColumn, direction: sortDescriptionDirection })
  const chips = [
    (params.filter || search) && ['Search', params.filter || search, 'filter'],
    params.status && ['State', params.status, 'status'],
    params.media_type && ['Type', params.media_type, 'media_type'],
    params.category && ['Category', params.category, 'category'],
    params.tag && ['Tag', params.tag, 'tag'],
    params.tracker && ['Tracker', params.tracker, 'tracker'],
  ].filter(Boolean) as Array<[string, string, keyof ListParams]>

  useEffect(() => {
    if (search === (params.filter ?? '')) return
    const t = setTimeout(() => onChange({ filter: search, offset: 0 }), 200)
    return () => clearTimeout(t)
  }, [search, params.filter, onChange])

  useEffect(() => {
    setSearch(params.filter ?? '')
  }, [params.filter])

  return (
    <div className="tng-filterbar" role="region" aria-label={t('Torrent filters')} {...textProps('Torrent filters')} style={{
      display: 'flex',
      gap: 8,
      padding: '7px 12px',
      background: 'var(--surface-2)',
      borderBottom: '1px solid var(--border-strong)',
      alignItems: 'center',
      flexWrap: 'wrap',
    }}>
      <label className="tng-filterbar-search" style={{
        flex: '1 1 220px', minWidth: 0, display: 'flex', alignItems: 'center', gap: 7,
        background: 'var(--bg)', border: '1px solid ' + (search ? 'var(--accent)' : 'var(--border-strong)'), borderRadius: 6,
        padding: '0 9px',
        boxShadow: search ? '0 0 0 2px color-mix(in srgb, var(--accent) 14%, transparent)' : undefined,
      }}>
        <span aria-hidden="true" style={{ color: 'var(--faint)', fontSize: 12 }}>⌕</span>
        <input
          aria-label={t('Search torrents')}
          {...textProps('Search torrents')}
          type="search"
          placeholder={t('Search torrents')}
          value={search}
          onChange={e => setSearch(e.target.value)}
          style={{
            flex: 1,
            minWidth: 0,
            background: 'transparent',
            border: 0,
            color: 'var(--text)',
            padding: '5px 0',
            fontSize: 13,
            outline: 'none',
          }}
        />
        {search && (
          <span style={{
            color: 'var(--accent-text)', background: 'var(--accent-soft)',
            border: '1px solid color-mix(in srgb, var(--accent) 45%, var(--border))',
            borderRadius: 999, padding: '1px 6px', fontSize: 10, fontWeight: 800,
            whiteSpace: 'nowrap',
          }}>
            {localized('live')}
          </span>
        )}
      </label>

      {(params.filter || search) && (
        <button
          className="tng-filterbar-button"
          type="button"
          aria-label={t('Clear torrent search')}
          onClick={() => {
            setSearch('')
            onChange({ filter: undefined, offset: 0 })
          }}
          style={{
            background: 'none',
            border: '1px solid var(--border-strong)',
            borderRadius: 5,
            color: 'var(--faint)',
            padding: '3px 8px',
            fontSize: 11,
            cursor: 'pointer',
          }}
        >
          {localized('Clear search')}
        </button>
      )}
      {hasSidebarFilters && (
        <button
          className="tng-filterbar-button"
          type="button"
          aria-label={t('Clear sidebar filters')}
          onClick={() => onChange({
            status: undefined,
            category: undefined,
            tag: undefined,
            tracker: undefined,
            media_type: undefined,
            offset: 0,
          })}
          title={t('Clear sidebar filters')}
          style={{
            background: 'none',
            border: '1px solid var(--border-strong)',
            borderRadius: 5,
            color: 'var(--muted)',
            padding: '3px 8px',
            fontSize: 11,
            cursor: 'pointer',
          }}
        >
          {localized('Clear filters')}
        </button>
      )}
      {hasSidebarFilters && (
        <span className="tng-filterbar-count" role="status" style={{
          color: 'var(--accent-text)', background: 'var(--accent-soft)', border: '1px solid var(--accent)',
          borderRadius: 999, padding: '2px 8px', fontSize: 11, fontWeight: 800, whiteSpace: 'nowrap',
        }}>
          {localized('{count} active', { count: activeCount.toLocaleString(getRegionalLocale()) })}
        </span>
      )}
      {(chips.length > 0 || params.sort) && (
        <div className="tng-filterbar-chips" style={{ display: 'flex', gap: 5, flexWrap: 'wrap', minWidth: 0 }}>
          <span className="tng-filter-chip tng-filter-chip-muted" aria-describedby={!knownSortColumn ? sortDescriptionId : undefined} title={sortDescription} {...textProps(sortDescriptionSource)} style={{
            display: 'inline-flex', alignItems: 'center', gap: 4,
            maxWidth: 190, border: '1px solid var(--border)', borderRadius: 999,
            background: 'var(--surface)', color: 'var(--muted)', padding: '2px 7px',
            fontSize: 11,
          }}>
            <span {...textProps(sortLabelSource)} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              <LocalizedText
                source={sortLabelSource}
                parameters={{ column: sortLabelColumn, direction: sortLabelDirection }}
                undParameters={knownSortColumn ? [] : ['column']}
                parameterDirections={{ column: 'auto' }}
              />
            </span>
          </span>
          {!knownSortColumn && (
            <span id={sortDescriptionId} className="tng-visually-hidden">
              <LocalizedText
                source={sortDescriptionSource}
                parameters={{ column: params.sort ?? '', direction: sortDescriptionDirection }}
                undParameters={['column']}
                parameterDirections={{ column: 'auto' }}
              />
            </span>
          )}
          {chips.map(([label, value, key]) => {
            const statusSource = key === 'status' ? ownLabel(STATUS_LABELS, value) : undefined
            const mediaTypeSource = key === 'media_type' ? ownLabel(MEDIA_TYPE_LABELS, value) : undefined
            return (
              <span
                key={`${label}:${value}`}
                className="tng-filter-chip"
                title={key === 'tracker' ? maskAnnounceUrl(value) : key === 'filter' || key === 'category' || key === 'tag' ? value : undefined}
                dir={key === 'tracker' ? 'ltr' : key === 'filter' || key === 'category' || key === 'tag' ? 'auto' : undefined}
                lang={key === 'tracker' || key === 'filter' || key === 'category' || key === 'tag' ? 'und' : undefined}
                style={{
                  display: 'inline-flex', alignItems: 'center', gap: 4,
                  maxWidth: 190, border: '1px solid var(--border)', borderRadius: 999,
                  background: 'var(--surface)', color: 'var(--muted)', padding: '2px 7px',
                  fontSize: 11,
                }}>
                <span style={{ color: 'var(--faint)' }}>{localized(label)}</span>
                <span {...(statusSource
                  ? textProps(statusSource)
                  : mediaTypeSource
                    ? textProps(mediaTypeSource)
                    : key === 'status' || key === 'media_type' ? {} : textProps(value))} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {key === 'tracker' ? <bdi dir="ltr" lang="und">{maskAnnounceUrl(value)}</bdi>
                    : key === 'status' ? statusSource
                      ? t(statusSource) : <bdi dir="auto" lang="und">{value}</bdi>
                      : key === 'media_type' ? mediaTypeSource
                        ? t(mediaTypeSource) : <bdi dir="auto" lang="und">{value}</bdi>
                        : <bdi dir="auto" lang="und">{value}</bdi>}
                </span>
                <button
                  type="button"
                  aria-label={t('Clear {label} filter', { label: t(label) })}
                  onClick={() => {
                    if (key === 'filter') setSearch('')
                    onChange({ [key]: undefined, offset: 0 })
                  }}
                  style={{
                    background: 'transparent', border: 0, color: 'var(--faint)', padding: '0 0 0 2px',
                    fontSize: 12, lineHeight: 1, cursor: 'pointer',
                  }}
                >
                  ×
                </button>
              </span>
            )
          })}
        </div>
      )}
    </div>
  )
}
