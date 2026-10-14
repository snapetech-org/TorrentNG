import { describe, expect, it } from 'vitest'
import { getPlaywrightPort } from './playwrightPort'

function githubEnvironment(
  repository: string,
  runId: string,
  runAttempt = '1',
): Record<string, string | undefined> {
  return {
    GITHUB_ACTIONS: 'true',
    GITHUB_REPOSITORY: repository,
    GITHUB_RUN_ID: runId,
    GITHUB_RUN_ATTEMPT: runAttempt,
  }
}

describe('getPlaywrightPort', () => {
  it('keeps local browser runs on the usual development port', () => {
    expect(getPlaywrightPort({})).toBe(4173)
  })

  it('assigns a stable high port from the workflow repository and run identity', () => {
    const firstRun = githubEnvironment('snapetech-org/TorrentNG', '38468401065')
    const retry = githubEnvironment('snapetech-org/TorrentNG', '38468401065', '2')
    const otherRepository = githubEnvironment(
      'snapetech-org/BookshelfNG',
      '38468401065',
    )
    const ports = [
      getPlaywrightPort(firstRun),
      getPlaywrightPort(retry),
      getPlaywrightPort(otherRepository),
    ]

    expect(ports.every((port) => port >= 20_000 && port < 60_000)).toBe(true)
    expect(new Set(ports).size).toBe(ports.length)
    expect(getPlaywrightPort(firstRun)).toBe(ports[0])
  })

  it('fails when a GitHub Actions run has no complete identity', () => {
    expect(() => getPlaywrightPort({ GITHUB_ACTIONS: 'true' })).toThrow(
      'GitHub Actions run identity is required to select a workflow-specific WebUI test port',
    )
  })
})
