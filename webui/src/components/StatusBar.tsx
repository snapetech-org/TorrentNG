import { LocalizedText } from '../lib/LocalizedText'
import { interfaceTextProps, useInterfaceLanguage } from '../lib/interfaceLocale'
import { formatRegionalNumber, getRegionalLocale } from '../lib/localeFormat'
import { useState } from 'react'
import type { LiveStats, StorageRoot } from '../api/client'

interface Props {
  loaded: number
  total: number
  selected: number
  stats: LiveStats
  backendType: string
  backendStatus: string
  cached?: number
  storage?: StorageRoot
  utpLabel?: string
  utpTitle?: string
  utpEnabled?: boolean
  togglingFeature?: 'dht' | 'pex' | null
  featureError?: InterfaceMessage | null
  actionMessage?: InterfaceMessage | null
  actionTone?: 'ok' | 'error'
  onToggleDht?: () => void
  onTogglePex?: () => void
}

type InterfaceMessage = { source: string; parameters?: Record<string, string | number> }

function fmtSpeed(bps: number): string {
  if (!bps) return formatRegionalNumber(0) + ' B/s'
  if (bps >= 1e9) return formatRegionalNumber(bps / 1e9, { maximumFractionDigits: 1 }) + ' GB/s'
  if (bps >= 1e6) return formatRegionalNumber(bps / 1e6, { maximumFractionDigits: 1 }) + ' MB/s'
  if (bps >= 1e3) return formatRegionalNumber(bps / 1e3, { maximumFractionDigits: 0 }) + ' KB/s'
  return formatRegionalNumber(bps) + ' B/s'
}

function fmtBytes(bytes?: number): string {
  if (bytes === undefined || !Number.isFinite(bytes)) return 'unknown'
  if (bytes >= 1e12) return formatRegionalNumber(bytes / 1e12, { maximumFractionDigits: 1 }) + ' TB'
  if (bytes >= 1e9) return formatRegionalNumber(bytes / 1e9, { maximumFractionDigits: 1 }) + ' GB'
  if (bytes >= 1e6) return formatRegionalNumber(bytes / 1e6, { maximumFractionDigits: 1 }) + ' MB'
  return formatRegionalNumber(Math.max(0, bytes)) + ' B'
}

function tone(value?: string): { color: string; bg: string; border: string } {
  if (value === 'open' || value === 'on') return themedTone('var(--success)')
  if (value === 'listening' || value === 'unknown') return themedTone('var(--warning)')
  if (value === 'closed' || value === 'off') return themedTone('var(--danger)')
  return themedTone('var(--muted)')
}

function themedTone(color: string): { color: string; bg: string; border: string } {
  return {
    color,
    bg: `color-mix(in srgb, ${color} 12%, transparent)`,
    border: `color-mix(in srgb, ${color} 38%, var(--border))`,
  }
}

function Badge({
  label, value, valueSuffix, title, state, onClick, disabled, meter,
}: {
  label: string
  value: string
  valueSuffix?: string
  title?: string
  state?: string
  onClick?: () => void
  disabled?: boolean
  meter?: number
}) {
  const { t, textProps } = useInterfaceLanguage()
  const colors = tone(state)
  const Element = onClick && !disabled ? 'button' : 'span'
  const handleClick = disabled ? undefined : onClick
  const localizedLabel = t(label)
  const stateLabels = new Set(['connected', 'connecting', 'starting', 'disconnected', 'unknown', 'on', 'off', 'open', 'closed', 'listening'])
  const localizedValue = stateLabels.has(value) ? t(value[0].toUpperCase() + value.slice(1)) : value
  return (
    <Element
      className="tng-status-badge"
      data-state={state ?? 'idle'}
      type={Element === 'button' ? 'button' : undefined}
      role={onClick ? 'switch' : undefined}
      aria-checked={onClick ? state === 'on' : undefined}
      aria-disabled={disabled || undefined}
      aria-label={onClick ? t('{label}: {value}. Activate to toggle', { label: localizedLabel, value: localizedValue }) : undefined}
      title={title}
      onClick={handleClick}
      style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 5,
      minHeight: 22,
      padding: '0 8px',
      border: `1px solid ${colors.border}`,
      borderRadius: 6,
      background: colors.bg,
      color: colors.color,
      whiteSpace: 'nowrap',
      fontVariantNumeric: 'tabular-nums',
      cursor: onClick && !disabled ? 'pointer' : 'default',
      font: 'inherit',
      opacity: disabled ? 0.7 : 1,
    }}>
      <span aria-hidden="true" style={{
        width: 6, height: 6, borderRadius: 999, background: colors.color,
        boxShadow: `0 0 0 2px color-mix(in srgb, ${colors.color} 12%, transparent)`,
        flex: '0 0 auto',
      }} />
      <span style={{ color: 'var(--faint)' }} {...textProps(label)}>{localizedLabel}</span>
      <span {...(stateLabels.has(value) ? textProps(value[0].toUpperCase() + value.slice(1)) : {})}>{localizedValue}{valueSuffix}</span>
      {meter !== undefined && (
        <span aria-hidden="true" style={{
          position: 'absolute', left: 6, right: 6, bottom: 3, height: 2,
          borderRadius: 999, overflow: 'hidden',
          background: 'color-mix(in srgb, var(--border-strong) 54%, transparent)',
        }}>
          <span style={{
            display: 'block', width: `${Math.min(100, Math.max(0, meter))}%`, height: '100%',
            background: colors.color,
          }} />
        </span>
      )}
    </Element>
  )
}

