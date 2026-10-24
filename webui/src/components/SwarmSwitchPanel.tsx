import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, type SwarmComparison, type SwarmVariant } from '../api/client'
import { LocalizedText, LocalizedErrorText } from '../lib/LocalizedText'
import { useInterfaceLanguage } from '../lib/interfaceLocale'
import { formatRegionalNumber } from '../lib/localeFormat'

const variantLabels: Record<SwarmVariant, string> = {
  remove_private: 'Remove private flag',
  private_zero: 'Set private flag to zero',
  remove_private_and_source: 'Remove private flag and tracker source',
  private_zero_without_source: 'Zero private flag and remove tracker source',
  supplied: 'Use a public .torrent file',
}
const variants = Object.entries(variantLabels) as [SwarmVariant, string][]
const button: React.CSSProperties = { background: 'var(--surface-2)', color: 'var(--text)', border: '1px solid var(--border-strong)', borderRadius: 5, padding: '7px 10px', cursor: 'pointer' }
const count = (n: number | null | undefined): ReactNode => n == null ? <LocalizedText source="Unknown" /> : formatRegionalNumber(n)
const time = (n: number | null | undefined, locale: string): ReactNode => {
  if (n == null) return <LocalizedText source="Never observed" />
  const date = new Date(n * 1000)
  return <time dateTime={date.toISOString()}>{date.toLocaleString(locale)}</time>
}

function MetricList({ items }: { items: { label: string; value: ReactNode }[] }) {
  const { t, textProps } = useInterfaceLanguage()
  return <dl style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 190px), 1fr))', gap: '8px 16px', margin: 0 }}>
    {items.map(item => <div key={item.label} style={{ minWidth: 0 }}>
      <dt {...textProps(item.label)} style={{ color: 'var(--muted)', fontSize: 12 }}>{t(item.label)}</dt>
      <dd style={{ margin: '2px 0 0', fontVariantNumeric: 'tabular-nums' }}>{item.value}</dd>
    </div>)}
  </dl>
}

function HashRecord({ label, value }: { label: string; value: string }) {
  return <div style={{ minWidth: 0 }}>
    <dt style={{ color: 'var(--muted)', fontSize: 12 }}><LocalizedText source={label} /></dt>
    <dd style={{ margin: '2px 0 0', minWidth: 0 }}><code dir="ltr" style={{ display: 'block', overflowWrap: 'anywhere', fontSize: 11 }}>{value}</code></dd>
  </div>
}

function HashList({ items }: { items: { label: string; value: string }[] }) {
  return <dl style={{ display: 'grid', gap: 8, margin: 0 }}>{items.map(item => <HashRecord key={item.label} {...item} />)}</dl>
}

