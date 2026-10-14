import { Fragment, type ReactNode } from 'react'
import { useInterfaceLanguage } from './interfaceLocale'

type Props = {
  source: string
  parameters?: Record<string, string | number>
  spaceBefore?: boolean
  spaceAfter?: boolean
}

export function LocalizedText({ source, parameters = {}, spaceBefore = false, spaceAfter = false }: Props) {
  const { t, textProps } = useInterfaceLanguage()
  return (
    <span {...textProps(source)}>
      {spaceBefore ? ' ' : null}{t(source, parameters)}{spaceAfter ? ' ' : null}
    </span>
  )
}

export function LocalizedRichText({ source, parameters }: { source: string; parameters: Record<string, ReactNode> }) {
  const { t, textProps } = useInterfaceLanguage()
  const translated = t(source)
  const parts = translated.split(/(\{[a-zA-Z][a-zA-Z0-9_]*\})/g)
  return (
    <span {...textProps(source)}>
      {parts.map((part, index) => {
        const match = /^\{([a-zA-Z][a-zA-Z0-9_]*)\}$/.exec(part)
        if (!match) return <Fragment key={index}>{part}</Fragment>
        const value = parameters[match[1]]
        return value === undefined ? <Fragment key={index}>{part}</Fragment> : <bdi key={index} dir="ltr">{value}</bdi>
      })}
    </span>
  )
}

export function LocalizedErrorText({ message }: { message: string }) {
  const { t, textProps } = useInterfaceLanguage()
  let source = message
  let parameters: Record<string, string | number> = {}
  let path: string | null = null
  let method: string | null = null
  const apiError = /^(?:Error:\s*)?API (\d{3}): (\/\S*)$/.exec(message)
  if (apiError) {
    source = 'API request failed (HTTP {status}): {path}'
    path = apiError[2]
    parameters = { status: apiError[1], path: 'TNG_LOCALIZED_ERROR_PATH' }
  } else {
    const probeError = /^(\S+) returned a non-(string|integer|boolean) value$/.exec(message)
    if (probeError) {
      source = `{method} returned a non-${probeError[2]} value`
      method = probeError[1]
      parameters = { method: 'TNG_LOCALIZED_ERROR_METHOD' }
    } else {
      const retryError = /^Too many login attempts\. Try again in (\d+) seconds\.$/.exec(message)
      if (retryError) {
        source = 'Too many login attempts. Try again in {retryAfter} seconds.'
        parameters = { retryAfter: retryError[1] }
      } else if (/^(?:TypeError: )?Failed to fetch$/.test(message) || message === 'Load failed') {
        source = 'Network request failed. Check your connection.'
      }
    }
  }
  const translated = t(source, parameters)
  if (path === null && method === null) return <span {...textProps(source)}>{translated}</span>
  const marker = path === null ? 'TNG_LOCALIZED_ERROR_METHOD' : 'TNG_LOCALIZED_ERROR_PATH'
  const dynamicValue = path ?? method ?? ''
  const segments = translated.split(marker)
  return (
    <span {...textProps(source)}>
      {segments.flatMap((segment, index) => index === 0
        ? [segment]
        : [<bdi key={`error-value-${index}`} dir="ltr">{dynamicValue}</bdi>, segment])}
    </span>
  )
}
