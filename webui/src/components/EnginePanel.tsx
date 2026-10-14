import { LocalizedErrorText, LocalizedText } from '../lib/LocalizedText'
import { interfaceTextProps, translateInterface, useInterfaceLanguage } from '../lib/interfaceLocale'
import { getRegionalLocale } from '../lib/localeFormat'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api, type EngineDiagnostics, type ProbeValue, type RtorrentSettingDescriptor } from '../api/client'

export function EnginePanel() {
  const { data, isLoading, isFetching, error, refetch } = useQuery({
    queryKey: ['engine'],
    queryFn: api.engine,
    staleTime: 2_000,
    refetchInterval: 5_000,
  })
  const { data: commands, refetch: refetchCommands } = useQuery({
    queryKey: ['engine-commands'],
    queryFn: api.engineCommands,
    refetchInterval: 60000,
    enabled: data?.backend?.type === 'rtorrent',
  })

  const driftProblems = data?.drift.filter(row => row.status !== 'match').length ?? 0
  const isRtorrent = data?.backend?.type === 'rtorrent'
  const supportsOverlay = data?.backend?.capabilities.supports_config_overlay === true

  return (
    <section aria-labelledby="backend-title" aria-busy={isLoading || isFetching} style={{ padding: '16px 24px' }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, marginBottom: 12 }}>
        <h2 id="backend-title" style={{ fontSize: 13, margin: 0, color: 'var(--text)' }}><LocalizedText source="Backend" /></h2>
        {data && (
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <Badge ok text={data.backend.type === 'torrentng' ? translateInterface('TorrentNG client') : data.backend.type} />
            {isRtorrent && <Badge ok={driftProblems === 0} text={driftProblems === 0 ? translateInterface('Profile matches defaults') : translateInterface('{count} drift', { count: driftProblems.toLocaleString(getRegionalLocale()) })} />}
            {isRtorrent && <Badge ok={data.capabilities.every(c => c.available)} text={`${data.capabilities.filter(c => c.available).length}/${data.capabilities.length} XMLRPC`} />}
            <button
              type="button"
              onClick={() => {
                refetch()
                refetchCommands()
              }}
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
        )}
      </div>

      {isLoading && <div role="status" aria-label={translateInterface("Loading backend diagnostics")} {...interfaceTextProps("Loading backend diagnostics")}><EngineSkeleton /></div>}
      {error && <Notice><LocalizedText source="Engine diagnostics unavailable" /></Notice>}
      {data && (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(320px, 1fr))', gap: 16 }}>
          <BackendSummary data={data} />
          <Provenance data={data} />
          {isRtorrent && <Capabilities data={data} />}
          {isRtorrent && <HttpStack data={data} />}
          {isRtorrent && <DhtStack data={data} />}
          {supportsOverlay && <RtorrentSettingsPanel />}
          {isRtorrent && <ProfileDrift data={data} />}
          {isRtorrent && <CommandIndex commands={commands} />}
        </div>
      )}
    </section>
  )
}

function BackendSummary({ data }: { data: EngineDiagnostics }) {
  const caps = data.backend.capabilities
  const rows: Array<[string, string]> = [
    ['Type', backendDisplayName(data.backend.type)],
    ['Tags', yesNo(caps.supports_tags)],
    ['Categories', yesNo(caps.supports_categories)],
    ['File priority', yesNo(caps.supports_file_priority)],
    ['Tracker edit', yesNo(caps.supports_tracker_edit)],
    ['Recheck', yesNo(caps.supports_recheck)],
    ['Export', yesNo(caps.supports_torrent_export)],
    ['Webseed reads', yesNo(caps.supports_webseed_reads)],
    ['Piece states', yesNo(caps.supports_piece_state_reads)],
    ['Piece hashes', yesNo(caps.supports_piece_hash_reads)],
    ['Peer snapshots', yesNo(caps.supports_peer_snapshots)],
    ['Peer add', yesNo(caps.supports_peer_add)],
    ['Peer ban', yesNo(caps.supports_peer_ban)],
    ['Queue order', yesNo(caps.supports_queue_order)],
    ['Torrent limits', yesNo(caps.supports_per_torrent_limits)],
    ['Global limits', yesNo(caps.supports_global_limits)],
    ['Share limits', yesNo(caps.supports_share_limits)],
    ['Mode flags', yesNo(caps.supports_mode_flags)],
    ['Location', yesNo(caps.supports_location_update)],
    ['Torrent rename', yesNo(caps.supports_torrent_rename)],
    ['File rename', yesNo(caps.supports_file_rename)],
    ['Runtime user agent', yesNo(caps.supports_runtime_user_agent)],
    ['Config overlay', yesNo(caps.supports_config_overlay)],
    ['Restart', yesNo(caps.supports_restart)],
  ]
  return (
    <Panel>
      <Subhead><LocalizedText source="Backend" /></Subhead>
      <Rows rows={rows} />
    </Panel>
  )
}

function EngineSkeleton() {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(320px, 1fr))', gap: 16 }}>
      {Array.from({ length: 4 }).map((_, index) => (
        <Panel key={index}>
          <span className="tng-skeleton" style={{ width: 120, height: 12, marginBottom: 12 }} />
          <span className="tng-skeleton" style={{ width: '92%', height: 10, marginBottom: 8 }} />
          <span className="tng-skeleton" style={{ width: '72%', height: 10, marginBottom: 8 }} />
          <span className="tng-skeleton" style={{ width: '84%', height: 10 }} />
        </Panel>
      ))}
    </div>
  )
}

function Notice({ children }: { children: React.ReactNode }) {
  return (
    <div role="alert" aria-live="assertive" style={{
      color: 'var(--danger)', background: 'color-mix(in srgb, var(--danger) 9%, var(--surface))',
      border: '1px solid color-mix(in srgb, var(--danger) 45%, var(--border))',
      borderRadius: 6, padding: '8px 9px', fontSize: 12,
    }}>{children}</div>
  )
}

function Provenance({ data }: { data: EngineDiagnostics }) {
  const p = data.provenance
  return (
    <Panel>
      <Subhead><LocalizedText source="Provenance" /></Subhead>
      <Rows rows={[
        ['TorrentNG', p.daemon_version ?? p.sidecar_version ?? 'unknown'],
        ...(data.backend.type === 'rtorrent' ? [
          ['rTorrent', p.rtorrent_version ?? 'unknown'],
          ['libtorrent', p.libtorrent_version ?? 'unknown'],
          ['XMLRPC', p.xmlrpc_backend ?? 'not applicable'],
          ['Packaged rTorrent', p.packaged_rtorrent_version ?? 'not declared'],
          ['Packaged libtorrent', p.packaged_libtorrent_version ?? 'not declared'],
          ['Patches', p.patch_set.length ? p.patch_set.join(', ') : 'none declared'],
        ] as Array<[string, string]> : []),
      ]} />
    </Panel>
  )
}

function yesNo(value: boolean): string {
  return value ? 'yes' : 'no'
}

function backendDisplayName(type: string): string {
  return type === 'torrentng' ? 'TorrentNG client' : type
}

