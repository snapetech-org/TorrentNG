use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::packet::UtpPacket;

pub const DEFAULT_UTP_ENDPOINT_BUFFER_BYTES: usize = 64 * 1024 * 1024;

/// Current buffer reservations for one uTP endpoint generation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UtpMemoryUsage {
    pub reserved_bytes: usize,
    pub limit_bytes: usize,
    pub rejected_reservations: u64,
}

#[derive(Debug)]
pub(crate) struct UtpMemoryBudget {
    permits: Arc<Semaphore>,
    limit_bytes: usize,
    rejected_reservations: AtomicU64,
}

#[derive(Debug)]
pub(crate) struct UtpMemoryReservation {
    _permit: OwnedSemaphorePermit,
}

pub(crate) struct UtpBufferedPacket {
    pub(crate) packet: UtpPacket,
    pub(crate) _reservation: UtpMemoryReservation,
}

impl UtpMemoryBudget {
    pub(crate) fn new(limit_bytes: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(limit_bytes)),
            limit_bytes,
            rejected_reservations: AtomicU64::new(0),
        }
    }

    pub(crate) fn try_reserve(self: &Arc<Self>, bytes: usize) -> Option<UtpMemoryReservation> {
        let permits = u32::try_from(bytes).ok()?;
        match Arc::clone(&self.permits).try_acquire_many_owned(permits) {
            Ok(permit) => Some(UtpMemoryReservation { _permit: permit }),
            Err(_) => {
                self.rejected_reservations.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    pub(crate) async fn reserve(
        self: &Arc<Self>,
        bytes: usize,
    ) -> Result<UtpMemoryReservation, usize> {
        let permits = u32::try_from(bytes).map_err(|_| bytes)?;
        if bytes > self.limit_bytes {
            return Err(bytes);
        }
        Arc::clone(&self.permits)
            .acquire_many_owned(permits)
            .await
            .map(|permit| UtpMemoryReservation { _permit: permit })
            .map_err(|_| bytes)
    }

    pub(crate) fn usage(&self) -> UtpMemoryUsage {
        UtpMemoryUsage {
            reserved_bytes: self
                .limit_bytes
                .saturating_sub(self.permits.available_permits()),
            limit_bytes: self.limit_bytes,
            rejected_reservations: self.rejected_reservations.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UtpMemoryBudget;
    use std::sync::Arc;

    #[test]
    fn endpoint_budget_charges_concurrent_streams_and_releases_on_drop() {
        let budget = Arc::new(UtpMemoryBudget::new(100));
        let first = budget.try_reserve(60).expect("first reservation");
        assert_eq!(budget.usage().reserved_bytes, 60);

        assert!(budget.try_reserve(41).is_none());
        assert_eq!(budget.usage().rejected_reservations, 1);

        drop(first);
        assert_eq!(budget.usage().reserved_bytes, 0);
        let second = budget.try_reserve(100).expect("released budget reused");
        assert_eq!(budget.usage().reserved_bytes, 100);
        drop(second);
        assert_eq!(budget.usage().reserved_bytes, 0);
    }
}
