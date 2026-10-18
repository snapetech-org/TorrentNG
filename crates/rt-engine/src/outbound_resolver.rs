//! Process-wide admission for outbound DNS work.
//!
//! Per-request timeouts and answer caps do not bound concurrent resolver
//! calls when many torrents announce or bootstrap at once. Tracker, webseed,
//! and DHT bootstrap lookups share this small, non-queuing budget.

use tokio::sync::{Semaphore, SemaphorePermit};

pub(crate) const DNS_LOOKUP_CONCURRENCY: usize = 32;

pub(crate) struct DnsLookupBudget {
    permits: Semaphore,
}

impl DnsLookupBudget {
    const fn new(limit: usize) -> Self {
        Self {
            permits: Semaphore::const_new(limit),
        }
    }

    pub(crate) fn try_acquire(&self) -> Result<SemaphorePermit<'_>, ()> {
        self.permits.try_acquire().map_err(|_| ())
    }
}

static DNS_LOOKUP_BUDGET: DnsLookupBudget = DnsLookupBudget::new(DNS_LOOKUP_CONCURRENCY);

pub(crate) fn try_acquire_dns_lookup() -> Result<SemaphorePermit<'static>, ()> {
    DNS_LOOKUP_BUDGET.try_acquire()
}

#[cfg(test)]
mod tests {
    use super::DnsLookupBudget;

    #[test]
    fn dns_budget_rejects_excess_work_without_a_wait_queue() {
        let budget = DnsLookupBudget::new(2);
        let first = budget.try_acquire().unwrap();
        let second = budget.try_acquire().unwrap();
        assert!(budget.try_acquire().is_err());

        drop(first);
        assert!(budget.try_acquire().is_ok());
        drop(second);
    }
}
