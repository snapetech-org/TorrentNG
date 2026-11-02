import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { LocalizedErrorText, LocalizedServerEventKind, LocalizedServerMessage, LocalizedText } from './LocalizedText'
import { applyInitialInterfaceLanguage } from './interfaceLocale'

beforeEach(() => {
  localStorage.setItem('torrentng.interface-language', 'en')
  applyInitialInterfaceLanguage()
})

afterEach(() => cleanup())

describe('server-originated localized text', () => {
  it('preserves unmatched diagnostics that collide with a translated UI key', async () => {
    localStorage.setItem('torrentng.interface-language', 'es')
    applyInitialInterfaceLanguage()
    render(<>
      <span data-testid="ui-label"><LocalizedText source="Downloading" /></span>
      <span data-testid="runtime-detail"><LocalizedErrorText message="Downloading" /></span>
      <LocalizedErrorText message="Invalid username/password or API token" />
    </>)

    await waitFor(() => expect(screen.getByTestId('ui-label').textContent).toBe('Descargando'))
    expect(screen.getByTestId('runtime-detail').textContent).toBe('Downloading')
    expect(screen.getByText('Nombre de usuario/contraseña o token API no válido')).toBeTruthy()
  })

  it('preserves external and unknown-kind log payloads even when they match app copy', async () => {
    localStorage.setItem('torrentng.interface-language', 'es')
    applyInitialInterfaceLanguage()
    render(<>
      <span data-testid="app-event"><LocalizedServerMessage kind="torrent_removed" message="torrent removed" /></span>
      <span data-testid="external-log"><LocalizedServerMessage kind="rtorrent_log" message="torrent removed" /></span>
      <span data-testid="unknown-event"><LocalizedServerMessage kind="provider_event_v42" message="torrent removed" /></span>
    </>)

    await waitFor(() => expect(screen.getByTestId('app-event').textContent).toBe('Torrent retirado'))
    expect(screen.getByTestId('external-log').textContent).toBe('torrent removed')
    expect(screen.getByTestId('unknown-event').textContent).toBe('torrent removed')
  })

  it('localizes a fixed server error through its source key', () => {
    render(<LocalizedErrorText message="info hash must not be empty" />)
    expect(screen.getByText('info hash must not be empty')).toBeTruthy()
  })

  it('routes exact Swarm Switch client errors through source localization and keeps unknown errors raw', () => {
    const { container } = render(<>
      <LocalizedErrorText message="Could not read the public torrent file." />
      <LocalizedErrorText message="unregistered provider diagnostic" />
    </>)
    expect(container.querySelector('span[lang="en"]')?.textContent).toBe('Could not read the public torrent file.')
    expect(screen.getByText('unregistered provider diagnostic')).toBeTruthy()
  })

  it('keeps dynamic server error values intact and isolated for bidirectional text', () => {
    const dynamicValue = '0012 / عربى / <piece>'
    render(<LocalizedErrorText message={`info hash contains ${dynamicValue} bytes; maximum is 256`} />)

    const value = screen.getByText(dynamicValue)
    expect(value.tagName).toBe('BDI')
    expect(value.getAttribute('dir')).toBe('ltr')
  })

  it('keeps translated dynamic values reactive and isolates them inside Arabic text', async () => {
    const dynamicValue = 'Episode 001 / عربى'
    const { container } = render(<LocalizedText source="Files ({count})" parameters={{ count: dynamicValue }} />)

    act(() => {
      localStorage.setItem('torrentng.interface-language', 'ar')
      applyInitialInterfaceLanguage()
    })

    await waitFor(() => expect(container.querySelector('span[lang="ar"]')?.textContent).toBe('الملفات (Episode 001 / عربى)'))
    const text = container.querySelector('span[lang="ar"]')
    const value = screen.getByText(dynamicValue)
    expect(text?.getAttribute('dir')).toBe('rtl')
    expect(value.tagName).toBe('BDI')
    expect(value.getAttribute('dir')).toBe('auto')
  })

  it('leaves unknown messages and event kinds unchanged', () => {
    const message = 'provider supplied detail: keep exactly as received'
    render(<>
      <span><LocalizedServerMessage message={message} kind="provider_event_v42" /></span>
      <span><LocalizedServerEventKind kind="provider_event_v42" /></span>
      <span><LocalizedServerEventKind kind="toString" /></span>
      <span><LocalizedServerEventKind kind="__proto__" /></span>
    </>)

    expect(screen.getByText(message)).toBeTruthy()
    expect(screen.getByText('provider_event_v42')).toBeTruthy()
    expect(screen.getByText('toString')).toBeTruthy()
    expect(screen.getByText('__proto__')).toBeTruthy()
  })

  it('uses the reviewed source label for known persisted event kinds', () => {
    render(<LocalizedServerEventKind kind="swarm_switched_public" />)
    expect(screen.getByText('Public recovery session started')).toBeTruthy()
  })
})
