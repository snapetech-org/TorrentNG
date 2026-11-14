import { LocalizedText } from '../lib/LocalizedText'
import { interfaceTextProps, translateInterface } from '../lib/interfaceLocale'
import { getRegionalLocale } from '../lib/localeFormat'
import { useQuery } from '@tanstack/react-query'
import { api } from '../api/client'
import { maskAnnounceUrl, TrackerUrl } from '../lib/maskUrl'

function hostLabel(url: string): string {
  try {
    return new URL(url).host || maskAnnounceUrl(url)
  } catch {
    return maskAnnounceUrl(url)
  }
}

function fmtDate(ts: number): string {
  return new Date(ts * 1000).toLocaleString(getRegionalLocale())
}

interface Props {
  /** Jump to the torrent list filtered to this tracker. Errored trackers are
   * otherwise a dead end: this panel can tell you which trackers have
   * announce failures, but without this there's no way to find the affected
   * torrents. The per-torrent STATE "Tracker Errors" filter is separate from
   * terminal torrent errors. */
  onSelectTracker?: (tracker: string) => void
}

export function TrackerHealthPanel({ onSelectTracker }: Props) {
  const { data, isLoading, isFetching, error, refetch } = useQuery({
    queryKey: ['tracker-health'],
    queryFn: api.trackerHealth,
    staleTime: 5_000,
    refetchInterval: 10_000,
  })
  const totals = data?.trackers.reduce((acc, tracker) => {
    acc.torrents += tracker.torrent_count
    acc.active += tracker.active_count
    acc.errors += tracker.error_count
    return acc
  }, { torrents: 0, active: 0, errors: 0 })

  return (
    <section aria-labelledby="tracker-health-title" style={{ padding: '18px 24px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
        <h2 id="tracker-health-title" style={{ margin: 0, fontSize: 14, fontWeight: 600, color: 'var(--text)', flex: 1 }}>
          <LocalizedText source="Tracker Health" /></h2>
        <button
          type="button"
          onClick={() => refetch()}
          disabled={isFetching}
          style={{
            background: 'none', border: '1px solid var(--border-strong)', borderRadius: 5,
            color: 'var(--muted)', padding: '4px 9px', fontSize: 12,
            cursor: isFetching ? 'not-allowed' : 'pointer', opacity: isFetching ? 0.55 : 1,
          }}
        >
          {isFetching ? <LocalizedText source="Refreshing…" /> : <LocalizedText source="Refresh" />}
        </button>
      </div>

      {isLoading && <div role="status" aria-label={translateInterface("Loading tracker health")} {...interfaceTextProps("Loading tracker health")}><TrackerSkeleton /></div>}
      {error && <Notice><LocalizedText source="Tracker health unavailable" /></Notice>}
      {data && data.trackers.length === 0 && (
        <div role="status" style={{
          color: 'var(--faint)', fontSize: 12, border: '1px dashed var(--border-strong)',
          borderRadius: 7, background: 'color-mix(in srgb, var(--surface) 72%, transparent)', padding: 14,
        }}><LocalizedText source="No tracker data cached yet" /></div>
      )}
      {totals && (
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', marginBottom: 12 }}>
          <Summary label="Trackers" value={data?.trackers.length ?? 0} tone="neutral" />
          <Summary label="Torrents" value={totals.torrents} tone="neutral" />
          <Summary label="Active" value={totals.active} tone="ok" />
          <Summary label="Errors" value={totals.errors} tone={totals.errors > 0 ? 'warn' : 'ok'} />
        </div>
      )}

      <div style={{ display: 'grid', gap: 8, maxWidth: 980, overflowX: 'auto', paddingBottom: 2 }}>
        {data?.trackers.map(tracker => {
          const errorRatio = tracker.torrent_count > 0
            ? tracker.error_count / tracker.torrent_count
            : 0
          const color = errorRatio >= 0.5 ? 'var(--danger)' : errorRatio > 0 ? 'var(--warning)' : 'var(--success)'
          return (
            <div
              key={tracker.tracker}
              className="tng-card tng-tracker-row"
              data-tone={errorRatio >= 0.5 ? 'error' : errorRatio > 0 ? 'warn' : 'ok'}
              role="group"
              aria-label={translateInterface('Tracker status for {host}', { host: `\u2068${hostLabel(tracker.tracker)}\u2069` })}
              {...interfaceTextProps('Tracker status for {host}')}
              style={{
                display: 'grid',
                gridTemplateColumns: 'minmax(220px, 1fr) 90px 90px 90px 120px',
                minWidth: 680,
                gap: 12,
                alignItems: 'center',
                cursor: undefined,
                border: `1px solid color-mix(in srgb, ${color} 45%, var(--border))`,
                borderRadius: 6,
                padding: '9px 12px',
                background: errorRatio > 0 ? 'color-mix(in srgb, var(--warning) 7%, var(--surface))' : 'var(--surface)',
                fontSize: 12,
              }}
            >
              <div style={{ minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                  <span aria-hidden="true" style={{ width: 7, height: 7, borderRadius: '50%', background: color, flexShrink: 0 }} />
                  <span style={{
                    color: 'var(--text)', fontWeight: 700, overflow: 'hidden',
                    textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  }} title={maskAnnounceUrl(tracker.tracker)} dir="ltr" lang="und">
                    <bdi dir="ltr" lang="und">{hostLabel(tracker.tracker)}</bdi>
                  </span>
                  {onSelectTracker && (
                    <button
                      type="button"
                      aria-label={translateInterface('View torrents on {host}', { host: `\u2068${hostLabel(tracker.tracker)}\u2069` })}
                      {...interfaceTextProps('View torrents on {host}')}
                      onClick={e => { e.stopPropagation(); onSelectTracker(tracker.tracker) }}
                      style={{
                        border: 0, background: 'none', padding: 0,
                        color: 'var(--accent)', fontSize: 11, flexShrink: 0,
                        cursor: 'pointer',
                      }}
                    >
                      <LocalizedText source="View torrents →" /></button>
                  )}
                </div>
                <div style={{
                    color: 'var(--faint)', overflow: 'hidden',
                    textOverflow: 'ellipsis', whiteSpace: 'nowrap', marginTop: 2,
                  }}
                >
                  <TrackerUrl url={tracker.tracker} />
                </div>
              </div>

              <Metric label="Torrents" value={tracker.torrent_count} color="var(--muted)" />
              <Metric label="Active" value={tracker.active_count} color="var(--accent)" />
              <Metric label="Errors" value={tracker.error_count} color={color} />
              <div style={{ color: 'var(--faint)', textAlign: 'right' }}>
                <div><LocalizedText source="Seed count: {count}" parameters={{ count: tracker.seed_count.toLocaleString(getRegionalLocale()) }} /></div>
                <div><LocalizedText source="Peer count: {count}" parameters={{ count: tracker.peer_count.toLocaleString(getRegionalLocale()) }} /></div>
                <div style={{ color: 'var(--faint)', marginTop: 2 }}>
                  {tracker.last_updated
                    ? <bdi dir="auto" lang={getRegionalLocale() ?? 'und'}>{fmtDate(tracker.last_updated)}</bdi>
                    : <LocalizedText source="Never" />}
                </div>
              </div>
              <span aria-hidden="true" style={{
                gridColumn: '1 / -1',
                height: 3,
                borderRadius: 999,
                overflow: 'hidden',
                background: 'color-mix(in srgb, var(--border-strong) 48%, transparent)',
              }}>
                <span style={{
                  display: 'block',
                  width: `${Math.min(100, Math.max(4, errorRatio > 0 ? errorRatio * 100 : (tracker.active_count / Math.max(1, tracker.torrent_count)) * 100))}%`,
                  height: '100%',
                  background: color,
                }} />
              </span>
            </div>
          )
        })}
      </div>
    </section>
  )
}

function TrackerSkeleton() {
  return (
    <div style={{ display: 'grid', gap: 8, maxWidth: 980 }}>
      {Array.from({ length: 3 }).map((_, index) => (
        <div key={index} style={{
          border: '1px solid var(--border)', borderRadius: 7, background: 'var(--surface)',
          padding: '10px 12px', display: 'grid', gap: 7,
        }}>
          <span className="tng-skeleton" style={{ width: '45%', height: 12 }} />
          <span className="tng-skeleton" style={{ width: '85%', height: 10 }} />
          <span className="tng-skeleton" style={{ width: '62%', height: 18 }} />
        </div>
      ))}
    </div>
  )
}

function Notice({ children }: { children: React.ReactNode }) {
  return (
    <div role="alert" aria-live="assertive" style={{
      color: 'var(--danger)', background: 'color-mix(in srgb, var(--danger) 9%, var(--surface))',
      border: '1px solid color-mix(in srgb, var(--danger) 45%, var(--border))',
      borderRadius: 6, padding: '8px 9px', fontSize: 12, marginBottom: 10,
    }}>{children}</div>
  )
}

function Summary({ label, value, tone }: { label: string; value: number; tone: 'ok' | 'warn' | 'neutral' }) {
  const color = tone === 'ok' ? 'var(--success)' : tone === 'warn' ? 'var(--warning)' : 'var(--muted)'
  return (
    <span className="tng-metric-tile" style={{
      display: 'inline-flex', alignItems: 'center', gap: 6,
      border: '1px solid var(--border)', borderRadius: 6,
      background: 'var(--surface)', padding: '5px 8px', fontSize: 12,
    }}>
      <span style={{ color: 'var(--faint)' }}><LocalizedText source={label} /></span>
      <span style={{ color, fontWeight: 800, fontVariantNumeric: 'tabular-nums' }}><bdi dir="ltr" lang={getRegionalLocale() ?? 'und'}>{value.toLocaleString(getRegionalLocale())}</bdi></span>
    </span>
  )
}

function Metric({ label, value, color }: { label: string; value: number; color: string }) {
  return (
    <div>
      <div style={{ color, fontWeight: 700, fontSize: 15 }}><bdi dir="ltr" lang={getRegionalLocale() ?? 'und'}>{value.toLocaleString(getRegionalLocale())}</bdi></div>
      <div style={{ color: 'var(--faint)', fontSize: 11 }}><LocalizedText source={label} /></div>
    </div>
  )
}
