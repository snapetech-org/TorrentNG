import { useEffect, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, type SwarmComparison, type SwarmVariant } from '../api/client'
import { LocalizedText, LocalizedErrorText } from '../lib/LocalizedText'
import { formatRegionalNumber } from '../lib/localeFormat'

const variants: [SwarmVariant, string][] = [
  ['remove_private', 'Remove private flag'],
  ['private_zero', 'Set private flag to zero'],
  ['remove_private_and_source', 'Remove private flag and tracker source'],
  ['private_zero_without_source', 'Zero private flag and remove tracker source'],
  ['supplied', 'Use a public .torrent file'],
]
const button: React.CSSProperties = { background: 'var(--surface-2)', color: 'var(--text)', border: '1px solid var(--border-strong)', borderRadius: 5, padding: '7px 10px', cursor: 'pointer' }
const count = (n: number | null | undefined) => n == null ? 'Unknown' : formatRegionalNumber(n)
const time = (n: number | null | undefined) => n ? new Date(n * 1000).toLocaleString() : 'Never observed'

export function SwarmSwitchPanel({ hash, showUnsupported = false, onRemoved }: { hash: string; showUnsupported?: boolean; onRemoved?: () => void }) {
  const qc = useQueryClient()
  const { data: engine } = useQuery({ queryKey: ['engine'], queryFn: api.engine, staleTime: 5000 })
  const supported = engine?.backend?.capabilities?.supports_swarm_switch === true
  const { data: status, error: statusError } = useQuery({ queryKey: ['swarm-switch', hash], queryFn: () => api.swarm.status(hash), enabled: supported, refetchInterval: 5000 })
  const [expanded, setExpanded] = useState(showUnsupported)
  const [variant, setVariant] = useState<SwarmVariant>('remove_private')
  const [fileData, setFileData] = useState<string>()
  const [comparison, setComparison] = useState<SwarmComparison>()
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string>()
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

  async function perform(action: () => Promise<void>) {
    setBusy(true); setMessage(undefined)
    try { await action() } catch (error) { setMessage(error instanceof Error ? error.message : String(error)) }
    finally {
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
    })
  }

  async function chooseFile(file?: File) {
    setComparison(undefined); setFileData(undefined); setMessage(undefined)
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
    })
  }

  return <section aria-label="Swarm Switch" style={{ padding: 14, borderBottom: '1px solid var(--border)', overflowWrap: 'anywhere' }}>
    <button type="button" style={{ ...button, width: '100%', textAlign: 'left' }} aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>
      <strong><LocalizedText source="Swarm Switch" /></strong>
      {linked && <span> · <LocalizedText source={needsRecovery ? 'Recovery required' : 'Public session'} /></span>}
    </button>
    {expanded && <div style={{ display: 'grid', minWidth: 0, gap: 10, paddingTop: 10 }}>
      {statusError && <p role="alert"><LocalizedErrorText message={statusError instanceof Error ? statusError.message : String(statusError)} /></p>}
      {!status && !statusError && <p role="status"><LocalizedText source="Loading swarm status…" /></p>}
      {independent && <p><LocalizedText source="This is an independent public torrent. There is no retained private torrent to restore." /></p>}
      {status && !independent && <>
        <p style={{ margin: 0, color: 'var(--muted)' }}><LocalizedText source={linked
          ? 'The original private torrent is retained and parked. Only the public counterpart can transfer. Restore verifies the files before private seeding resumes.'
          : 'Compare a public counterpart, then use its swarm with the same files. Your private torrent and tracker credentials stay saved. Only one swarm runs at a time.'} /></p>
        {linked && <div><LocalizedText source="Public transfer" />: {status.active_state} · {count(status.verified_bytes)} / {count(status.total_bytes)} <LocalizedText source="bytes verified" /></div>}
        {needsRecovery && <p role="alert"><LocalizedText source="The previous switch was interrupted. Restore the private swarm to recover; both identities are protected from concurrent transfers." /></p>}
        {!linked && <>
          <label style={{ display: 'grid', gap: 5 }}><LocalizedText source="Public candidate" />
            <select value={variant} disabled={busy} style={{ ...button, minWidth: 0, width: '100%', boxSizing: 'border-box' }} onChange={e => { setVariant(e.target.value as SwarmVariant); setComparison(undefined); setFileData(undefined); setMessage(undefined) }}>
              {variants.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
            </select>
          </label>
          {variant === 'supplied' && <label><LocalizedText source="Public torrent file" /><input aria-label="Public torrent file" type="file" accept=".torrent,application/x-bittorrent" disabled={busy} onChange={e => { void chooseFile(e.target.files?.[0]) }} style={{ maxWidth: '100%' }} /></label>}
          <p style={{ margin: 0, color: 'var(--muted)' }}><LocalizedText source="Different packaging can mean a different swarm. A supplied torrent must prove identical content and file paths. V1 torrents also need matching piece sizes and hashes." /></p>
        </>}
        <button type="button" style={button} disabled={busy || (!linked && variant === 'supplied' && !fileData)} onClick={() => { void compare() }}>
          <LocalizedText source={busy ? 'Working…' : comparison ? 'Refresh swarm comparison' : 'Compare swarms'} />
        </button>
        {comparison && <div aria-label="Swarm comparison" style={{ display: 'grid', gap: 8 }}>
          <div><strong><LocalizedText source="Private swarm" /></strong><code style={{ display: 'block', fontSize: 10 }}>{comparison.candidate.original_hash}</code></div>
          {comparison.private.length === 0 && <p><LocalizedText source="No private tracker observations are available." /></p>}
          {comparison.private.map((tracker, index) => <div key={`${tracker.tracker}-${index}`} style={{ background: 'var(--surface-2)', padding: 8, borderRadius: 5 }}>
            <div>{tracker.tracker}</div>
            <div><LocalizedText source="Seeds" />: {count(tracker.seeds)} · <LocalizedText source="Leechers" />: {count(tracker.leechers)}</div>
            <small><LocalizedText source={tracker.fresh ? 'Fresh tracker scrape' : 'Last known report; fresh scrape unavailable'} /> · {time(tracker.observed_at)}</small>
          </div>)}
          <div><strong><LocalizedText source="Public swarm" /></strong><code style={{ display: 'block', fontSize: 10 }}>{comparison.candidate.public_hash}</code></div>
          <div style={{ background: 'var(--surface-2)', padding: 8, borderRadius: 5 }}>
            <div><LocalizedText source="Discovered peers" />: {count(comparison.public.discovered_peers)}</div>
            <div><LocalizedText source="Estimated seeds" />: {comparison.public.seed_estimate == null ? 'Unknown' : `≈ ${count(comparison.public.seed_estimate)}`}</div>
            <div><LocalizedText source="Estimated peers" />: {comparison.public.peer_estimate == null ? 'Unknown' : `≈ ${count(comparison.public.peer_estimate)}`}</div>
            <div><LocalizedText source="Reachable in sample" />: {comparison.public.reachable_peers} / {comparison.public.sampled_peers} · <LocalizedText source="Observed seeds" />: {comparison.public.observed_seeds}</div>
            <small>{comparison.public.responding_nodes} <LocalizedText source="DHT nodes responded" /> · {time(comparison.public.observed_at)}</small>
          </div>
          <small style={{ color: 'var(--muted)' }}><LocalizedText source="DHT results are a limited snapshot, not swarm totals. Estimates require DHT scrape support. Observed seeds come from peer handshakes and availability messages. Unknown does not mean zero; zero found does not prove an empty swarm." /></small>
          {observedAge >= 60 && <p role="status"><LocalizedText source="This comparison is over a minute old. Refresh it before switching swarms." /></p>}
        </div>}
        {!linked && <>
          {!status.dht_enabled && <p role="status"><LocalizedText source="Enable DHT in Settings → Backend before switching. Comparison is still available." /></p>}
          <p style={{ margin: 0, color: 'var(--muted)' }}><LocalizedText source="Public mode shares this counterpart’s info metadata and your peer address. Tracker URLs and outer comments are removed. Restoring stops public participation; it cannot retract already shared metadata." /></p>
          <button type="button" style={{ ...button, borderColor: 'var(--accent)' }} disabled={busy || !freshComparison || !status.dht_enabled || !status.can_switch} onClick={() => { void perform(async () => {
            if (!comparison) return
            await api.swarm.public(sourceHash, { variant, public_torrent_base64: fileData, expected_public_hash: comparison.candidate.public_hash })
            setComparison(undefined)
          }) }}><LocalizedText source="Use public swarm" /></button>
        </>}
        {linked && <>
          {!needsRecovery && !freshComparison && <small><LocalizedText source="Compare swarms before restoring. Recovery remains available if a switch was interrupted." /></small>}
          <button type="button" style={{ ...button, borderColor: 'var(--accent)' }} disabled={busy || (!needsRecovery && !freshComparison)} onClick={() => { void perform(async () => { await api.swarm.restore(sourceHash); setComparison(undefined); if (hash !== sourceHash) onRemoved?.() }) }}><LocalizedText source="Restore private swarm" /></button>
          <button type="button" style={button} disabled={busy || needsRecovery} onClick={() => setConfirmForget(!confirmForget)}><LocalizedText source="Delete retained private copy…" /></button>
          {confirmForget && <div role="group" aria-label="Delete retained private copy">
            <p><LocalizedText source="The public torrent and payload files will remain. Deleting the private copy permanently removes the restore action for this session." /></p>
            <button type="button" style={button} disabled={busy} onClick={() => { void perform(async () => {
              await api.torrents.remove(sourceHash, false)
              setConfirmForget(false); setComparison(undefined)
              if (hash === sourceHash) onRemoved?.()
            }) }}><LocalizedText source="Delete private copy, keep files" /></button>
          </div>}
        </>}
      </>}
      {message && <p role="alert" style={{ color: 'var(--danger)' }}><LocalizedErrorText message={message} /></p>}
    </div>}
  </section>
}
