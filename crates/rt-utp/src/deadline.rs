use std::{
    collections::{BTreeSet, HashMap},
    future::pending,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::sleep_until,
};

use crate::{error::UtpError, packet::UtpPacket};

const DEADLINE_COMMAND_CAPACITY: usize = 1024;

type DeadlineKey = (Instant, u64, u64);

enum DeadlineCommand {
    Schedule {
        id: u64,
        generation: u64,
        deadline: Option<Instant>,
        ready: watch::Sender<Option<u64>>,
    },
    Cancel {
        id: u64,
        generation: u64,
    },
}

/// One timer task and ordered deadline set per shared UDP endpoint.
#[derive(Clone)]
pub(crate) struct UtpDeadlineScheduler {
    tx: mpsc::Sender<DeadlineCommand>,
    next_id: Arc<AtomicU64>,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

pub(crate) struct UtpDeadlineClient {
    id: u64,
    generation: u64,
    tx: mpsc::Sender<DeadlineCommand>,
    ready_tx: watch::Sender<Option<u64>>,
    ready_rx: watch::Receiver<Option<u64>>,
}

#[derive(Default)]
struct DeadlineQueue {
    ordered: BTreeSet<DeadlineKey>,
    active: HashMap<u64, (Instant, u64)>,
}

impl DeadlineQueue {
    fn schedule(&mut self, id: u64, generation: u64, deadline: Instant) {
        self.cancel(id, None);
        self.active.insert(id, (deadline, generation));
        self.ordered.insert((deadline, id, generation));
    }

    fn cancel(&mut self, id: u64, generation: Option<u64>) {
        let Some((deadline, active_generation)) = self.active.get(&id).copied() else {
            return;
        };
        if generation.is_some_and(|generation| generation != active_generation) {
            return;
        }
        self.active.remove(&id);
        self.ordered.remove(&(deadline, id, active_generation));
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.ordered.first().map(|(deadline, _, _)| *deadline)
    }

    fn pop_due(&mut self, now: Instant) -> Vec<(u64, u64)> {
        let mut due = Vec::new();
        while let Some((deadline, id, generation)) = self.ordered.first().copied() {
            if deadline > now {
                break;
            }
            self.ordered.pop_first();
            if self.active.get(&id) == Some(&(deadline, generation)) {
                self.active.remove(&id);
                due.push((id, generation));
            }
        }
        due
    }
}

impl UtpDeadlineScheduler {
    pub(crate) fn new(stop: watch::Receiver<bool>) -> Self {
        let (tx, rx) = mpsc::channel(DEADLINE_COMMAND_CAPACITY);
        let task = tokio::spawn(run_deadline_scheduler(rx, stop));
        Self {
            tx,
            next_id: Arc::new(AtomicU64::new(1)),
            task: Arc::new(Mutex::new(Some(task))),
        }
    }

    pub(crate) fn client(&self) -> UtpDeadlineClient {
        let (ready_tx, ready_rx) = watch::channel(None);
        UtpDeadlineClient {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            generation: 0,
            tx: self.tx.clone(),
            ready_tx,
            ready_rx,
        }
    }

    pub(crate) async fn wait_stopped(&self) -> Result<(), UtpError> {
        let task = match self.task.lock() {
            Ok(mut task) => task.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(task) = task {
            task.await
                .map_err(|error| UtpError::Io(format!("uTP deadline scheduler failed: {error}")))?;
        }
        Ok(())
    }
}

impl UtpDeadlineClient {
    pub(crate) async fn recv_packet(
        &mut self,
        rx: &mut mpsc::Receiver<UtpPacket>,
        wait: Duration,
    ) -> Result<UtpPacket, UtpError> {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let deadline = Instant::now().checked_add(wait);
        self.tx
            .send(DeadlineCommand::Schedule {
                id: self.id,
                generation,
                deadline,
                ready: self.ready_tx.clone(),
            })
            .await
            .map_err(|_| UtpError::Closed)?;

        loop {
            tokio::select! {
                biased;
                packet = rx.recv() => {
                    return packet.ok_or(UtpError::Closed);
                }
                changed = self.ready_rx.changed() => {
                    changed.map_err(|_| UtpError::Closed)?;
                    match *self.ready_rx.borrow_and_update() {
                        Some(ready_generation) if ready_generation == generation => {
                            return Err(UtpError::Timeout);
                        }
                        Some(_) => {}
                        None => return Err(UtpError::Closed),
                    }
                }
            }
        }
    }
}

impl Drop for UtpDeadlineClient {
    fn drop(&mut self) {
        let _ = self.tx.try_send(DeadlineCommand::Cancel {
            id: self.id,
            generation: self.generation,
        });
    }
}

async fn run_deadline_scheduler(
    mut rx: mpsc::Receiver<DeadlineCommand>,
    mut stop: watch::Receiver<bool>,
) {
    let mut deadlines = DeadlineQueue::default();
    let mut ready = HashMap::<u64, watch::Sender<Option<u64>>>::new();

    loop {
        if *stop.borrow() {
            close_waiters(&ready);
            break;
        }
        let closed_clients = ready
            .iter()
            .filter_map(|(id, signal)| (signal.receiver_count() == 0).then_some(*id))
            .collect::<Vec<_>>();
        for id in closed_clients {
            deadlines.cancel(id, None);
            ready.remove(&id);
        }
        let next_deadline = deadlines.next_deadline();
        tokio::select! {
            command = rx.recv() => match command {
                Some(DeadlineCommand::Schedule { id, generation, deadline, ready: signal }) => {
                    ready.remove(&id);
                    if let Some(deadline) = deadline {
                        deadlines.schedule(id, generation, deadline);
                        ready.insert(id, signal);
                    } else {
                        deadlines.cancel(id, None);
                    }
                }
                Some(DeadlineCommand::Cancel { id, generation }) => {
                    deadlines.cancel(id, Some(generation));
                    if !deadlines.active.contains_key(&id) {
                        ready.remove(&id);
                    }
                }
                None => break,
            },
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    close_waiters(&ready);
                    break;
                }
            }
            _ = wait_until(next_deadline) => {
                for (id, generation) in deadlines.pop_due(Instant::now()) {
                    if let Some(signal) = ready.remove(&id) {
                        signal.send_replace(Some(generation));
                    }
                }
            }
        }
    }
}

fn close_waiters(ready: &HashMap<u64, watch::Sender<Option<u64>>>) {
    for signal in ready.values() {
        signal.send_replace(None);
    }
}

async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(tokio::time::Instant::from_std(deadline)).await,
        None => pending::<()>().await,
    }
}

#[cfg(test)]
mod tests {
    use super::DeadlineQueue;
    use std::time::{Duration, Instant};

    #[test]
    fn replacement_and_cancellation_keep_only_current_deadlines() {
        let now = Instant::now();
        let mut deadlines = DeadlineQueue::default();
        deadlines.schedule(1, 1, now + Duration::from_secs(2));
        deadlines.schedule(2, 1, now + Duration::from_secs(1));
        deadlines.schedule(1, 2, now + Duration::from_secs(3));

        assert_eq!(
            deadlines.next_deadline(),
            Some(now + Duration::from_secs(1))
        );
        assert_eq!(
            deadlines.pop_due(now + Duration::from_secs(2)),
            vec![(2, 1)]
        );
        deadlines.cancel(1, Some(1));
        assert_eq!(
            deadlines.next_deadline(),
            Some(now + Duration::from_secs(3))
        );
        assert_eq!(
            deadlines.pop_due(now + Duration::from_secs(3)),
            vec![(1, 2)]
        );
        assert_eq!(deadlines.next_deadline(), None);
    }
}
