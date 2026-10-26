type StorageUsage = {
  used_bytes: number
  total_bytes: number
  used_percent?: number | null
}

export function storageUsedPercent(storage: StorageUsage): number {
  const reported = storage.used_percent
  const percent = typeof reported === 'number' && Number.isFinite(reported)
    ? reported
    : Number.isFinite(storage.used_bytes) && Number.isFinite(storage.total_bytes) && storage.total_bytes > 0
      ? storage.used_bytes / storage.total_bytes * 100
      : 0

  return Number.isFinite(percent) ? Math.max(0, Math.min(100, percent)) : 0
}
