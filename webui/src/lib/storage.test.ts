import { describe, expect, it } from 'vitest'
import { storageUsedPercent } from './storage'

describe('storageUsedPercent', () => {
  it('uses a finite percentage reported by the API', () => {
    expect(storageUsedPercent({ used_bytes: 20, total_bytes: 100, used_percent: 25.5 })).toBe(25.5)
  })

  it('derives the percentage from byte counts when the API omits it', () => {
    expect(storageUsedPercent({ used_bytes: 75, total_bytes: 100 })).toBe(75)
  })

  it('derives the percentage when the API percentage is not finite', () => {
    expect(storageUsedPercent({ used_bytes: 25, total_bytes: 100, used_percent: Number.NaN })).toBe(25)
  })

  it('returns zero when the total capacity is unavailable', () => {
    expect(storageUsedPercent({ used_bytes: 0, total_bytes: 0 })).toBe(0)
  })

  it('clamps percentages to the progress bar range', () => {
    expect(storageUsedPercent({ used_bytes: 200, total_bytes: 100, used_percent: 200 })).toBe(100)
    expect(storageUsedPercent({ used_bytes: 0, total_bytes: 100, used_percent: -10 })).toBe(0)
  })
})