function Notice({ tone, message, children }: { tone: 'ok' | 'error'; message?: InterfaceMessage; children?: React.ReactNode }) {
  return (
    <span role={tone === 'error' ? 'alert' : 'status'} aria-live={tone === 'error' ? 'assertive' : 'polite'} style={{
      display: 'inline-flex', alignItems: 'center', gap: 5, minHeight: 22,
      border: '1px solid ' + (tone === 'error' ? 'color-mix(in srgb, var(--danger) 45%, var(--border))' : 'color-mix(in srgb, var(--success) 38%, var(--border))'),
      background: tone === 'error' ? 'color-mix(in srgb, var(--danger) 12%, transparent)' : 'color-mix(in srgb, var(--success) 10%, transparent)',
      color: tone === 'error' ? 'var(--danger)' : 'var(--success)',
      borderRadius: 6,
      padding: '0 8px',
      whiteSpace: 'nowrap',
    }}>
      <span>{tone === 'error' ? '!' : '✓'}</span>
      <span>{message ? <LocalizedText source={message.source} parameters={message.parameters} /> : children}</span>
    </span>
  )
}

export function StatusBar({
  loaded, total, selected, stats, backendType, backendStatus, cached, storage, togglingFeature, onToggleDht, onTogglePex,
  featureError, actionMessage, actionTone = 'ok', utpLabel, utpTitle, utpEnabled,
}: Props) {
  const { t } = useInterfaceLanguage()
  const [mobileExpanded, setMobileExpanded] = useState(false)
  const connected = backendStatus === 'connected'
  const rendered = Math.min(loaded, total)
  const storageLabel = storage?.ok
    ? `${fmtBytes(storage.used_bytes)} / ${fmtBytes(storage.total_bytes)}`
    : 'unknown'
  const storagePct = storage?.ok && storage.total_bytes > 0
    ? (storage.used_bytes / storage.total_bytes) * 100
    : undefined
  const storageTitle = storage?.ok
    ? t('Free space on {path}: {size}', { size: fmtBytes(storage.available_bytes), path: storage.path })
    : storage?.error ?? t('Storage status unavailable')

  return (
    <footer
      className="tng-statusbar"
      aria-label={t("Application status")}
      data-connected={connected ? 'true' : 'false'}
      data-selected={selected > 0 ? 'true' : 'false'}
      data-mobile-expanded={mobileExpanded ? 'true' : 'false'}
      style={{
      minHeight: 38, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 12,
      padding: '0 12px', background: 'var(--bg)', borderTop: '1px solid var(--border-strong)',
      color: 'var(--faint)', fontSize: 11, overflowX: 'auto',
    }} {...interfaceTextProps("Application status")}>
      <div className="tng-statusbar-summary" style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 'max-content' }}>
        <Badge label={backendType === 'torrentng' ? 'TorrentNG client' : backendType === 'backend' ? 'Backend' : backendType} value={connected ? 'connected' : 'disconnected'} state={connected ? 'on' : 'closed'} />
      </div>
      <div className="tng-statusbar-counts" style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 'max-content' }}>
        <Badge label="Torrents" value={total.toLocaleString(getRegionalLocale())} state="idle" />
        {rendered !== total && <Badge label="Rendered" value={rendered.toLocaleString(getRegionalLocale())} state="idle" />}
        {cached !== undefined && <Badge label="Cached" value={cached.toLocaleString(getRegionalLocale())} state="idle" />}
        {selected > 0 && <Badge label="Selected" value={selected.toLocaleString(getRegionalLocale())} state="on" />}
        {featureError && <Notice tone="error" message={featureError} />}
        {actionMessage && <Notice tone={actionTone} message={actionMessage} />}
      </div>
      <nav className="tng-status-support" aria-label={t("Support TorrentNG")} {...interfaceTextProps("Support TorrentNG")}>
        <span><LocalizedText source="Keep seeding" /></span>
        <a
          href="https://ko-fi.com/snapetech"
          target="_blank"
          rel="noreferrer"
          title={t("Support TorrentNG development on Ko-fi")}
         {...interfaceTextProps("Support TorrentNG development on Ko-fi")}>Ko-fi</a>
      </nav>
      <span className="tng-statusbar-spacer" style={{ flex: 1 }} />
      <div className="tng-statusbar-live" style={{ display: 'flex', alignItems: 'center', gap: 7, minWidth: 'max-content' }}>
        <Badge label="Download" value={fmtSpeed(stats.download_speed)} state={(stats.download_speed ?? 0) > 0 ? 'on' : 'unknown'} />
        <Badge label="Upload" value={fmtSpeed(stats.upload_speed)} state={(stats.upload_speed ?? 0) > 0 ? 'on' : 'unknown'} />
      </div>
      <button
        type="button"
        className="tng-statusbar-more-toggle"
        onClick={() => setMobileExpanded(v => !v)}
        aria-expanded={mobileExpanded}
        style={{
          background: 'var(--surface)', border: '1px solid var(--border-strong)', borderRadius: 5,
          color: 'var(--muted)', padding: '2px 8px', fontSize: 11, cursor: 'pointer', minWidth: 'max-content',
        }}
      >
        {mobileExpanded ? <LocalizedText source="Less ▴" /> : <LocalizedText source="More ▾" />}
      </button>
      <div className="tng-statusbar-metrics" style={{ display: 'flex', alignItems: 'center', gap: 7, minWidth: 'max-content' }}>
        <Badge label="Total downloaded" value={fmtBytes(stats.download_total)} title={t("Downloaded since the daemon last started (not a lifetime total)")} state="on" />
        <Badge label="Total uploaded" value={fmtBytes(stats.upload_total)} title={t("Uploaded since the daemon last started (not a lifetime total)")} state="on" />
        <Badge
          label="Disk"
          value={storageLabel}
          title={storageTitle}
          state={storagePct !== undefined && storagePct > 92 ? 'closed' : storagePct !== undefined && storagePct > 82 ? 'unknown' : storage?.ok ? 'on' : 'unknown'}
          meter={storagePct}
        />
        <Badge
          label="Connections"
          value={`${(stats.connections ?? 0).toLocaleString(getRegionalLocale())}${stats.pending_connections ? ` +${formatRegionalNumber(stats.pending_connections)}` : ''}`}
          title={t("Established and pending peer sockets on the incoming port")}
          state={(stats.connections ?? 0) > 0 ? 'on' : 'unknown'}
        />
        <Badge
          label="Firewall"
          value={stats.firewall ?? 'unknown'}
          valueSuffix={stats.listen_port ? ` :${stats.listen_port}` : ''}
          title={t("Incoming port listener state inferred from live TCP sockets")}
          state={stats.firewall}
        />
        <Badge
          label="DHT"
          title={t("Toggle runtime backend DHT")}
          state={stats.dht}
          disabled={togglingFeature === 'dht'}
          value={togglingFeature === 'dht' ? '...' : (stats.dht ?? 'unknown')}
          onClick={stats.dht === 'unknown' || togglingFeature ? undefined : onToggleDht}
        />
        <Badge
          label="PEX"
          title={t("Toggle runtime backend peer exchange")}
          state={stats.pex}
          disabled={togglingFeature === 'pex'}
          value={togglingFeature === 'pex' ? '...' : (stats.pex ?? 'unknown')}
          onClick={stats.pex === 'unknown' || togglingFeature ? undefined : onTogglePex}
        />
        {utpLabel && (
          <Badge
            label="uTP"
            value={utpLabel}
            title={utpTitle}
            state={utpEnabled ? 'on' : 'off'}
          />
        )}
      </div>
    </footer>
  )
}
