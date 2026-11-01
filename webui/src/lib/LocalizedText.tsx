import { Fragment, type ReactNode } from 'react'
import { useInterfaceLanguage } from './interfaceLocale'
import serverMessages from './server-messages.json'
import { compileServerMessageMatchers, matchServerMessage } from './serverMessageMatch'
import { SERVER_EVENT_KIND_LABELS } from './serverEventKinds'

const serverMessageMatchers = compileServerMessageMatchers(serverMessages)
const applicationErrorSources = new Set([
  'Could not read the public torrent file.',
  'Public metainfo exceeds 8 MiB.',
])

type Props = {
  source: string
  parameters?: Record<string, string | number>
  undParameters?: string[]
  parameterLanguages?: Record<string, string>
  parameterDirections?: Record<string, 'auto' | 'ltr'>
  spaceBefore?: boolean
  spaceAfter?: boolean
}

export function LocalizedText({ source, parameters = {}, undParameters = [], parameterLanguages = {}, parameterDirections = {}, spaceBefore = false, spaceAfter = false }: Props) {
  const { t, textProps } = useInterfaceLanguage()
  const parameterValues = new Map<string, string | number>()
  const undParameterMarkers = new Set<string>()
  const parameterLanguageMarkers = new Map<string, string>()
  const parameterDirectionMarkers = new Map<string, 'auto' | 'ltr'>()
  const markerParameters = Object.fromEntries(Object.entries(parameters).map(([name, value], index) => {
    const marker = `\uE000TNG_LOCALIZED_PARAMETER_${index}\uE001`
    const direction = parameterDirections[name]
    parameterValues.set(marker, value)
    if (undParameters.includes(name)) undParameterMarkers.add(marker)
    if (parameterLanguages[name]) parameterLanguageMarkers.set(marker, parameterLanguages[name])
    if (direction) parameterDirectionMarkers.set(marker, direction)
    return [name, marker]
  }))
  const translated = t(source, markerParameters)
  const markerPattern = parameterValues.size
    ? new RegExp(`(${[...parameterValues.keys()].join('|')})`, 'g')
    : null
  const parts = markerPattern ? translated.split(markerPattern) : [translated]
  return (
    <span {...textProps(source)}>
      {spaceBefore ? ' ' : null}{parts.map((part, index) => parameterValues.has(part)
        ? <bdi key={`parameter-${index}`} dir={parameterDirectionMarkers.get(part) ?? 'auto'} lang={undParameterMarkers.has(part) ? 'und' : parameterLanguageMarkers.get(part)}>{parameterValues.get(part)}</bdi>
        : <Fragment key={`text-${index}`}>{part}</Fragment>)}{spaceAfter ? ' ' : null}
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
        return value === undefined ? <Fragment key={index}>{part}</Fragment> : <bdi key={index} dir="ltr" lang="und">{value}</bdi>
      })}
    </span>
  )
}

export function LocalizedServerMessage({ message, kind }: { message: string; kind: string }) {
  if (kind === 'rtorrent_log' || !Object.hasOwn(SERVER_EVENT_KIND_LABELS, kind)) return <>{message}</>
  const match = matchServerMessage(message, serverMessageMatchers)
  if (!match) return <>{message}</>
  return <LocalizedRichText source={match.source} parameters={match.tokens} />
}

export function LocalizedServerEventKind({ kind }: { kind: string }) {
  const source = Object.hasOwn(SERVER_EVENT_KIND_LABELS, kind)
    ? SERVER_EVENT_KIND_LABELS[kind as keyof typeof SERVER_EVENT_KIND_LABELS]
    : undefined
  return source ? <LocalizedText source={source} /> : <>{kind}</>
}

export function LocalizedErrorText({ message }: { message: string }) {
  const { t, textProps } = useInterfaceLanguage()
  const serverMatch = matchServerMessage(message, serverMessageMatchers)
  if (serverMatch) {
    return <LocalizedRichText source={serverMatch.source} parameters={serverMatch.tokens} />
  }
  if (applicationErrorSources.has(message)) return <LocalizedText source={message} />
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
      } else if (message === 'Invalid username/password or API token' ||
                 message === 'Too many login attempts. Please try again later.') {
        // These exact AuthError messages are authored by api/client.ts login().
        source = message
      } else {
        return <bdi dir="auto" lang="und">{message}</bdi>
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
        : [<bdi key={`error-value-${index}`} dir="ltr" lang="und">{dynamicValue}</bdi>, segment])}
    </span>
  )
}
