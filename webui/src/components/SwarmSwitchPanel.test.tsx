import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SwarmSwitchPanel } from './SwarmSwitchPanel'
import type { SwarmComparison, SwarmStatus } from '../api/client'

const hash = 'a'.repeat(40)
const publicHash = 'b'.repeat(40)
let status: SwarmStatus
let comparison: SwarmComparison
let calls: { path: string; method: string; body: unknown }[]
let supported: boolean
let previewFailure: string | undefined
const response = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { 'content-type': 'application/json' } })

beforeEach(() => {
  calls = []; supported = true; previewFailure = undefined
  status = { original_hash: hash, public_hash: null, phase: 'private', can_switch: true, can_restore: false, dht_enabled: true, active_state: 'paused', verified_bytes: 0, total_bytes: 100 }
  comparison = { candidate: { original_hash: hash, public_hash: publicHash, public_v2_hash: null, piece_length: 16384, piece_count: 1, total_bytes: 100, files: 1, variant: 'remove_private' },
    public: { status: 'observed', observed_at: Math.floor(Date.now() / 1000), responding_nodes: 8, discovered_peers: 0, seed_estimate: null, peer_estimate: null, sampled_peers: 0, reachable_peers: 0, observed_seeds: 0, bounded: false },
    private: [{ tracker: 'https://tracker.example', seeds: 2, leechers: 3, observed_at: 1700000000, status: 'cached_or_unavailable', fresh: false }] }
  vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    calls.push({ path, method: init?.method ?? 'GET', body: init?.body ? JSON.parse(String(init.body)) : null })
    if (path.endsWith('/engine')) return response({ backend: { capabilities: { supports_swarm_switch: supported } } })
    if (path.endsWith('/preview')) return previewFailure ? response({ error: { message: previewFailure } }, 409) : response(comparison)
    if (path.endsWith('/public')) { status = { ...status, public_hash: publicHash, phase: 'public', can_switch: false, can_restore: true }; return response(status) }
    if (path.endsWith('/restore')) { status = { ...status, public_hash: null, phase: 'private', can_switch: true, can_restore: false }; return response(status) }
    if (path.endsWith('?delete_files=false')) { status = { ...status, phase: 'public_independent', original_hash: null, can_switch: false, can_restore: false }; return new Response(null, { status: 204 }) }
    return response(status)
  }))
})
afterEach(() => { cleanup(); vi.unstubAllGlobals() })
function mount() { const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } }); return render(<QueryClientProvider client={client}><SwarmSwitchPanel hash={hash} showUnsupported /></QueryClientProvider>) }

describe('Swarm Switch', () => {
  it('requires comparison, distinguishes zero from unknown, and submits the exact candidate', async () => {
    const user = userEvent.setup(); mount()
    const activate = await screen.findByRole('button', { name: 'Use public swarm' })
    expect((activate as HTMLButtonElement).disabled).toBe(true)
    await user.click(screen.getByRole('button', { name: 'Compare swarms' }))
    await screen.findByLabelText('Swarm comparison')
    expect(screen.getByText('Discovered peers').parentElement?.textContent).toContain('0')
    expect(screen.getByText('Estimated seeds').parentElement?.textContent).toContain('Unknown')
    expect(screen.getByText('Last known report; fresh scrape unavailable')).toBeTruthy()
    await user.click(activate)
    await screen.findByRole('button', { name: 'Restore private swarm' })
    expect(calls.find(c => c.path.endsWith('/public'))?.body).toEqual({ variant: 'remove_private', expected_public_hash: publicHash })
  })
  it('invalidates comparisons when the candidate changes', async () => {
    const user = userEvent.setup(); mount()
    await user.click(await screen.findByRole('button', { name: 'Compare swarms' }))
    await screen.findByLabelText('Swarm comparison')
    await user.selectOptions(screen.getByRole('combobox'), 'private_zero')
    expect(screen.queryByLabelText('Swarm comparison')).toBeNull()
    expect((screen.getByRole('button', { name: 'Use public swarm' }) as HTMLButtonElement).disabled).toBe(true)
  })
  it('shows why incompatible metadata was rejected', async () => {
    previewFailure = 'Piece hashes differ; this public torrent is not an identical payload'
    const user = userEvent.setup(); mount()
    await user.click(await screen.findByRole('button', { name: 'Compare swarms' }))
    expect((await screen.findByRole('alert')).textContent).toContain('Piece hashes differ')
    expect(calls.some(c => c.path.endsWith('/public'))).toBe(false)
  })
  it('does not offer restore for an independent public torrent', async () => {
    status = { ...status, original_hash: null, phase: 'public_independent', can_switch: false }
    mount(); await screen.findByText('This is an independent public torrent. There is no retained private torrent to restore.')
    expect(screen.queryByRole('button', { name: 'Restore private swarm' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Use public swarm' })).toBeNull()
  })
  it('deletes only the private copy and preserves payload data', async () => {
    status = { ...status, public_hash: publicHash, phase: 'public', can_restore: true, can_switch: false }
    const user = userEvent.setup(); mount()
    await user.click(await screen.findByRole('button', { name: 'Delete retained private copy…' }))
    await user.click(screen.getByRole('button', { name: 'Delete private copy, keep files' }))
    await waitFor(() => expect(calls.some(c => c.path.endsWith(`/torrents/${hash}?delete_files=false`))).toBe(true))
    expect(calls.some(c => c.path.includes('delete_files=true'))).toBe(false)
  })
  it('gates unsupported engines and stale observations', async () => {
    supported = false; mount()
    await screen.findByText('Swarm Switch requires a TorrentNG engine that supports this feature.')
    expect(calls.some(c => c.path.includes('/swarm-switch'))).toBe(false)
  })
  it('requires refreshing a stale public comparison', async () => {
    comparison.public.observed_at -= 120
    const user = userEvent.setup(); mount()
    await user.click(await screen.findByRole('button', { name: 'Compare swarms' }))
    await screen.findByText('This comparison is over a minute old. Refresh it before switching swarms.')
    expect((screen.getByRole('button', { name: 'Use public swarm' }) as HTMLButtonElement).disabled).toBe(true)
  })
  it('compares before restoring a stable public session, while allowing interrupted recovery', async () => {
    status = { ...status, public_hash: publicHash, phase: 'public', can_restore: true, can_switch: false }
    const user = userEvent.setup(); mount()
    const restore = await screen.findByRole('button', { name: 'Restore private swarm' })
    expect((restore as HTMLButtonElement).disabled).toBe(true)
    await user.click(screen.getByRole('button', { name: 'Compare swarms' }))
    await screen.findByLabelText('Swarm comparison')
    await user.click(restore)
    await waitFor(() => expect(calls.some(c => c.path.endsWith(`/torrents/${hash}/swarm-switch/restore`))).toBe(true))
    cleanup(); status = { ...status, phase: 'preparing', can_restore: true, can_switch: false }; mount()
    expect(((await screen.findByRole('button', { name: 'Restore private swarm' })) as HTMLButtonElement).disabled).toBe(false)
  })

})