function Capabilities({ data }: { data: EngineDiagnostics }) {
  const { t, textProps } = useInterfaceLanguage()
  return (
    <Panel>
      <Subhead><LocalizedText source="Capabilities" /></Subhead>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(210px, 1fr))', gap: 8 }}>
        {data.capabilities.map(cap => (
          <div key={cap.key} className="tng-engine-capability" data-available={cap.available ? 'true' : 'false'} style={{
            border: '1px solid var(--border)',
            borderRadius: 6,
            padding: '8px 10px',
            background: cap.available ? 'var(--surface)' : 'color-mix(in srgb, var(--danger) 10%, var(--surface))',
            minWidth: 0,
          }}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
              <span {...textProps(cap.label)} style={{ fontSize: 12, color: 'var(--text)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{t(cap.label)}</span>
              <Badge ok={cap.available} text={cap.available ? 'yes' : 'no'} />
            </div>
            <div title={cap.command} style={{ fontSize: 11, color: 'var(--faint)', marginTop: 4, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {cap.command}
            </div>
            {cap.detail && (
              <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 5, lineHeight: 1.4 }}>
                <span {...textProps(cap.detail)}>{t(cap.detail)}</span>
              </div>
            )}
          </div>
        ))}
      </div>
    </Panel>
  )
}

function HttpStack({ data }: { data: EngineDiagnostics }) {
  const h = data.http
  return (
    <Panel wide>
      <Subhead><LocalizedText source="Tracker HTTP Stack" /></Subhead>
      <Rows rows={[
        ['User agent', val(h.user_agent)],
        ['Open requests', val(h.current_open)],
        ['Max total', val(h.max_total_connections)],
        ['Max per host', val(h.max_host_connections)],
        ['Max cached', val(h.max_cache_connections)],
        ['DNS cache timeout', suffix(h.dns_cache_timeout, 's')],
        ['Proxy', val(h.proxy_address) || 'none'],
        ['CA path', val(h.ca_path) || 'default'],
        ['CA cert', val(h.ca_cert) || 'default'],
        ['Verify peer', bool(h.ssl_verify_peer)],
        ['Verify host', bool(h.ssl_verify_host)],
      ]} />
    </Panel>
  )
}

function DhtStack({ data }: { data: EngineDiagnostics }) {
  const d = data.dht
  return (
    <Panel wide>
      <Subhead><LocalizedText source="DHT and peer discovery" /></Subhead>
      <Rows rows={[
        ['DHT', val(d.enabled) || 'unknown'],
        ['DHT port', val(d.port)],
        ['DHT override port', val(d.override_port) || 'none'],
        ['Listen port', val(d.listen_port)],
        ['Listen range', val(d.listen_range)],
        ['PEX', bool(d.pex)],
        ['UDP trackers', bool(d.udp_trackers)],
        ['DHT statistics', val(d.statistics) || 'unavailable'],
      ]} />
    </Panel>
  )
}

function ProfileDrift({ data }: { data: EngineDiagnostics }) {
  const { t, textProps } = useInterfaceLanguage()
  const rows = data.drift
  const problems = rows.filter(row => row.status !== 'match')
  return (
    <Panel wide>
      <Subhead><LocalizedText source="Engine Profile Drift" /></Subhead>
      {problems.length === 0 && <div style={{ color: 'var(--success)', fontSize: 12 }}><LocalizedText source="Running profile matches TorrentNG defaults" /></div>}
      {problems.length > 0 && (
        <div style={{ display: 'grid', gap: 6 }}>
          {problems.map(row => (
            <div key={row.key} className="tng-engine-drift" data-status={row.status} style={{
              display: 'grid',
              gridTemplateColumns: '180px minmax(0, 1fr) minmax(0, 1fr)',
              gap: 10,
              alignItems: 'center',
              border: '1px solid color-mix(in srgb, var(--danger) 50%, var(--border))',
              borderRadius: 6,
              padding: '7px 9px',
              background: 'color-mix(in srgb, var(--danger) 10%, var(--surface))',
              fontSize: 12,
            }}>
              <div title={row.command} {...textProps(row.label)} style={{ color: 'var(--text)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{t(row.label)}</div>
              <div title={row.expected} style={{ color: 'var(--faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}><LocalizedText source="Expected: {value}" parameters={{ value: row.expected }} /></div>
              <div title={row.actual ?? row.detail ?? ''} style={{ color: 'var(--danger)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {row.status === 'unavailable' ? <LocalizedText source="Unavailable" /> : <LocalizedText source="Actual: {value}" parameters={{ value: row.actual ?? '' }} />}
              </div>
            </div>
          ))}
        </div>
      )}
    </Panel>
  )
}

function RtorrentSettingsPanel() {
  const qc = useQueryClient()
  const { language, t } = useInterfaceLanguage()
  const { data, isLoading, error } = useQuery({
    queryKey: ['rtorrent-settings'],
    queryFn: api.rtorrentSettings.get,
    staleTime: 2_000,
  })
  const [draft, setDraft] = useState<Record<string, string | number | boolean>>({})
  const [customRc, setCustomRc] = useState('')
  const [filter, setFilter] = useState('')
  const [viewFilter, setViewFilter] = useState<SettingsViewFilter>('all')
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set())
  const [notice, setNotice] = useState<
    | { tone: 'ok' | 'warn' | 'error'; source: string; parameters?: Record<string, string | number> }
    | { tone: 'error'; raw: string }
    | null
  >(null)
  const [lastSavedAt, setLastSavedAt] = useState<number | null>(null)
  const [lastSaveFailed, setLastSaveFailed] = useState(false)
  const searchRef = useRef<HTMLInputElement>(null)
  const customRcRef = useRef<HTMLLabelElement>(null)
  const immediateSaveRef = useRef(0)

  useEffect(() => {
    if (!data) return
    const dirtySettings = data.settings.filter(setting => {
      const row = data.values.find(value => value.key === setting.key)
      return !sameSettingValue(draft[setting.key], baselineValue(setting, row?.saved ?? null, row?.live.value ?? null))
    })
    if (dirtySettings.length > 0 || customRc !== data.custom_rc) return
    const next: Record<string, string | number | boolean> = {}
    for (const setting of data.settings) {
      const row = data.values.find(value => value.key === setting.key)
      const value = row?.saved ?? row?.live.value ?? setting.default_value
      next[setting.key] = inputValue(setting.value_type, value)
    }
    if (!sameDraft(draft, next)) setDraft(next)
    if (customRc !== data.custom_rc) setCustomRc(data.custom_rc)
  }, [customRc, data, draft])

  const save = useMutation({
    mutationFn: (payload: { values: Record<string, string | number | boolean>; customRc: string }) =>
      api.rtorrentSettings.save(payload.values, payload.customRc, true),
    onSuccess: result => {
      qc.invalidateQueries({ queryKey: ['rtorrent-settings'] })
      qc.invalidateQueries({ queryKey: ['engine'] })
      setLastSavedAt(Date.now())
      setLastSaveFailed(false)
      setNotice({
        tone: result.errors.length ? 'warn' : 'ok',
        source: 'Save result · live applied: {live} · restart required: {restart} · live errors: {errors}',
        parameters: {
          live: result.applied.length.toLocaleString(getRegionalLocale()),
          restart: t(result.restart_required ? 'Yes' : 'No'),
          errors: result.errors.length.toLocaleString(getRegionalLocale()),
        },
      })
    },
    onError: e => {
      setLastSaveFailed(true)
      setNotice({ tone: 'error', raw: String(e) })
    },
  })
  const restart = useMutation({
    mutationFn: api.rtorrentSettings.restart,
    onSuccess: () => setNotice({ tone: 'warn', source: 'Restart requested. The container or service should come back automatically.' }),
    onError: e => setNotice({ tone: 'error', raw: String(e) }),
  })

  const dirtySettings = data?.settings.filter(setting => {
    const row = data.values.find(value => value.key === setting.key)
    return !sameSettingValue(draft[setting.key], baselineValue(setting, row?.saved ?? null, row?.live.value ?? null))
  }) ?? []
  const customRcDirty = data ? customRc !== data.custom_rc : false
  const dirtyCount = dirtySettings.length + (customRcDirty ? 1 : 0)
  const unavailableCount = data?.values.filter(value => !value.live.ok).length ?? 0
  const filteredGroups = useMemo(() => {
    if (!data) return []
    const needle = filter.trim().toLowerCase()
    return groupSettings(data.settings
      .filter(setting => {
        const row = data.values.find(value => value.key === setting.key)
        const isDirty = !sameSettingValue(draft[setting.key], baselineValue(setting, row?.saved ?? null, row?.live.value ?? null))
        if (viewFilter === 'edited' && !isDirty) return false
        if (viewFilter === 'restart' && !setting.restart_required) return false
        if (viewFilter === 'live' && setting.restart_required) return false
        if (viewFilter === 'unavailable' && row?.live.ok !== false) return false
        if (!needle) return true
        return [
          setting.key,
          setting.label,
          t(setting.label),
          setting.command,
          setting.setter,
          setting.value_type,
          settingCategory(setting),
          settingDescription(setting, t),
        ].some(value => value.toLowerCase().includes(needle))
      }))
  }, [data, draft, filter, language, t, viewFilter])
  const resetAll = useCallback(() => {
    if (!data) return
    const next: Record<string, string | number | boolean> = {}
    for (const setting of data.settings) {
      const row = data.values.find(value => value.key === setting.key)
      next[setting.key] = baselineValue(setting, row?.saved ?? null, row?.live.value ?? null)
    }
    setDraft(next)
    setCustomRc(data.custom_rc)
    if (data.overlay_writable && !save.isPending) {
      window.setTimeout(() => save.mutate({ values: next, customRc: data.custom_rc }), 0)
    }
    setLastSaveFailed(false)
    setNotice(null)
  }, [data, save])
  const defaultAll = () => {
    if (!data) return
    if (!window.confirm(translateInterface('Apply defaults to every managed rTorrent setting? The changes will autosave shortly.'))) return
    const next: Record<string, string | number | boolean> = {}
    for (const setting of data.settings) {
      next[setting.key] = inputValue(setting.value_type, setting.default_value)
    }
    setDraft(next)
    setLastSaveFailed(false)
    if (data.overlay_writable && !save.isPending) {
      window.setTimeout(() => save.mutate({ values: next, customRc }), 0)
    }
    setNotice({ tone: 'warn', source: 'Managed settings reset to their defaults. Changes will be saved automatically shortly.' })
  }
  const resetGroup = (settings: RtorrentSettingDescriptor[]) => {
    if (!data) return
    setDraft(prev => {
      const next = { ...prev }
      for (const setting of settings) {
        const row = data.values.find(value => value.key === setting.key)
        next[setting.key] = baselineValue(setting, row?.saved ?? null, row?.live.value ?? null)
      }
      if (data.overlay_writable && !save.isPending) {
        window.setTimeout(() => save.mutate({ values: next, customRc }), 0)
      }
      return next
    })
  }
  const defaultGroup = (settings: RtorrentSettingDescriptor[]) => {
    if (!window.confirm(translateInterface('Apply defaults to {count} settings in this group? Autosave will apply shortly.', { count: settings.length.toLocaleString(getRegionalLocale()) }))) return
    setDraft(prev => {
      const next = { ...prev }
      for (const setting of settings) {
        next[setting.key] = inputValue(setting.value_type, setting.default_value)
      }
      if (data?.overlay_writable && !save.isPending) {
        window.setTimeout(() => save.mutate({ values: next, customRc }), 0)
      }
      return next
    })
  }
  const toggleGroup = (name: string) => {
    setCollapsedGroups(prev => {
      const next = new Set(prev)
      if (next.has(name)) next.delete(name)
      else next.add(name)
      return next
    })
  }
  const copyText = async (text: string, label: string) => {
    try {
      await navigator.clipboard?.writeText(text)
      setNotice({ tone: 'ok', source: '{label} copied', parameters: { label } })
    } catch {
      setNotice({ tone: 'error', source: 'Could not copy {label}', parameters: { label } })
    }
  }
  const restartDirtyCount = data ? dirtySettings.filter(setting => setting.restart_required).length : 0
  const liveDirtyCount = dirtySettings.length - restartDirtyCount
  const visibleSettingCount = filteredGroups.reduce((sum, group) => sum + group.settings.length, 0)
  const collapsedVisibleCount = filteredGroups
    .filter(group => collapsedGroups.has(group.name))
    .reduce((sum, group) => sum + group.settings.length, 0)
  const allGroupsCollapsed = filteredGroups.length > 0 && filteredGroups.every(group => collapsedGroups.has(group.name))
  const autosaveText = lastSaveFailed
    ? t('{count} pending changes could not be saved. Retry when the API is available.', { count: dirtyCount.toLocaleString(getRegionalLocale()) })
    : save.isPending
      ? t('Saving changes…')
      : dirtyCount === 0
        ? lastSavedAt
          ? t('All settings are saved; last save {when}.', { when: formatRelativeTime(lastSavedAt, language, t) })
          : t('All settings are saved.')
        : t(
          customRcDirty
            ? 'Pending autosave · changes: {count} · live: {live} · restart: {restart} · custom lines edited.'
            : 'Pending autosave · changes: {count} · live: {live} · restart: {restart}.',
          {
            count: dirtyCount.toLocaleString(getRegionalLocale()),
            live: liveDirtyCount.toLocaleString(getRegionalLocale()),
            restart: restartDirtyCount.toLocaleString(getRegionalLocale()),
          },
        )
  const toggleAllGroups = () => {
    if (allGroupsCollapsed) {
      setCollapsedGroups(new Set())
    } else {
      setCollapsedGroups(new Set(filteredGroups.map(group => group.name)))
    }
  }
  const visibleSettings = filteredGroups.flatMap(group => group.settings)
  const copyVisibleCommands = () => {
    const text = visibleSettings
      .map(setting => `${t(setting.label)}\n  get: ${setting.command}\n  set: ${setting.setter}`)
      .join('\n')
    copyText(text, t('Visible commands'))
  }
  const copyChangeSummary = () => {
    if (!data) return
    const lines = dirtySettings.map(setting => {
      const row = data.values.find(value => value.key === setting.key)
      const base = baselineValue(setting, row?.saved ?? null, row?.live.value ?? null)
      const next = draft[setting.key] ?? base
      return `${t(setting.label)}: ${formatSettingValue(base, setting.unit)} -> ${formatSettingValue(next, setting.unit)} (${setting.restart_required ? t('Restart required') : t('Live')})`
    })
    if (customRcDirty) lines.push(`${t('Custom rTorrent lines')}: ${t('edited')} (${t('Restart required')})`)
    copyText(lines.join('\n'), t('Change summary'))
  }
  const updateSetting = useCallback((setting: RtorrentSettingDescriptor, value: string | number | boolean, immediate = false) => {
    const nextValue = setting.value_type === 'int' ? clampSettingValue(setting, Number(value)) : value
    setLastSaveFailed(false)
    setDraft(prev => {
      const next = { ...prev, [setting.key]: nextValue }
      if (immediate && data?.overlay_writable && !save.isPending) {
        immediateSaveRef.current = Date.now()
        window.setTimeout(() => save.mutate({ values: next, customRc }), 0)
      }
      return next
    })
  }, [customRc, data?.overlay_writable, save])
  const saveNow = useCallback(() => {
    if (!data?.overlay_writable || dirtyCount === 0 || save.isPending) return
    setLastSaveFailed(false)
    save.mutate({ values: draft, customRc })
  }, [customRc, data?.overlay_writable, dirtyCount, draft, save])

  useEffect(() => {
    if (!data?.overlay_writable || dirtyCount === 0 || save.isPending) return
    if (Date.now() - immediateSaveRef.current < 700) return
    const timer = window.setTimeout(() => {
      save.mutate({ values: draft, customRc })
    }, 650)
    return () => window.clearTimeout(timer)
  }, [customRc, data?.overlay_writable, dirtyCount, draft, save])

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.isComposing) return
      if (!(e.ctrlKey || e.metaKey)) return
      if (e.key.toLowerCase() === 's') {
        e.preventDefault()
        saveNow()
      }
      if (e.key.toLowerCase() === 'z' && e.shiftKey) {
        e.preventDefault()
        resetAll()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [resetAll, saveNow])

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.isComposing) return
      const target = e.target
      const isTyping = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement
      if (e.key === '/' && !isTyping && !e.ctrlKey && !e.metaKey && !e.altKey) {
        e.preventDefault()
        searchRef.current?.focus()
      }
      if (e.key === 'Escape' && target === searchRef.current && filter) {
        e.preventDefault()
        setFilter('')
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [filter])

  useEffect(() => {
    function onBeforeUnload(e: BeforeUnloadEvent) {
      if (dirtyCount === 0 || save.isPending) return
      e.preventDefault()
      e.returnValue = ''
    }
    window.addEventListener('beforeunload', onBeforeUnload)
    return () => window.removeEventListener('beforeunload', onBeforeUnload)
  }, [dirtyCount, save.isPending])

  return (
    <Panel wide>
      <Subhead><LocalizedText source="Settings Control Panel" /></Subhead>
      {isLoading && <span className="tng-skeleton" style={{ width: '70%', height: 12 }} />}
      {error && <InlineNotice><LocalizedText source="rTorrent settings unavailable" /></InlineNotice>}
      {data && (
        <div style={{ display: 'grid', gap: 10 }}>
          <div id="rt-settings-autosave-status" role="status" aria-live="polite" style={visuallyHiddenStyle}>
            {autosaveText}
          </div>
          {lastSaveFailed && (
            <div role="alert" style={visuallyHiddenStyle}>
              <LocalizedText source="{count} pending changes could not be saved. Retry when the API is available." parameters={{ count: dirtyCount.toLocaleString(getRegionalLocale()) }} /></div>
          )}
          <div style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))',
            gap: 8,
          }}>
            <SettingStat label={translateInterface("Managed settings")} value={data.settings.length.toLocaleString(getRegionalLocale())} />
            <SettingStat label={translateInterface("Pending autosave")} value={dirtyCount.toLocaleString(getRegionalLocale())} tone={dirtyCount > 0 ? 'warn' : 'ok'} />
            <SettingStat label={translateInterface("Custom lines")} value={translateInterface(customRcDirty ? 'edited' : 'saved')} tone={customRcDirty ? 'warn' : 'ok'} />
            <SettingStat label={translateInterface("Live readback unavailable")} value={unavailableCount.toLocaleString(getRegionalLocale())} tone={unavailableCount > 0 ? 'warn' : 'ok'} />
            <SettingStat label={translateInterface('Overlay')} value={translateInterface(data.overlay_writable ? 'Writable' : 'Readonly')} tone={data.overlay_writable ? 'ok' : 'warn'} />
            <SettingStat label={translateInterface("Restart support")} value={translateInterface(data.restart_supported ? 'Available' : 'Manual')} tone={data.restart_supported ? 'ok' : 'warn'} />
          </div>
          {!data.overlay_writable && (
            <div role="alert" style={{
              color: 'var(--warning)',
              border: '1px solid color-mix(in srgb, var(--warning) 50%, var(--border))',
              borderRadius: 7,
              background: 'color-mix(in srgb, var(--warning) 8%, var(--surface))',
              padding: '8px 10px',
              fontSize: 12,
            }}>
              <LocalizedText source="Settings overlay directory is not writable from this process. Review values is still available; save may fail until the service can write the overlay path." /></div>
          )}
          <div style={{ color: 'var(--faint)', fontSize: 12, lineHeight: 1.45, display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
            <span>
              <LocalizedText source="Values are saved to" spaceAfter /><span style={{ color: 'var(--muted)', fontFamily: 'monospace' }}>{data.overlay_path}</span><LocalizedText source=". Live-safe controls are applied immediately; controls marked restart are saved for the next daemon start." /></span>
            <button type="button" onClick={() => copyText(data.overlay_path, t('Overlay path'))} style={smallButtonStyle(true)}>
              <LocalizedText source="Copy path" /></button>
          </div>
          <div style={{
            display: 'grid',
            gridTemplateColumns: 'minmax(260px, 640px) minmax(240px, 1fr)',
            gap: 10,
            alignItems: 'end',
          }}>
            <label style={{ display: 'grid', gap: 5, minWidth: 0 }}>
              <span style={{ color: 'var(--text)', fontSize: 12, fontWeight: 800 }}><LocalizedText source="Find settings" /></span>
              <span style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) auto', gap: 6 }}>
                <input
                  ref={searchRef}
                  type="search"
                  value={filter}
                  onChange={e => setFilter(e.target.value)}
                  aria-keyshortcuts="/"
                  aria-label={translateInterface("Find settings")}
                  aria-describedby="rt-settings-result-count"
                  title={translateInterface("Find settings")}
                  placeholder={translateInterface("Filter by label, command, category, or type")}
                  style={{ ...inputStyle, padding: '7px 9px' }}
                {...interfaceTextProps("Find settings")} />
                <button type="button" onClick={() => { setFilter(''); searchRef.current?.focus() }} disabled={!filter} style={smallButtonStyle(Boolean(filter))}>
                  <LocalizedText source="Clear" />
                </button>
              </span>
            </label>
            <ViewFilter
              value={viewFilter}
              onChange={setViewFilter}
              dirtyCount={dirtySettings.length}
              restartCount={data.settings.filter(setting => setting.restart_required).length}
              liveCount={data.settings.filter(setting => !setting.restart_required).length}
              unavailableCount={unavailableCount}
            />
          </div>
          {(filter || viewFilter !== 'all') && (
            <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
              <button type="button" onClick={() => { setFilter(''); setViewFilter('all') }} style={smallButtonStyle(true)}>
                <LocalizedText source="Clear filters" /></button>
              <span style={{ color: 'var(--faint)', fontSize: 11 }}>
                <LocalizedText
                  source={filter && viewFilter !== 'all' ? 'Filtered by {filter} and {view}' : filter ? 'Filtered by {filter}' : 'Showing {view} settings'}
                  parameters={{ filter: `"${filter}"`, view: t(viewFilter === 'restart' ? 'Restart required' : viewFilter === 'edited' ? 'Edited' : viewFilter === 'live' ? 'Live' : 'Unavailable') }}
                />
              </span>
            </div>
          )}
          <div id="rt-settings-result-count" role="status" aria-live="polite" style={{ color: 'var(--faint)', fontSize: 11 }}>
            <LocalizedText
              source={collapsedVisibleCount > 0
                ? 'Showing {visible} of {total} managed settings; {hidden} hidden in collapsed groups.'
                : 'Showing {visible} of {total} managed settings.'}
              parameters={{
                visible: visibleSettingCount.toLocaleString(getRegionalLocale()),
                total: data.settings.length.toLocaleString(getRegionalLocale()),
                hidden: collapsedVisibleCount.toLocaleString(getRegionalLocale()),
              }}
            />
          </div>
          {filteredGroups.length > 0 && (
            <nav aria-label={translateInterface("Settings groups")} style={{
              display: 'flex',
              gap: 6,
              flexWrap: 'wrap',
              border: '1px solid var(--border)',
              borderRadius: 8,
              background: 'color-mix(in srgb, var(--surface) 70%, var(--bg))',
              padding: 8,
            }} {...interfaceTextProps("Settings groups")}>
              {filteredGroups.map(group => {
                const groupId = groupDomId(group.name)
                return (
                  <button
                    key={group.name}
                    type="button"
                    onClick={() => document.getElementById(groupId)?.scrollIntoView({ block: 'start', behavior: 'smooth' })}
                    style={smallButtonStyle(true)}
                    aria-controls={groupId}
                  >
                    <LocalizedText source="{group} ({count})" parameters={{ group: group.name, count: group.settings.length.toLocaleString(getRegionalLocale()) }} />
                  </button>
                )
              })}
              <button
                type="button"
                onClick={() => customRcRef.current?.scrollIntoView({ block: 'start', behavior: 'smooth' })}
                style={smallButtonStyle(true)}
                aria-controls="rt-custom-rc"
              >
                <LocalizedText source="Custom lines ({count})" parameters={{ count: customRc.split('\n').filter(line => line.trim()).length.toLocaleString(getRegionalLocale()) }} />
              </button>
            </nav>
          )}
          <div style={{
            display: 'grid',
            gridTemplateColumns: 'minmax(260px, 1fr) auto',
            gap: 10,
            alignItems: 'center',
            border: '1px solid var(--border)',
            borderRadius: 8,
            background: 'color-mix(in srgb, var(--surface) 82%, var(--bg))',
            padding: 10,
          }}>
            <div style={{ display: 'grid', gap: 4, minWidth: 0 }}>
              <div style={{ color: 'var(--text)', fontSize: 12, fontWeight: 800 }}><LocalizedText source="Autosave state" /></div>
              <div style={{ color: 'var(--faint)', fontSize: 11 }}>
                {autosaveText}
              </div>
            </div>
            <div role="toolbar" aria-label={translateInterface("Settings workspace actions")} style={{ display: 'flex', gap: 6, flexWrap: 'wrap', justifyContent: 'flex-end' }} {...interfaceTextProps("Settings workspace actions")}>
              <button type="button" onClick={toggleAllGroups} style={smallButtonStyle(filteredGroups.length > 0)} disabled={filteredGroups.length === 0}>
                {allGroupsCollapsed ? <LocalizedText source="Expand all" /> : <LocalizedText source="Collapse all" />}
              </button>
              <button
                type="button"
                onClick={defaultAll}
                title={translateInterface(!data.overlay_writable ? 'Settings overlay is readonly' : 'Apply defaults to all managed settings')}
                disabled={save.isPending || !data.overlay_writable}
                style={smallButtonStyle(!save.isPending && data.overlay_writable)}
              >
                <LocalizedText source="Reset all managed settings to defaults" /></button>
              <button type="button" onClick={copyVisibleCommands} disabled={visibleSettings.length === 0} style={smallButtonStyle(visibleSettings.length > 0)}>
                <LocalizedText source="Copy visible commands" /></button>
              <button type="button" onClick={copyChangeSummary} disabled={dirtyCount === 0} style={smallButtonStyle(dirtyCount > 0)}>
                <LocalizedText source="Copy change summary" /></button>
            </div>
          </div>
          {dirtyCount > 0 && (
            <ChangeReview
              settings={dirtySettings}
              draft={draft}
              customRcDirty={customRcDirty}
              values={data.values}
              saving={save.isPending}
            />
          )}
          {lastSaveFailed && (
            <div role="alert" style={{
              color: 'var(--danger)',
              border: '1px solid color-mix(in srgb, var(--danger) 48%, var(--border))',
              borderRadius: 8,
              background: 'color-mix(in srgb, var(--danger) 8%, var(--surface))',
              padding: '8px 10px',
              fontSize: 12,
            }}>
              <LocalizedText source="{count} pending changes could not be saved. Retry when the API is available." parameters={{ count: dirtyCount.toLocaleString(getRegionalLocale()) }} /></div>
          )}
          <div style={{ display: 'grid', gap: 12 }}>
            {filteredGroups.map(group => {
              const collapsed = collapsedGroups.has(group.name)
              const groupDirty = group.settings.filter(setting => {
                const row = data.values.find(value => value.key === setting.key)
                return !sameSettingValue(draft[setting.key], baselineValue(setting, row?.saved ?? null, row?.live.value ?? null))
              }).length
              const groupRestart = group.settings.filter(setting => setting.restart_required).length
              const groupId = groupDomId(group.name)
              const bodyId = `${groupId}-body`
              return (
              <fieldset id={groupId} key={group.name} tabIndex={-1} style={{
                border: '1px solid var(--border)',
                borderRadius: 8,
                background: 'color-mix(in srgb, var(--surface) 72%, var(--bg))',
                padding: '10px 12px 12px',
                minWidth: 0,
              }}>
                <legend style={{
                  color: 'var(--accent-text)',
                  fontSize: 11,
                  fontWeight: 900,
                  textTransform: 'uppercase',
                  padding: '0 6px',
                }}>
                  <LocalizedText source={group.name} />
                </legend>
                <div style={{
                  display: 'grid',
                  gridTemplateColumns: 'minmax(0, 1fr) auto',
                  gap: 10,
                  alignItems: 'start',
                  marginBottom: 9,
                }}>
                  <div style={{ display: 'grid', gap: 5, minWidth: 0 }}>
                    <div style={{ color: 'var(--faint)', fontSize: 11 }}><LocalizedText source={group.description} /></div>
                    <div style={{ display: 'flex', gap: 5, flexWrap: 'wrap' }}>
                      <SettingPill tone={groupDirty > 0 ? 'warn' : 'ok'}><LocalizedText source="Edited: {count}" parameters={{ count: groupDirty.toLocaleString(getRegionalLocale()) }} /></SettingPill>
                      <SettingPill tone={groupRestart > 0 ? 'warn' : 'ok'}><LocalizedText source="Restart required: {count}" parameters={{ count: groupRestart.toLocaleString(getRegionalLocale()) }} /></SettingPill>
                      <SettingPill tone="ok"><LocalizedText source="Shown: {count}" parameters={{ count: group.settings.length.toLocaleString(getRegionalLocale()) }} /></SettingPill>
                    </div>
                  </div>
                  <div role="toolbar" aria-label={translateInterface('{group} actions', { group: group.name })} {...interfaceTextProps('{group} actions')} style={{ display: 'flex', gap: 6, flexWrap: 'wrap', justifyContent: 'flex-end' }}>
                    <button
                      type="button"
                      onClick={() => defaultGroup(group.settings)}
                      title={translateInterface(!data.overlay_writable ? 'Settings overlay is readonly' : 'Apply defaults to this group')}
                      style={smallButtonStyle(!save.isPending && data.overlay_writable)}
                      disabled={save.isPending || !data.overlay_writable}
                    >
                      <LocalizedText source="Defaults" />
                    </button>
                    <button type="button" onClick={() => resetGroup(group.settings)} style={smallButtonStyle(groupDirty > 0 && !save.isPending)} disabled={groupDirty === 0 || save.isPending}>
                      <LocalizedText source="Reset group" /></button>
                    <button
                      type="button"
                      aria-expanded={!collapsed}
                      aria-controls={bodyId}
                      onClick={() => toggleGroup(group.name)}
                      style={smallButtonStyle(true)}
                    >
                      {collapsed ? <LocalizedText source="Expand" /> : <LocalizedText source="Collapse" />}
                    </button>
                  </div>
                </div>
                <div id={bodyId} hidden={collapsed} style={{ display: collapsed ? 'none' : 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 380px), 1fr))', gap: 10 }}>
                  {group.settings.map(setting => {
                    const row = data.values.find(value => value.key === setting.key)
                    const base = baselineValue(setting, row?.saved ?? null, row?.live.value ?? null)
                    const value = draft[setting.key] ?? base
                    const saved = row?.saved ?? null
                    const live = row?.live.value ?? null
                    const dirty = !sameSettingValue(value, base)
                    const descriptionId = `rt-setting-${setting.key}-description`
                    const titleId = `rt-setting-${setting.key}-title`
                    const liveErrorId = `rt-setting-${setting.key}-live-error`
                    const liveError = row?.live.ok === false ? row.live.error ?? 'Live readback unavailable' : null
                    const settingLabel = t(setting.label)
                    return (
                      <section key={setting.key} className="tng-form-card" data-dirty={dirty ? 'true' : 'false'} aria-labelledby={titleId} aria-describedby={liveError ? `${descriptionId} ${liveErrorId}` : descriptionId} style={{
                        border: `1px solid ${dirty ? 'color-mix(in srgb, var(--warning) 58%, var(--border))' : 'var(--border)'}`,
                        borderRadius: 7,
                        background: dirty ? 'color-mix(in srgb, var(--warning) 7%, var(--surface))' : 'var(--surface)',
                        padding: 10,
                        display: 'grid',
                        gap: 9,
                        minWidth: 0,
                      }}>
                        <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8, color: 'var(--text)', fontSize: 13, fontWeight: 800 }}>
                          <span id={titleId} {...interfaceTextProps(setting.label)}>{settingLabel}</span>
                          <span style={{ display: 'inline-flex', gap: 5, alignItems: 'center', flexShrink: 0 }}>
                            {dirty && <SettingPill tone="warn"><LocalizedText source="edited" /></SettingPill>}
                            {dirty && <SettingPill tone={lastSaveFailed ? 'warn' : save.isPending ? 'warn' : 'ok'}><LocalizedText source={lastSaveFailed ? 'retry' : save.isPending ? 'saving' : 'pending'} /></SettingPill>}
                            {setting.restart_required && <SettingPill tone="warn"><LocalizedText source="Restart required" /></SettingPill>}
                            {row?.live.ok === false && <SettingPill tone="warn"><LocalizedText source="unavailable" /></SettingPill>}
                          </span>
                        </div>
                        <div id={descriptionId} style={{ color: 'var(--faint)', fontSize: 11, lineHeight: 1.35 }}>
                          {settingDescription(setting, t)}
                        </div>
                        {liveError && (
                          <div id={liveErrorId} role="status" style={{
                            color: 'var(--warning)',
                            border: '1px solid color-mix(in srgb, var(--warning) 45%, var(--border))',
                            borderRadius: 6,
                            background: 'color-mix(in srgb, var(--warning) 8%, var(--surface))',
                            padding: '6px 7px',
                            fontSize: 11,
                            overflowWrap: 'anywhere',
                          }}>
                            <LocalizedErrorText message={liveError} />
                          </div>
                        )}
                        {setting.value_type === 'bool' ? (
                          <BooleanKnob
                            setting={setting}
                            value={Boolean(value)}
                            describedBy={descriptionId}
                            onChange={next => updateSetting(setting, next, true)}
                          />
                        ) : setting.key === 'dht_mode' ? (
                          <SegmentedSetting
                            label={settingLabel}
                            value={String(value)}
                            options={[['auto', 'Auto'], ['on', 'On'], ['disable', 'Off']]}
                            describedBy={descriptionId}
                            onChange={next => updateSetting(setting, next, true)}
                          />
                        ) : (
                        <NumericKnob
                          setting={setting}
                          value={Number(value)}
                          describedBy={descriptionId}
                          onChange={next => updateSetting(setting, next)}
                          onPreset={next => updateSetting(setting, next, true)}
                        />
                        )}
                        <div style={readoutGridStyle}>
                          <Readout label="Editing" value={formatSettingValue(value, setting.unit)} />
                          <Readout label="Current" value={live === null ? 'unavailable' : formatSettingValue(live, setting.unit)} />
                          <Readout label="Saved" value={saved === null ? 'default' : formatSettingValue(saved, setting.unit)} />
                          <Readout label="Default" value={formatSettingValue(inputValue(setting.value_type, setting.default_value), setting.unit)} />
                          <Readout label="Command" value={setting.command} mono />
                        </div>
                        <div style={{ display: 'grid', gridTemplateColumns: 'minmax(120px, 1fr) auto', gap: 8, alignItems: 'center' }}>
                          <span style={{ color: 'var(--faint)', fontSize: 10 }}>
                            {setting.restart_required ? <LocalizedText source="Autosaves now; effective after restart" /> : <LocalizedText source="Autosaves and applies live" />}
                          </span>
                          <div role="toolbar" aria-label={translateInterface('{setting} actions', { setting: settingLabel })} {...interfaceTextProps('{setting} actions')} style={{ display: 'flex', gap: 6, flexWrap: 'wrap', justifyContent: 'flex-end' }}>
                            <button
                              type="button"
                              aria-label={translateInterface('Reset {setting} to baseline', { setting: settingLabel })}
                              onClick={() => updateSetting(setting, base, true)}
                              disabled={!dirty || save.isPending}
                              style={smallButtonStyle(dirty && !save.isPending)}
                            >
                              <LocalizedText source="Reset" />
                            </button>
                            {saved !== null && (
                              <button
                                type="button"
                                aria-label={translateInterface('Set {setting} to saved value', { setting: settingLabel })}
                                onClick={() => updateSetting(setting, inputValue(setting.value_type, saved), true)}
                                disabled={save.isPending}
                                style={smallButtonStyle(!save.isPending)}
                              >
                                <LocalizedText source="Saved" />
                              </button>
                            )}
                            {live !== null && (
                              <button
                                type="button"
                                aria-label={translateInterface('Set {setting} to live value', { setting: settingLabel })}
                                onClick={() => updateSetting(setting, inputValue(setting.value_type, live), true)}
                                disabled={save.isPending}
                                style={smallButtonStyle(!save.isPending)}
                              >
                                <LocalizedText source="Current" />
                              </button>
                            )}
                            <button
                              type="button"
                              aria-label={translateInterface('Set {setting} to default value', { setting: settingLabel })}
                              onClick={() => updateSetting(setting, inputValue(setting.value_type, setting.default_value), true)}
                              disabled={save.isPending}
                              style={smallButtonStyle(!save.isPending)}
                            >
                              <LocalizedText source="Default" />
                            </button>
                            <button
                              type="button"
                              aria-label={translateInterface('Copy {setting} current value', { setting: settingLabel })}
                              onClick={() => copyText(formatSettingValue(value, setting.unit), translateInterface('{setting} value', { setting: settingLabel }))}
                              style={smallButtonStyle(true)}
                            >
                              <LocalizedText source="Copy value" /></button>
                            <button
                              type="button"
                              aria-label={translateInterface('Copy {setting} configuration commands', { setting: settingLabel })}
                              onClick={() => copyText(`${setting.command} / ${setting.setter}`, settingLabel)}
                              style={smallButtonStyle(true)}
                            >
                              <LocalizedText source="Copy command" /></button>
                          </div>
                        </div>
                      </section>
                    )
                  })}
                </div>
              </fieldset>
              )
            })}
            {filteredGroups.length === 0 && (
              <div style={{
                color: 'var(--faint)',
                border: '1px dashed var(--border-strong)',
                borderRadius: 8,
                padding: 14,
                fontSize: 12,
              }}>
                <LocalizedText source="No settings match the current filter." /></div>
            )}
          </div>
          <label id="rt-custom-rc" ref={customRcRef} style={{
            scrollMarginTop: 12,
            display: 'grid',
            gap: 6,
            border: `1px solid ${customRcDirty ? 'color-mix(in srgb, var(--warning) 58%, var(--border))' : 'var(--border)'}`,
            borderRadius: 8,
            padding: 10,
            background: customRcDirty ? 'color-mix(in srgb, var(--warning) 7%, var(--surface))' : 'var(--surface)',
          }}>
            <span style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
              <span style={{ color: 'var(--text)', fontSize: 12, fontWeight: 800 }}><LocalizedText source="Custom rTorrent lines" /></span>
              {customRcDirty && <SettingPill tone="warn"><LocalizedText source="edited" /></SettingPill>}
            </span>
            <span style={{ color: 'var(--faint)', fontSize: 11 }}><LocalizedText source="Advanced overrides are imported after managed settings and usually need a restart." /></span>
            <textarea
              value={customRc}
              onChange={e => setCustomRc(e.target.value)}
              onBlur={saveNow}
              rows={4}
              aria-label={translateInterface("Custom rTorrent configuration lines")}
              placeholder={translateInterface("Optional advanced rtorrent.rc overrides imported after managed settings")}
              style={{ ...inputStyle, resize: 'vertical', fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace' }}
            {...interfaceTextProps("Custom rTorrent configuration lines")} />
            <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8, alignItems: 'center' }}>
              <span style={{ color: 'var(--faint)', fontSize: 10 }}><LocalizedText source="Custom lines: {count}" parameters={{ count: customRc.split('\n').filter(line => line.trim()).length.toLocaleString(getRegionalLocale()) }} /></span>
              <button type="button" onClick={() => setCustomRc(data.custom_rc)} disabled={!customRcDirty || save.isPending} style={smallButtonStyle(customRcDirty && !save.isPending)}>
                <LocalizedText source="Reset custom lines" /></button>
            </div>
          </label>
          {notice && <div style={{
            color: notice.tone === 'error' ? 'var(--danger)' : notice.tone === 'warn' ? 'var(--warning)' : 'var(--success)',
            fontSize: 12,
            border: `1px solid color-mix(in srgb, ${notice.tone === 'error' ? 'var(--danger)' : notice.tone === 'warn' ? 'var(--warning)' : 'var(--success)'} 42%, var(--border))`,
            borderRadius: 6,
            padding: '7px 9px',
            background: 'var(--bg)',
          }} role="status" aria-live="polite">{'source' in notice
            ? <LocalizedText source={notice.source} parameters={notice.parameters} />
            : <LocalizedErrorText message={notice.raw} />}</div>}
          <div style={{
            position: 'sticky',
            bottom: 0,
            zIndex: 2,
            display: 'flex',
            gap: 8,
            flexWrap: 'wrap',
            alignItems: 'center',
            borderTop: '1px solid var(--border)',
            background: 'color-mix(in srgb, var(--surface) 92%, var(--bg))',
            padding: '10px 0 0',
          }} role="toolbar" aria-label={translateInterface("Settings save actions")} {...interfaceTextProps("Settings save actions")}>
            <button
              type="button"
              onClick={saveNow}
              disabled={save.isPending || dirtyCount === 0 || !data.overlay_writable}
              aria-keyshortcuts="Control+S Meta+S"
              title={translateInterface(lastSaveFailed ? 'Retry autosave now' : 'Save pending changes now')}
              style={buttonStyle(lastSaveFailed ? 'var(--danger)' : 'var(--accent)', lastSaveFailed ? 'color-mix(in srgb, var(--danger) 12%, var(--surface))' : 'var(--accent-soft)', lastSaveFailed ? 'var(--danger)' : 'var(--accent-text)', dirtyCount > 0 && !save.isPending && data.overlay_writable)}
            >
              {save.isPending ? <LocalizedText source="Saving…" /> : lastSaveFailed ? <LocalizedText source="Retry save" /> : <LocalizedText source="Save now" />}
            </button>
            <button
              type="button"
              onClick={resetAll}
              disabled={dirtyCount === 0 || save.isPending}
              aria-keyshortcuts="Control+Shift+Z Meta+Shift+Z"
              title={translateInterface("Reset edits")}
              style={buttonStyle('var(--border-strong)', 'var(--surface-2)', 'var(--muted)', dirtyCount > 0 && !save.isPending)}
             {...interfaceTextProps("Reset edits")}>
              <LocalizedText source="Reset edits" /></button>
            <button
              type="button"
              onClick={() => {
                if (window.confirm(translateInterface('Restart rTorrent/TorrentNG now? Active transfers will reconnect after the service comes back.'))) restart.mutate()
              }}
              disabled={restart.isPending}
              style={buttonStyle('var(--warning)', 'color-mix(in srgb, var(--warning) 12%, var(--surface))', 'var(--warning)')}
            >
              {restart.isPending ? <LocalizedText source="Restarting…" /> : <LocalizedText source="Restart daemon" />}
            </button>
            <span role="status" aria-live="polite" style={{ color: lastSaveFailed ? 'var(--danger)' : dirtyCount > 0 ? 'var(--warning)' : 'var(--faint)', fontSize: 12, fontWeight: 800 }}>
              {!data.overlay_writable ? <LocalizedText source="Read-only configuration overlay" /> : save.isPending ? <LocalizedText source="Autosaving" /> : lastSaveFailed ? <LocalizedText source="Autosave failed" /> : dirtyCount > 0 ? <LocalizedText source="{count} pending" parameters={{ count: dirtyCount.toLocaleString(getRegionalLocale()) }} /> : <LocalizedText source="Saved" />}
            </span>
          </div>
        </div>
      )}
    </Panel>
  )
}