export function SwarmSwitchPanel({ hash, showUnsupported = false, onRemoved }: { hash: string; showUnsupported?: boolean; onRemoved?: () => void }) {
  const qc = useQueryClient()
  const { language } = useInterfaceLanguage()
  const id = useId()
  const titleId = `${id}-title`
  const contentId = `${id}-content`
  const comparisonId = `${id}-comparison`
  const privateTitleId = `${id}-private-title`
  const publicTitleId = `${id}-public-title`
  const fileInputId = `${id}-public-torrent`
  const fileHelpId = `${id}-public-torrent-help`
  const confirmTitleId = `${id}-delete-private-title`
  const confirmHelpId = `${id}-delete-private-help`
  const { data: engine } = useQuery({ queryKey: ['engine'], queryFn: api.engine, staleTime: 5000 })
  const supported = engine?.backend?.capabilities?.supports_swarm_switch === true
  const { data: status, error: statusError } = useQuery({ queryKey: ['swarm-switch', hash], queryFn: () => api.swarm.status(hash), enabled: supported, refetchInterval: 5000 })
  const [expanded, setExpanded] = useState(showUnsupported)
  const [variant, setVariant] = useState<SwarmVariant>('remove_private')
  const [fileData, setFileData] = useState<string>()
  const [fileName, setFileName] = useState<string>()
  const [comparison, setComparison] = useState<SwarmComparison>()
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string>()
  const [notice, setNotice] = useState<string>()
  const [confirmForget, setConfirmForget] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer) }, [])
  const generation = useRef(0)
  useEffect(() => () => { generation.current++ }, [hash])

  if (!supported) return showUnsupported ? <p><LocalizedText source="Swarm Switch requires a TorrentNG engine that supports this feature." /></p> : null
  const linked = status?.can_restore === true
  const needsRecovery = linked && status?.phase !== 'public'
  const independent = status?.phase === 'public_independent'
  const sourceHash = status?.original_hash ?? hash
  const observedAge = comparison ? now / 1000 - comparison.public.observed_at : Infinity
  const freshComparison = comparison && observedAge < 60

  async function perform(action: () => Promise<void>, progress?: string, success?: string) {
    setBusy(true)
    setMessage(undefined)
    setNotice(progress)
    try {
      await action()
      setNotice(success)
    } catch (error) {
      setNotice(undefined)
      setMessage(error instanceof Error ? error.message : String(error))
    } finally {
      setBusy(false)
      void qc.invalidateQueries({ queryKey: ['swarm-switch'] })
      void qc.invalidateQueries({ queryKey: ['torrents'] })
      void qc.invalidateQueries({ queryKey: ['trackers'] })
    }
  }

  async function compare() {
    const current = ++generation.current
    await perform(async () => {
      const result = await api.swarm.compare(sourceHash, { variant, public_torrent_base64: fileData })
      if (generation.current === current) setComparison(result)
    }, 'Querying private tracker reports and the public DHT candidate…', 'Comparison updated. Review the exact hashes and observations below.')
  }

  async function chooseFile(file?: File) {
    setComparison(undefined)
    setFileData(undefined)
    setFileName(undefined)
    setMessage(undefined)
    setNotice(undefined)
    if (!file) return
    if (file.size > 8 * 1024 * 1024) { setMessage('Public metainfo exceeds 8 MiB.'); return }
    await perform(async () => {
      const encoded = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader()
        reader.onerror = () => reject(new Error('Could not read the public torrent file.'))
        reader.onload = () => resolve(String(reader.result).split(',')[1])
        reader.readAsDataURL(file)
      })
      setFileData(encoded)
      setFileName(file.name)
    }, 'Reading the public torrent file…', 'Public torrent file loaded. Compare its exact hash and swarm before switching.')
  }

  return <section className="tng-swarm-switch" aria-labelledby={titleId} style={{ padding: 14, borderBottom: '1px solid var(--border)', overflowWrap: 'anywhere' }}>
    <h3 style={{ margin: 0, fontSize: 14 }}>
      <button type="button" style={{ ...button, width: '100%', display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10, textAlign: 'left' }} aria-expanded={expanded} aria-controls={contentId} onClick={() => setExpanded(!expanded)}>
        <span id={titleId}><LocalizedText source="Swarm Switch" />{linked && <> · <LocalizedText source={needsRecovery ? 'Recovery required' : 'Public session'} /></>}</span>
        <span aria-hidden="true">{expanded ? '▾' : '▸'}</span>
      </button>
    </h3>
    {expanded && <div id={contentId} style={{ display: 'grid', minWidth: 0, gap: 10, paddingTop: 10 }}>
      {statusError && <p role="alert" aria-live="assertive"><LocalizedErrorText message={statusError instanceof Error ? statusError.message : String(statusError)} /></p>}
      {!status && !statusError && <p role="status" aria-live="polite"><LocalizedText source="Loading swarm status…" /></p>}
      {notice && <p role="status" aria-live="polite" aria-atomic="true" style={{ margin: 0, color: 'var(--accent)' }}><LocalizedText source={notice} /></p>}
      {independent && <p><LocalizedText source="This is an independent public torrent. There is no retained private torrent to restore." /></p>}
      {status && !independent && <>
        <p style={{ margin: 0, color: 'var(--muted)' }}><LocalizedText source={linked
          ? 'The original private torrent is retained and parked. Only the public counterpart can transfer. Restore verifies the files before private seeding resumes.'
          : 'Compare the original private swarm with one exact public candidate. TorrentNG keeps your private torrent and tracker credentials saved; only one swarm runs at a time.'} /></p>
        {!linked && <p style={{ margin: 0, color: 'var(--muted)' }}><LocalizedText source="Comparison reads the original private tracker reports and queries the public candidate on DHT. It does not announce either hash." /></p>}
        {linked && <MetricList items={[{ label: 'Public transfer', value: <>{status.active_state} · {count(status.verified_bytes)} / {count(status.total_bytes)} <LocalizedText source="bytes verified" /></> }]} />}
        {needsRecovery && <p role="alert"><LocalizedText source="The previous switch was interrupted. Restore the private swarm to recover; both identities are protected from concurrent transfers." /></p>}
        {!linked && <>
          <label htmlFor={`${id}-candidate`} style={{ display: 'grid', gap: 5 }}><LocalizedText source="Public candidate" />
            <select id={`${id}-candidate`} value={variant} disabled={busy} style={{ ...button, minWidth: 0, width: '100%', boxSizing: 'border-box' }} aria-describedby={`${id}-candidate-help`} onChange={e => { setVariant(e.target.value as SwarmVariant); setComparison(undefined); setFileData(undefined); setFileName(undefined); setMessage(undefined); setNotice(undefined) }}>
              {variants.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
            </select>
          </label>
          <p id={`${id}-candidate-help`} style={{ margin: 0, color: 'var(--muted)' }}><LocalizedText source="Different packaging can mean a different swarm. A supplied torrent must prove identical content and file paths. V1 torrents also need matching piece sizes and hashes." /></p>
          {variant === 'supplied' && <div style={{ display: 'grid', gap: 4 }}>
            <label htmlFor={fileInputId}><LocalizedText source="Public torrent file" /></label>
            <input id={fileInputId} type="file" accept=".torrent,application/x-bittorrent" disabled={busy} aria-describedby={fileHelpId} onChange={e => { void chooseFile(e.target.files?.[0]) }} style={{ maxWidth: '100%' }} />
            <small id={fileHelpId} style={{ color: 'var(--muted)' }}><LocalizedText source="Choose a known public .torrent file, up to 8 MiB. TorrentNG checks that its content matches before it can be used." /></small>
            {fileName && <small role="status" aria-live="polite"><LocalizedText source="Selected file:" /> <bdi dir="auto">{fileName}</bdi></small>}
          </div>}
        </>}
        <button type="button" style={button} disabled={busy || (!linked && variant === 'supplied' && !fileData)} onClick={() => { void compare() }}>
          <LocalizedText source={busy ? 'Working…' : comparison ? 'Refresh swarm comparison' : 'Compare swarms'} />
        </button>
        {comparison && <div id={comparisonId} role="region" aria-labelledby={`${id}-comparison-title`} style={{ display: 'grid', gap: 10, minWidth: 0, border: '1px solid var(--border)', borderRadius: 6, padding: 10 }}>
          <h4 id={`${id}-comparison-title`} style={{ margin: 0 }}><LocalizedText source="Swarm comparison" /></h4>
          <section role="group" aria-labelledby={privateTitleId} style={{ display: 'grid', gap: 8, minWidth: 0 }}>
            <h5 id={privateTitleId} style={{ margin: 0 }}><LocalizedText source="Private swarm" /></h5>
            <HashList items={[{ label: 'Private infohash', value: comparison.candidate.original_hash }]} />
            {comparison.private.length === 0
              ? <p style={{ margin: 0 }}><LocalizedText source="No private tracker observations are available." /></p>
              : <ul style={{ display: 'grid', gap: 8, listStyle: 'none', margin: 0, padding: 0 }}>
                {comparison.private.map((tracker, index) => <li key={`${tracker.tracker}-${index}`} style={{ background: 'var(--surface-2)', padding: 8, borderRadius: 5, minWidth: 0 }}>
                  <h6 style={{ margin: '0 0 6px', fontSize: 12, fontWeight: 600 }}><bdi dir="ltr">{tracker.tracker}</bdi></h6>
                  <MetricList items={[
                    { label: 'Tracker-reported seeds', value: count(tracker.seeds) },
                    { label: 'Tracker-reported leechers', value: count(tracker.leechers) },
                    { label: 'Report freshness', value: <LocalizedText source={tracker.fresh ? 'Fresh tracker scrape' : 'Last known report; fresh scrape unavailable'} /> },
                    { label: 'Observed', value: time(tracker.observed_at, language) },
                  ]} />
                </li>)}
              </ul>}
          </section>
          <section role="group" aria-labelledby={publicTitleId} style={{ display: 'grid', gap: 8, minWidth: 0 }}>
            <h5 id={publicTitleId} style={{ margin: 0 }}><LocalizedText source="Public swarm" /></h5>
            <HashList items={[
              { label: 'Candidate public infohash', value: comparison.candidate.public_hash },
              ...(comparison.candidate.public_v2_hash ? [{ label: 'Candidate public v2 infohash', value: comparison.candidate.public_v2_hash }] : []),
            ]} />
            <div style={{ background: 'var(--surface-2)', padding: 8, borderRadius: 5, minWidth: 0 }}>
              <MetricList items={[
                { label: 'Discovered peers', value: count(comparison.public.discovered_peers) },
                { label: 'Estimated seeds', value: comparison.public.seed_estimate == null ? <LocalizedText source="Unknown" /> : <>≈ {count(comparison.public.seed_estimate)}</> },
                { label: 'Estimated peers', value: comparison.public.peer_estimate == null ? <LocalizedText source="Unknown" /> : <>≈ {count(comparison.public.peer_estimate)}</> },
                { label: 'Reachable in sample', value: <>{count(comparison.public.reachable_peers)} / {count(comparison.public.sampled_peers)}</> },
                { label: 'Observed seeds in sample', value: count(comparison.public.observed_seeds) },
                { label: 'Responding DHT nodes', value: count(comparison.public.responding_nodes) },
                { label: 'Snapshot observed', value: time(comparison.public.observed_at, language) },
              ]} />
            </div>
            <small style={{ color: 'var(--muted)' }}><LocalizedText source="DHT results are a limited snapshot, not swarm totals. Estimates require DHT scrape support. Observed seeds come from peer handshakes and availability messages. Unknown does not mean zero; zero found does not prove an empty swarm." /></small>
          </section>
          <details>
            <summary style={{ cursor: 'pointer' }}><LocalizedText source="Candidate details" /></summary>
            <div style={{ paddingTop: 8 }}>
              <MetricList items={[
                { label: 'Candidate format', value: <LocalizedText source={variantLabels[comparison.candidate.variant]} /> },
                { label: 'Piece length', value: count(comparison.candidate.piece_length) },
                { label: 'Piece count', value: count(comparison.candidate.piece_count) },
                { label: 'File count', value: count(comparison.candidate.files) },
                { label: 'Payload size in bytes', value: count(comparison.candidate.total_bytes) },
              ]} />
            </div>
          </details>
          {observedAge >= 60 && <p role="status" aria-live="polite" style={{ margin: 0 }}><LocalizedText source="This comparison is over a minute old. Refresh it before switching swarms." /></p>}
        </div>}
        {!linked && <>
          {!status.dht_enabled && <p style={{ margin: 0 }}><LocalizedText source="Enable DHT in Settings → Backend before switching. Comparison is still available." /></p>}
          {!status.can_switch && status.active_state === 'metadata_pending' && <p style={{ margin: 0 }}><LocalizedText source="Wait for torrent metadata to finish loading before switching." /></p>}
          <p style={{ margin: 0, color: 'var(--muted)' }}><LocalizedText source="Public mode shares this counterpart’s info metadata and your peer address. Tracker URLs and outer comments are removed. Restoring stops public participation; it cannot retract already shared metadata." /></p>
          <button type="button" style={{ ...button, borderColor: 'var(--accent)' }} aria-describedby={`${id}-activation-help`} disabled={busy || !freshComparison || !status.dht_enabled || !status.can_switch} onClick={() => { void perform(async () => {
            if (!comparison) return
            await api.swarm.public(sourceHash, { variant, public_torrent_base64: fileData, expected_public_hash: comparison.candidate.public_hash })
            setComparison(undefined)
          }, 'Switching to the public swarm…', 'Public swarm selected. TorrentNG is checking the existing files before public transfer starts.') }}><LocalizedText source={busy ? 'Working…' : 'Use public swarm'} /></button>
          <small id={`${id}-activation-help`} style={{ color: 'var(--muted)' }}><LocalizedText source="Switching requires a fresh comparison, an exact candidate match, DHT enabled, and complete torrent metadata." /></small>
        </>}
        {linked && <>
          {!needsRecovery && !freshComparison && <small><LocalizedText source="Compare swarms before restoring. Recovery remains available if a switch was interrupted." /></small>}
          <button type="button" style={{ ...button, borderColor: 'var(--accent)' }} disabled={busy || (!needsRecovery && !freshComparison)} onClick={() => { void perform(async () => { await api.swarm.restore(sourceHash); setComparison(undefined); if (hash !== sourceHash) onRemoved?.() }, 'Restoring the private swarm…', 'Private swarm restored. TorrentNG is checking the retained files before private transfers resume.') }}><LocalizedText source={busy ? 'Working…' : 'Restore private swarm'} /></button>
          <button type="button" style={button} aria-expanded={confirmForget} aria-controls={`${id}-delete-confirmation`} disabled={busy} onClick={() => setConfirmForget(!confirmForget)}><LocalizedText source={confirmForget ? 'Keep retained private copy' : 'Delete retained private copy…'} /></button>
          {confirmForget && <div id={`${id}-delete-confirmation`} role="group" aria-labelledby={confirmTitleId} aria-describedby={confirmHelpId} style={{ display: 'grid', gap: 8, border: '1px solid var(--danger)', borderRadius: 5, padding: 10 }}>
            <h5 id={confirmTitleId} style={{ margin: 0 }}><LocalizedText source="Delete retained private copy?" /></h5>
            <p id={confirmHelpId} style={{ margin: 0 }}><LocalizedText source="The public torrent and payload files will remain. Deleting the private copy permanently removes the restore action for this session." /></p>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
              <button type="button" style={button} disabled={busy} onClick={() => setConfirmForget(false)}><LocalizedText source="Cancel" /></button>
              <button type="button" style={{ ...button, borderColor: 'var(--danger)', color: 'var(--danger)' }} disabled={busy} onClick={() => { void perform(async () => {
                await api.torrents.remove(sourceHash, false)
                setConfirmForget(false); setComparison(undefined)
                if (hash === sourceHash) onRemoved?.()
              }, 'Deleting the retained private torrent…', 'Retained private copy deleted. The public torrent and payload files remain; private restore is no longer available.') }}><LocalizedText source={busy ? 'Working…' : 'Delete private copy, keep files'} /></button>
            </div>
          </div>}
        </>}
      </>}
      {message && <p role="alert" aria-live="assertive" style={{ color: 'var(--danger)' }}><LocalizedErrorText message={message} /></p>}
    </div>}
  </section>
}
