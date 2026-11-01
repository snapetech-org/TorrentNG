import { LocalizedServerEventKind, LocalizedServerMessage, LocalizedText } from '../lib/LocalizedText'
import { interfaceTextProps, translateInterface } from '../lib/interfaceLocale'
import { getRegionalLocale } from '../lib/localeFormat'
import { SERVER_EVENT_KIND_LABELS } from '../lib/serverEventKinds'
import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api, type AppLogEvent } from '../api/client'

const LIMIT = 100

export function LogsPanel() {
  const [level, setLevel] = useState('')
  const [kind, setKind] = useState('')
  const [lastKnownId, setLastKnownId] = useState<number | undefined>()
  const query = useQuery({
    queryKey: ['logs', level, kind, lastKnownId],
    queryFn: () => api.logs({ limit: LIMIT, level, kind, last_known_id: lastKnownId }),
    refetchInterval: lastKnownId ? 5_000 : 15_000,
  })
  const logs = query.data?.logs ?? []
  const newestId = useMemo(
    () => logs.reduce((max, event) => Math.max(max, event.event_id ?? 0), lastKnownId ?? 0),
    [logs, lastKnownId],
  )

  return (
    <section aria-labelledby="operator-logs-title" aria-busy={query.isFetching} style={{ padding: '18px 24px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12, flexWrap: 'wrap' }}>
        <h2 id="operator-logs-title" style={{ margin: 0, fontSize: 14, fontWeight: 700, color: 'var(--text)', flex: 1, minWidth: 180 }}>
          <LocalizedText source="Operator Logs" /></h2>
        <select value={level} onChange={event => setLevel(event.target.value)} style={selectStyle} aria-label={translateInterface("Log level")} {...interfaceTextProps("Log level")}>
          <option value=""><LocalizedText source="All levels" /></option>
          <option value="info"><LocalizedText source="Info" /></option>
          <option value="warn"><LocalizedText source="Warnings" /></option>
          <option value="error"><LocalizedText source="Errors" /></option>
        </select>
        <select value={kind} onChange={event => setKind(event.target.value)} style={selectStyle} aria-label={translateInterface("Log kind")} {...interfaceTextProps("Log kind")}>
          <option value=""><LocalizedText source="All kinds" /></option>
          <option value="rtorrent_log"><LocalizedText source="rTorrent" /></option>
          <option value="rtorrent_log_ingest_error"><LocalizedText source="rTorrent log import errors" /></option>
          <option value="rtorrent_log_ingest_recovered"><LocalizedText source="rTorrent log import recovered" /></option>
          <option value="rtorrent_sync_error"><LocalizedText source="Sync errors" /></option>
          <option value="rtorrent_sync_recovered"><LocalizedText source="Sync recovered" /></option>
          <option value="rtorrent_stats_error"><LocalizedText source="Transfer stats errors" /></option>
          <option value="rtorrent_stats_recovered"><LocalizedText source="Transfer stats recovery" /></option>
          <option value="rtorrent_user_agent_error"><LocalizedText source="User-agent errors" /></option>
          <option value="torrent_added"><LocalizedText source="Torrent added" /></option>
          <option value="torrent_removed"><LocalizedText source="Torrent removed" /></option>
          <option value="torrent_updated"><LocalizedText source="Torrent updated" /></option>
          <option value="categories_updated"><LocalizedText source="Categories" /></option>
          <option value="tags_updated"><LocalizedText source="Tags" /></option>
          <option value="saved_views_updated"><LocalizedText source="Saved views" /></option>
          <option value="ratio_groups_updated"><LocalizedText source="Ratio groups" /></option>
          <option value="rss_rules_updated"><LocalizedText source="RSS rules" /></option>
          <option value="workflows_updated"><LocalizedText source="Workflows" /></option>
          <option value="workflow_runs_updated"><LocalizedText source="Workflow run events" /></option>
          <option value="settings_changed"><LocalizedText source="Settings" /></option>
          <option value="admin_restart_requested"><LocalizedText source="Restart requests" /></option>
          <option value="sidecar_started"><LocalizedText source="Startup" /></option>
        </select>
        <button type="button" onClick={() => query.refetch()} disabled={query.isFetching} style={buttonStyle}>
          {query.isFetching ? <LocalizedText source="Refreshing..." /> : <LocalizedText source="Refresh" />}
        </button>
        <button
          type="button"
          onClick={() => setLastKnownId(newestId || undefined)}
          disabled={!newestId || query.isFetching}
          style={buttonStyle}
        >
          <LocalizedText source="Newer" />
        </button>
        {lastKnownId && (
          <button type="button" onClick={() => setLastKnownId(undefined)} style={buttonStyle}>
            <LocalizedText source="Go to newest logs" /></button>
        )}
      </div>

      {query.isLoading && <div role="status" aria-label={translateInterface("Loading logs")} {...interfaceTextProps("Loading logs")}><SkeletonRows /></div>}
      {query.error && <Notice><LocalizedText source="Logs unavailable" /></Notice>}
      {!query.isLoading && !query.error && logs.length === 0 && (
        <EmptyState><LocalizedText source="No log events matched the current filters." /></EmptyState>
      )}
      <div style={{ display: 'grid', gap: 8, maxWidth: 1040 }}>
        {logs.map(event => (
          <LogRow key={`${event.event_id ?? event.occurred_at}-${event.kind}`} event={event} />
        ))}
      </div>
    </section>
  )
}

function LogRow({ event }: { event: AppLogEvent }) {
  const level = normalizeLevel(event.level)
  const levelLabel = level === 'warn' ? 'Warnings' : level === 'error' ? 'Error' : 'Info'
  const color = level === 'error' ? 'var(--danger)' : level === 'warn' ? 'var(--warning)' : 'var(--accent)'
  const payload = parsePayload(event.payload)
  const knownEventKind = Object.hasOwn(SERVER_EVENT_KIND_LABELS, event.kind)
  return (
    <article style={{
      display: 'grid',
      gridTemplateColumns: '86px minmax(120px, 250px) 1fr',
      gap: 10,
      alignItems: 'start',
      border: `1px solid color-mix(in srgb, ${color} 40%, var(--border))`,
      borderRadius: 7,
      background: level === 'info' ? 'var(--surface)' : `color-mix(in srgb, ${color} 7%, var(--surface))`,
      padding: '9px 11px',
      minWidth: 0,
    }}>
      <div style={{ display: 'grid', gap: 4 }}>
        <span style={{
          width: 'fit-content',
          color,
          border: `1px solid color-mix(in srgb, ${color} 55%, var(--border))`,
          borderRadius: 999,
          padding: '1px 7px',
          fontSize: 11,
          fontWeight: 800,
          textTransform: 'uppercase',
        }}>
          <LocalizedText source={levelLabel} />
        </span>
        <span style={{ color: 'var(--faint)', fontSize: 11 }}><bdi dir="ltr" lang="und">{event.event_id ?? '-'}</bdi></span>
      </div>
      <div style={{ minWidth: 0 }}>
        <div
          style={{ color: 'var(--text)', fontWeight: 800, fontSize: 12, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
          title={knownEventKind ? undefined : event.kind}
          dir={knownEventKind ? undefined : 'auto'}
          lang={knownEventKind ? undefined : 'und'}
        >
          {knownEventKind
            ? <LocalizedServerEventKind kind={event.kind} />
            : <bdi dir="auto" lang="und">{event.kind}</bdi>}
        </div>
        <div style={{ color: 'var(--faint)', fontSize: 11 }}>
          {event.occurred_at
            ? <bdi dir="auto" lang={getRegionalLocale() ?? 'und'}>{formatTime(event.occurred_at)}</bdi>
            : <LocalizedText source="Unknown" />}
        </div>
      </div>
      <div style={{ minWidth: 0 }}>
        <div style={{ color: 'var(--text)', fontSize: 12, lineHeight: 1.4, overflowWrap: 'anywhere' }}>
          {event.message
            ? <bdi dir="auto" lang="und"><LocalizedServerMessage message={event.message} kind={event.kind} /></bdi>
            : knownEventKind
              ? <LocalizedServerEventKind kind={event.kind} />
              : <bdi dir="auto" lang="und">{event.kind}</bdi>}
        </div>
        {payload && (
          <div style={{
            marginTop: 5,
            color: 'var(--faint)',
            fontFamily: 'monospace',
            fontSize: 11,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }} title={payload} dir="auto" lang="und">
            <bdi dir="auto" lang="und">{payload}</bdi>
          </div>
        )}
      </div>
    </article>
  )
}

function normalizeLevel(level: string): 'info' | 'warn' | 'error' {
  const value = level.toLowerCase()
  if (value === 'error' || value === 'critical') return 'error'
  if (value === 'warn' || value === 'warning') return 'warn'
  return 'info'
}

function formatTime(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleString(getRegionalLocale())
}

function parsePayload(payload: string): string | null {
  try {
    const value = JSON.parse(payload) as Record<string, unknown>
    const parts = ['component', 'operation', 'result', 'source', 'torrent', 'job_id', 'tracker', 'error']
      .flatMap(key => value[key] ? [`${key}=${String(value[key])}`] : [])
    return parts.length > 0 ? parts.join(' ') : null
  } catch {
    return null
  }
}

function SkeletonRows() {
  return (
    <div style={{ display: 'grid', gap: 8, maxWidth: 1040 }}>
      {Array.from({ length: 5 }).map((_, index) => (
        <div key={index} style={{
          height: 58,
          borderRadius: 7,
          border: '1px solid var(--border)',
          background: 'linear-gradient(90deg, var(--surface), var(--surface-2), var(--surface))',
        }} />
      ))}
    </div>
  )
}

function EmptyState({ children }: { children: React.ReactNode }) {
  return (
    <div role="alert" aria-live="assertive" style={{
      color: 'var(--faint)',
      fontSize: 12,
      border: '1px dashed var(--border-strong)',
      borderRadius: 7,
      background: 'color-mix(in srgb, var(--surface) 72%, transparent)',
      padding: 14,
      maxWidth: 1040,
    }}>
      {children}
    </div>
  )
}

function Notice({ children }: { children: React.ReactNode }) {
  return (
    <div style={{
      color: 'var(--danger)',
      border: '1px solid color-mix(in srgb, var(--danger) 45%, var(--border))',
      background: 'color-mix(in srgb, var(--danger) 8%, var(--surface))',
      borderRadius: 7,
      padding: 10,
      fontSize: 12,
      maxWidth: 1040,
    }}>
      {children}
    </div>
  )
}

const selectStyle: React.CSSProperties = {
  background: 'var(--surface)',
  border: '1px solid var(--border-strong)',
  borderRadius: 5,
  color: 'var(--text)',
  padding: '5px 8px',
  fontSize: 12,
}

const buttonStyle: React.CSSProperties = {
  background: 'none',
  border: '1px solid var(--border-strong)',
  borderRadius: 5,
  color: 'var(--muted)',
  padding: '5px 9px',
  fontSize: 12,
  cursor: 'pointer',
}