function SettingStat({ label, value, tone = 'neutral' }: { label: string; value: string; tone?: 'neutral' | 'ok' | 'warn' }) {
  const color = tone === 'ok' ? 'var(--success)' : tone === 'warn' ? 'var(--warning)' : 'var(--muted)'
  return (
    <div style={{
      border: '1px solid var(--border)',
      borderRadius: 7,
      background: 'color-mix(in srgb, var(--surface) 82%, var(--bg))',
      padding: '8px 10px',
      display: 'grid',
      gap: 2,
      minWidth: 0,
    }}>
      <span style={{ color: 'var(--faint)', fontSize: 10, fontWeight: 800, textTransform: 'uppercase' }}>{label}</span>
      <span style={{ color, fontSize: 13, fontWeight: 800, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{value}</span>
    </div>
  )
}

function SettingPill({ tone, children }: { tone: 'warn' | 'ok'; children: React.ReactNode }) {
  const color = tone === 'warn' ? 'var(--warning)' : 'var(--success)'
  return (
    <span style={{
      color,
      border: `1px solid color-mix(in srgb, ${color} 48%, var(--border))`,
      background: `color-mix(in srgb, ${color} 10%, transparent)`,
      borderRadius: 999,
      padding: '1px 6px',
      fontSize: 10,
      fontWeight: 800,
      lineHeight: 1.5,
    }}>
      {children}
    </span>
  )
}

function ChangeReview({ settings, draft, customRcDirty, values, saving }: {
  settings: RtorrentSettingDescriptor[]
  draft: Record<string, string | number | boolean>
  customRcDirty: boolean
  values: Array<{ key: string; live: ProbeValue<string>; saved: string | null }>
  saving: boolean
}) {
  return (
    <details style={{
      border: '1px solid color-mix(in srgb, var(--warning) 44%, var(--border))',
      borderRadius: 8,
      background: 'color-mix(in srgb, var(--warning) 6%, var(--surface))',
      padding: '8px 10px',
    }}>
      <summary style={{ color: 'var(--text)', fontSize: 12, fontWeight: 800, cursor: 'pointer' }}>
        <LocalizedText source="Review autosave changes ({count})" parameters={{ count: settings.length + (customRcDirty ? 1 : 0) }} />{saving ? <LocalizedText source="- saving" /> : ''}
      </summary>
      <div style={{ display: 'grid', gap: 6, marginTop: 8 }}>
        {settings.map(setting => {
          const row = values.find(value => value.key === setting.key)
          const base = baselineValue(setting, row?.saved ?? null, row?.live.value ?? null)
          const next = draft[setting.key] ?? base
          return (
            <div key={setting.key} style={{
              display: 'grid',
              gridTemplateColumns: 'minmax(140px, 1fr) minmax(90px, auto) minmax(90px, auto) auto',
              gap: 8,
              alignItems: 'center',
              border: '1px solid var(--border)',
              borderRadius: 6,
              background: 'var(--surface)',
              padding: '7px 8px',
              fontSize: 11,
              minWidth: 0,
            }}>
              <span title={setting.command} {...interfaceTextProps(setting.label)} style={{ color: 'var(--text)', fontWeight: 800, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{translateInterface(setting.label)}</span>
              <span style={{ color: 'var(--faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                <LocalizedText source="From {value}" parameters={{ value: formatSettingValue(base, setting.unit) }} />
              </span>
              <span style={{ color: 'var(--warning)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                <LocalizedText source="To {value}" parameters={{ value: formatSettingValue(next, setting.unit) }} />
              </span>
              <SettingPill tone={setting.restart_required ? 'warn' : 'ok'}><LocalizedText source={setting.restart_required ? 'Restart required' : 'Live'} /></SettingPill>
            </div>
          )
        })}
        {customRcDirty && (
          <div style={{
            border: '1px solid var(--border)',
            borderRadius: 6,
            background: 'var(--surface)',
            padding: '7px 8px',
            color: 'var(--warning)',
            fontSize: 11,
            fontWeight: 800,
          }}>
            <LocalizedText source="Custom rTorrent lines edited" /></div>
        )}
      </div>
    </details>
  )
}

type SettingsViewFilter = 'all' | 'edited' | 'restart' | 'live' | 'unavailable'

function ViewFilter({ value, onChange, dirtyCount, restartCount, liveCount, unavailableCount }: {
  value: SettingsViewFilter
  onChange: (value: SettingsViewFilter) => void
  dirtyCount: number
  restartCount: number
  liveCount: number
  unavailableCount: number
}) {
  const options: Array<[SettingsViewFilter, string, number | undefined, string]> = [
    ['all', 'All', undefined, 'Show every managed setting'],
    ['edited', 'Edited: {count}', dirtyCount, 'Show managed settings pending autosave'],
    ['restart', 'Restart required: {count}', restartCount, 'Show settings that need restart'],
    ['live', 'Live: {count}', liveCount, 'Show settings that can apply live'],
    ['unavailable', 'Unavailable: {count}', unavailableCount, 'Show settings without a live readback'],
  ]
  return (
    <div style={{ display: 'grid', gap: 5, minWidth: 0 }}>
      <span style={{ color: 'var(--text)', fontSize: 12, fontWeight: 800 }}><LocalizedText source="View" /></span>
      <div role="group" aria-label={translateInterface("Settings view filter")} style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(96px, 1fr))', gap: 5 }} {...interfaceTextProps("Settings view filter")}>
        {options.map(([optionValue, labelSource, count, titleSource]) => {
          const label = translateInterface(labelSource, count === undefined ? {} : { count: count.toLocaleString(getRegionalLocale()) })
          const title = translateInterface(titleSource)
          const active = value === optionValue
          return (
            <button
              key={optionValue}
              type="button"
              title={title}
              aria-pressed={active}
              onClick={() => onChange(optionValue)}
              style={{
                border: `1px solid ${active ? 'var(--accent)' : 'var(--border-strong)'}`,
                borderRadius: 6,
                background: active ? 'var(--accent-soft)' : 'var(--bg)',
                color: active ? 'var(--accent-text)' : 'var(--muted)',
                padding: '7px 8px',
                fontSize: 11,
                fontWeight: 800,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}
            >
              {label}
            </button>
          )
        })}
      </div>
    </div>
  )
}

function NumericKnob({ setting, value, describedBy, onChange, onPreset }: {
  setting: RtorrentSettingDescriptor
  value: number
  describedBy: string
  onChange: (value: number) => void
  onPreset: (value: number) => void
}) {
  const min = setting.minimum ?? 0
  const max = setting.maximum ?? Math.max(value, 100)
  const percent = Math.max(0, Math.min(100, ((value - min) / Math.max(1, max - min)) * 100))
  const dial = `conic-gradient(var(--accent) ${percent}%, var(--surface-2) 0)`

  return (
    <div style={{
      display: 'grid',
      gridTemplateColumns: '72px minmax(0, 1fr)',
      gap: 10,
      alignItems: 'center',
      minWidth: 0,
    }}>
      <div aria-hidden="true" style={{
        width: 64,
        height: 64,
        borderRadius: '50%',
        background: dial,
        border: '1px solid var(--border-strong)',
        display: 'grid',
        placeItems: 'center',
        boxShadow: 'inset 0 1px 0 rgba(255,255,255,0.04)',
      }}>
        <div style={{
          width: 42,
          height: 42,
          borderRadius: '50%',
          background: 'var(--bg)',
          border: '1px solid var(--border)',
          display: 'grid',
          placeItems: 'center',
          color: 'var(--text)',
          fontSize: 11,
          fontWeight: 900,
        }}>
          {Math.round(percent)}%
        </div>
      </div>
      <div style={{ display: 'grid', gap: 6, minWidth: 0 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8, color: 'var(--faint)', fontSize: 10 }}>
          <span>{min.toLocaleString(getRegionalLocale())}</span>
          <span>{max.toLocaleString(getRegionalLocale())}</span>
        </div>
        <input
          type="range"
          min={min}
          max={max}
          step={1}
          value={Number.isFinite(value) ? value : min}
          onChange={e => onChange(Number(e.target.value))}
          style={{ width: '100%', accentColor: 'var(--accent)' }}
          aria-label={translateInterface('{setting} slider', { setting: translateInterface(setting.label) })}
          aria-describedby={describedBy}
          aria-valuetext={formatSettingValue(value, setting.unit)}
        />
        <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) auto', gap: 6, alignItems: 'center' }}>
          <input
            type="number"
            min={min}
            max={max}
            step={1}
            value={Number.isFinite(value) ? value : min}
            onChange={e => onChange(Number(e.target.value))}
            style={inputStyle}
            aria-label={translateInterface('{setting} value', { setting: translateInterface(setting.label) })}
            aria-describedby={describedBy}
          />
          {setting.unit && <span style={{ color: 'var(--faint)', fontSize: 11, fontWeight: 800 }}>{setting.unit}</span>}
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', gap: 5 }}>
          <button type="button" aria-label={translateInterface('Set {setting} to minimum {value}', { setting: translateInterface(setting.label), value: formatSettingValue(min, setting.unit) })} onClick={() => onPreset(min)} style={miniPresetButtonStyle}><LocalizedText source="Min" /></button>
          <button type="button" aria-label={translateInterface('Set {setting} to default {value}', { setting: translateInterface(setting.label), value: formatSettingValue(inputValue(setting.value_type, setting.default_value), setting.unit) })} onClick={() => onPreset(Number(inputValue(setting.value_type, setting.default_value)))} style={miniPresetButtonStyle}><LocalizedText source="Default" /></button>
          <button type="button" aria-label={translateInterface('Set {setting} to maximum {value}', { setting: translateInterface(setting.label), value: formatSettingValue(max, setting.unit) })} onClick={() => onPreset(max)} style={miniPresetButtonStyle}><LocalizedText source="Max" /></button>
        </div>
      </div>
    </div>
  )
}

function BooleanKnob({ setting, value, describedBy, onChange }: {
  setting: RtorrentSettingDescriptor
  value: boolean
  describedBy: string
  onChange: (value: boolean) => void
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={value}
      aria-label={translateInterface(setting.label)}
      aria-describedby={describedBy}
      onClick={() => onChange(!value)}
      style={{
        width: '100%',
        border: `1px solid ${value ? 'var(--accent)' : 'var(--border-strong)'}`,
        borderRadius: 7,
        background: value ? 'var(--accent-soft)' : 'var(--bg)',
        color: value ? 'var(--accent-text)' : 'var(--muted)',
        padding: 8,
        display: 'grid',
        gridTemplateColumns: '54px 1fr',
        gap: 10,
        alignItems: 'center',
        textAlign: 'left',
      }}
    >
      <span style={{
        width: 48,
        height: 26,
        borderRadius: 999,
        border: `1px solid ${value ? 'var(--accent)' : 'var(--border-strong)'}`,
        background: value ? 'color-mix(in srgb, var(--accent) 35%, var(--surface))' : 'var(--surface-2)',
        padding: 2,
        boxSizing: 'border-box',
        display: 'flex',
        justifyContent: value ? 'flex-end' : 'flex-start',
      }}>
        <span style={{
          width: 20,
          height: 20,
          borderRadius: '50%',
          background: value ? 'var(--accent-text)' : 'var(--faint)',
          boxShadow: '0 1px 4px var(--shadow)',
        }} />
      </span>
      <span style={{ fontSize: 13, fontWeight: 800 }}><LocalizedText source={value ? 'On' : 'Off'} /></span>
    </button>
  )
}

function SegmentedSetting({ label, value, options, describedBy, onChange }: {
  label: string
  value: string
  options: Array<[string, string]>
  describedBy: string
  onChange: (value: string) => void
}) {
  return (
    <div role="group" aria-label={label} aria-describedby={describedBy} style={{ display: 'grid', gridTemplateColumns: `repeat(${options.length}, minmax(0, 1fr))`, gap: 5 }}>
      {options.map(([optionValue, label]) => {
        const active = value === optionValue
        return (
          <button
            key={optionValue}
            type="button"
            aria-pressed={active}
            onClick={() => onChange(optionValue)}
            style={{
              border: `1px solid ${active ? 'var(--accent)' : 'var(--border-strong)'}`,
              borderRadius: 6,
              background: active ? 'var(--accent-soft)' : 'var(--bg)',
              color: active ? 'var(--accent-text)' : 'var(--muted)',
              padding: '7px 8px',
              fontSize: 12,
              fontWeight: 800,
            }}
          >
            <LocalizedText source={label} />
          </button>
        )
      })}
    </div>
  )
}

function Readout({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div style={{ minWidth: 0, display: 'grid', gap: 2 }}>
      <span style={{ color: 'var(--faint)', fontSize: 9, fontWeight: 800, textTransform: 'uppercase' }}><LocalizedText source={label} /></span>
      <span title={value} style={{
        color: 'var(--muted)',
        fontSize: 10,
        fontFamily: mono ? 'ui-monospace, SFMono-Regular, Menlo, monospace' : undefined,
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
      }}>
        {value}
      </span>
    </div>
  )
}

function formatSettingValue(value: string | number | boolean, unit: string | null): string {
  return `${String(value)}${unit ? ` ${unit}` : ''}`
}

interface SettingGroup {
  name: string
  description: string
  settings: RtorrentSettingDescriptor[]
}

function groupSettings(settings: RtorrentSettingDescriptor[]): SettingGroup[] {
  const groups: SettingGroup[] = [
    { name: 'Transfer Slots', description: 'Global and per-torrent concurrency limits.', settings: [] },
    { name: 'Tracker HTTP', description: 'Tracker request load, connection reuse, and DNS behavior.', settings: [] },
    { name: 'Peer Discovery', description: 'DHT, PEX, and UDP tracker discovery controls.', settings: [] },
    { name: 'Storage And Session', description: 'Cache, file/socket ceilings, hashing, and session persistence.', settings: [] },
    { name: 'Advanced', description: 'Settings that do not fit a narrower operational group.', settings: [] },
  ]
  const byName = new Map(groups.map(group => [group.name, group]))
  for (const setting of settings) {
    byName.get(settingCategory(setting))?.settings.push(setting)
  }
  return groups.filter(group => group.settings.length > 0)
}

function groupDomId(name: string): string {
  return `rt-setting-group-${name.replace(/\W+/g, '-').toLowerCase()}`
}

function settingCategory(setting: RtorrentSettingDescriptor): SettingGroup['name'] {
  if (setting.key.includes('uploads') || setting.key.includes('downloads')) return 'Transfer Slots'
  if (setting.key.startsWith('http_') || setting.key === 'trackers_numwant') return 'Tracker HTTP'
  if (setting.key === 'dht_mode' || setting.key === 'pex' || setting.key === 'udp_trackers') return 'Peer Discovery'
  if (setting.key.includes('memory') || setting.key.includes('files') || setting.key.includes('sockets') || setting.key.includes('hash') || setting.key.includes('session')) return 'Storage And Session'
  return 'Advanced'
}

function settingDescription(setting: RtorrentSettingDescriptor, t: (source: string, parameters?: Record<string, string | number>) => string = translateInterface): string {
  const bounds = setting.value_type === 'int' && setting.minimum !== null && setting.maximum !== null
    ? t('Range: {minimum}–{maximum}{unit}.', {
      minimum: setting.minimum.toLocaleString(getRegionalLocale()),
      maximum: setting.maximum.toLocaleString(getRegionalLocale()),
      unit: setting.unit ? ` ${setting.unit}` : '',
    })
    : ''
  const apply = setting.restart_required
    ? t('Saved immediately; daemon restart required to take effect.')
    : t('Saved and applied live when supported by rTorrent.')
  return [t('{command} via {setter}.', { command: setting.command, setter: setting.setter }), bounds, apply].filter(Boolean).join(' ')
}

function baselineValue(setting: RtorrentSettingDescriptor, saved: string | null, live: string | null): string | number | boolean {
  return inputValue(setting.value_type, saved ?? live ?? setting.default_value)
}

function sameSettingValue(left: unknown, right: unknown): boolean {
  if (typeof left === 'number' || typeof right === 'number') return Number(left) === Number(right)
  return String(left) === String(right)
}

function sameDraft(
  left: Record<string, string | number | boolean>,
  right: Record<string, string | number | boolean>,
): boolean {
  const leftKeys = Object.keys(left)
  const rightKeys = Object.keys(right)
  if (leftKeys.length !== rightKeys.length) return false
  return rightKeys.every(key => sameSettingValue(left[key], right[key]))
}

function formatRelativeTime(ts: number, language: string, t: (source: string) => string): string {
  const seconds = Math.max(0, Math.round((Date.now() - ts) / 1000))
  if (seconds < 5) return t('just now')
  const formatter = new Intl.RelativeTimeFormat(language, { numeric: 'auto' })
  if (seconds < 60) return formatter.format(-seconds, 'second')
  return formatter.format(-Math.round(seconds / 60), 'minute')
}

function clampSettingValue(setting: RtorrentSettingDescriptor, value: number): number {
  const min = setting.minimum ?? Number.NEGATIVE_INFINITY
  const max = setting.maximum ?? Number.POSITIVE_INFINITY
  if (!Number.isFinite(value)) return Number.isFinite(min) ? min : 0
  return Math.max(min, Math.min(max, value))
}

function inputValue(type: string, value: unknown): string | number | boolean {
  if (type === 'bool') return value === true || value === 'true' || value === 'yes' || value === '1'
  if (typeof value === 'number' || typeof value === 'boolean') return value
  const text = String(value ?? '')
  const numeric = Number(text.replace(/M$/, ''))
  return Number.isFinite(numeric) && type === 'int' ? numeric : text
}

const inputStyle: React.CSSProperties = {
  minWidth: 0,
  width: '100%',
  boxSizing: 'border-box',
  background: 'var(--bg)',
  border: '1px solid var(--border-strong)',
  borderRadius: 5,
  color: 'var(--text)',
  padding: '5px 8px',
  fontSize: 12,
  outline: 'none',
}

const readoutGridStyle: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fit, minmax(84px, 1fr))',
  gap: 6,
}

const miniPresetButtonStyle: React.CSSProperties = {
  border: '1px solid var(--border-strong)',
  borderRadius: 5,
  background: 'var(--bg)',
  color: 'var(--muted)',
  padding: '4px 6px',
  fontSize: 10,
  fontWeight: 800,
}

const visuallyHiddenStyle: React.CSSProperties = {
  position: 'absolute',
  width: 1,
  height: 1,
  padding: 0,
  margin: -1,
  overflow: 'hidden',
  clip: 'rect(0, 0, 0, 0)',
  whiteSpace: 'nowrap',
  border: 0,
}

function smallButtonStyle(enabled: boolean): React.CSSProperties {
  return {
    border: '1px solid var(--border-strong)',
    borderRadius: 5,
    background: enabled ? 'var(--surface-2)' : 'transparent',
    color: enabled ? 'var(--muted)' : 'var(--faint)',
    padding: '3px 8px',
    fontSize: 11,
    fontWeight: 800,
    cursor: enabled ? 'pointer' : 'default',
    opacity: enabled ? 1 : 0.55,
  }
}

function buttonStyle(border: string, background: string, color: string, enabled = true): React.CSSProperties {
  return {
    background: enabled ? background : 'var(--surface-2)',
    border: `1px solid ${border}`,
    borderRadius: 5,
    color: enabled ? color : 'var(--faint)',
    padding: '6px 10px',
    fontSize: 12,
    fontWeight: 800,
    cursor: enabled ? 'pointer' : 'default',
    opacity: enabled ? 1 : 0.6,
  }
}

function CommandIndex({ commands }: { commands?: { ok: boolean; count: number; commands: string[]; error: string | null } }) {
  const [filter, setFilter] = useState('')
  const filtered = useMemo(() => {
    const needle = filter.trim().toLowerCase()
    if (!commands?.ok || !needle) return commands?.commands ?? []
    return commands.commands.filter(command => command.toLowerCase().includes(needle))
  }, [commands, filter])
  return (
    <Panel wide>
      <Subhead><LocalizedText source="XML-RPC Command Interface" /></Subhead>
      {!commands && <div style={{ display: 'grid', gap: 7 }}>
        <span className="tng-skeleton" style={{ width: '42%', height: 10 }} />
        <span className="tng-skeleton" style={{ width: '72%', height: 8 }} />
      </div>}
      {commands && !commands.ok && <InlineNotice><LocalizedText source="Command index unavailable" /></InlineNotice>}
      {commands?.ok && (
        <details>
          <summary style={{ color: 'var(--text)', fontSize: 12, cursor: 'pointer' }}>{commands.count} <LocalizedText source="commands exposed by rTorrent" /></summary>
          <div style={{ marginTop: 8, display: 'flex', alignItems: 'center', gap: 8 }}>
            <input
              value={filter}
              onChange={e => setFilter(e.target.value)}
              aria-label={translateInterface("Filter XMLRPC commands")}
              placeholder={translateInterface("Filter commands")}
              style={{
                minWidth: 0, flex: '1 1 220px', background: 'var(--bg)', border: '1px solid var(--border-strong)',
                borderRadius: 5, color: 'var(--text)', padding: '5px 8px', fontSize: 12,
              }}
            {...interfaceTextProps("Filter XMLRPC commands")} />
            <span style={{ color: 'var(--faint)', fontSize: 11, whiteSpace: 'nowrap' }}>
              <LocalizedText source="Shown: {count}" parameters={{ count: filtered.length.toLocaleString(getRegionalLocale()) }} />
            </span>
          </div>
          <div style={{
            marginTop: 8,
            maxHeight: 180,
            overflow: 'auto',
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))',
            gap: 4,
            fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
            fontSize: 11,
            color: 'var(--muted)',
          }}>
            {filtered.map(command => <div key={command} title={command} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{command}</div>)}
          </div>
        </details>
      )}
    </Panel>
  )
}

function Panel({ children, wide }: { children: React.ReactNode; wide?: boolean }) {
  return (
    <div className="tng-card tng-engine-panel" style={{
      gridColumn: wide ? '1 / -1' : undefined,
      border: '1px solid var(--border)',
      borderRadius: 7,
      background: 'var(--surface)',
      padding: 12,
      minWidth: 0,
    }}>
      {children}
    </div>
  )
}

function Rows({ rows }: { rows: [string, string][] }) {
  const { t } = useInterfaceLanguage()
  const localizableValues = new Set(['yes', 'no', 'not applicable', 'not declared', 'none declared', 'none', 'default', 'unknown', 'unavailable', 'torrentng client'])
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '160px minmax(0, 1fr)', gap: '6px 12px', fontSize: 12 }}>
      {rows.map(([k, v]) => (
        <div key={k} className="tng-engine-kv" style={{ display: 'contents' }}>
          <div style={{ color: 'var(--faint)' }}><LocalizedText source={k} /></div>
          <div title={v} style={{ color: 'var(--text)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{localizableValues.has(v.toLowerCase()) ? t(v) : v}</div>
        </div>
      ))}
    </div>
  )
}

function Subhead({ children }: { children: React.ReactNode }) {
  return <h3 style={{
    fontSize: 11, fontWeight: 800, textTransform: 'uppercase', color: 'var(--accent-text)',
    margin: '0 0 8px', display: 'flex', alignItems: 'center', gap: 6,
  }}>
    <span aria-hidden="true" style={{ width: 6, height: 6, borderRadius: 999, background: 'var(--accent)', flexShrink: 0 }} />
    {children}
  </h3>
}

function InlineNotice({ children }: { children: React.ReactNode }) {
  return <div role="alert" aria-live="assertive" style={{
    color: 'var(--danger)',
    background: 'color-mix(in srgb, var(--danger) 9%, var(--surface))',
    border: '1px solid color-mix(in srgb, var(--danger) 45%, var(--border))',
    borderRadius: 6,
    padding: '8px 9px',
    fontSize: 12,
  }}>{children}</div>
}

function Badge({ ok, text }: { ok: boolean; text: string }) {
  return (
    <span style={{
      border: '1px solid ' + (ok ? 'color-mix(in srgb, var(--success) 45%, var(--border))' : 'color-mix(in srgb, var(--danger) 50%, var(--border))'),
      color: ok ? 'var(--success)' : 'var(--danger)',
      background: ok ? 'color-mix(in srgb, var(--success) 10%, transparent)' : 'color-mix(in srgb, var(--danger) 10%, transparent)',
      borderRadius: 999,
      padding: '1px 7px',
      fontSize: 10,
      whiteSpace: 'nowrap',
    }}>{text}</span>
  )
}

function val<T>(probe: ProbeValue<T>): string {
  if (!probe.ok) return 'unavailable'
  if (probe.value === null || probe.value === undefined) return ''
  return String(probe.value)
}

function suffix(probe: ProbeValue<number>, unit: string): string {
  const v = val(probe)
  return v ? `${v}${unit}` : v
}

function bool(probe: ProbeValue<boolean>): string {
  if (!probe.ok) return 'unavailable'
  return probe.value ? 'enabled' : 'disabled'
}
