import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { LocalizedErrorText, LocalizedServerEventKind, LocalizedServerMessage } from './LocalizedText'
import { applyInitialInterfaceLanguage } from './interfaceLocale'

beforeEach(() => {
  localStorage.setItem('torrentng.interface-language', 'en')
  applyInitialInterfaceLanguage()
})

afterEach(() => cleanup())

describe('server-originated localized text', () => {
  it('localizes a fixed server error through its source key', () => {
    render(<LocalizedErrorText message="info hash must not be empty" />)
    expect(screen.getByText('info hash must not be empty')).toBeTruthy()
  })

  it('keeps dynamic server error values intact and isolated for bidirectional text', () => {
    const dynamicValue = '0012 / عربى / <piece>'
    render(<LocalizedErrorText message={`info hash contains ${dynamicValue} bytes; maximum is 256`} />)

    const value = screen.getByText(dynamicValue)
    expect(value.tagName).toBe('BDI')
    expect(value.getAttribute('dir')).toBe('ltr')
  })

  it('leaves unknown messages and event kinds unchanged', () => {
    const message = 'provider supplied detail: keep exactly as received'
    render(<>
      <span><LocalizedServerMessage message={message} /></span>
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
