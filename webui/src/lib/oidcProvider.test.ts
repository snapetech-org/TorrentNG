import { describe, expect, it } from 'vitest'
import { getOidcProviderPresentation } from './oidcProvider'

describe('getOidcProviderPresentation', () => {
  it('returns the branded setup guide for a known provider', () => {
    expect(getOidcProviderPresentation('amazon-cognito')).toMatchObject({
      label: 'Amazon Cognito',
      icon: 'amazon-cognito',
      setupUrl: 'https://docs.aws.amazon.com/cognito/latest/developerguide/federation-endpoints.html',
    })
  })

  it('uses the Apple mark and setup guide for Apple sign in', () => {
    expect(getOidcProviderPresentation('apple-sign-in')).toMatchObject({
      label: 'Apple',
      icon: 'apple',
      setupUrl: 'https://developer.apple.com/documentation/signinwithapple/configuring-your-environment-for-sign-in-with-apple',
    })
  })

  it('uses generic branding for unknown and prototype property names', () => {
    expect(getOidcProviderPresentation('constructor').icon).toBe('generic')
    expect(getOidcProviderPresentation(null).label).toBe('identity provider')
  })
})
