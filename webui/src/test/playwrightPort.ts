const DEFAULT_PORT = 4173
const RUNNER_PORT_START = 20_000
const RUNNER_PORT_COUNT = 40_000

type Environment = Readonly<Record<string, string | undefined>>

function hashSeed(seed: string): number {
  let hash = 0x811c9dc5
  for (let index = 0; index < seed.length; index += 1) {
    hash ^= seed.charCodeAt(index)
    hash = Math.imul(hash, 0x01000193)
  }
  return hash >>> 0
}

export function getPlaywrightPort(environment: Environment): number {
  if (environment.GITHUB_ACTIONS !== 'true') {
    return DEFAULT_PORT
  }

  const repository = environment.GITHUB_REPOSITORY
  const runId = environment.GITHUB_RUN_ID
  const runAttempt = environment.GITHUB_RUN_ATTEMPT
  if (
    !repository ||
    !/^[1-9]\d*$/.test(runId ?? '') ||
    !/^[1-9]\d*$/.test(runAttempt ?? '')
  ) {
    throw new Error(
      'GitHub Actions run identity is required to select a workflow-specific WebUI test port',
    )
  }

  const seed = `${repository}:${runId}:${runAttempt}`
  const offset = hashSeed(seed) % RUNNER_PORT_COUNT
  return RUNNER_PORT_START + offset
}
