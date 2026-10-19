use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
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

use crate::{error::UtpError, memory::UtpBufferedPacket};

const DEADLINE_COMMAND_CAPACITY: usize = 1024;
const READY_QUEUE_QUANTUM: usize = 32;

type DeadlineKey = (Instant, u64, u64);

pub(crate) fn metadata_budget_bytes(max_clients: usize) -> usize {
    DEADLINE_COMMAND_CAPACITY
        .saturating_add(max_clients)
        .saturating_mul(std::mem::size_of::<DeadlineCommand>())
        .saturating_add(max_clients.saturating_mul(256))
        .saturating_add(4 * 1024)
}

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
    max_clients: usize,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

pub(crate) struct UtpDeadlineClient {
    id: u64,
    generation: u64,
    tx: mpsc::Sender<DeadlineCommand>,
    cancel_permit: Option<mpsc::OwnedPermit<DeadlineCommand>>,
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

    fn pop_due(&mut self, now: Instant) -> Option<(u64, u64)> {
        let (deadline, id, generation) = self.ordered.first().copied()?;
        if deadline > now {
            return None;
        }
        self.ordered.pop_first();
        if self.active.get(&id) != Some(&(deadline, generation)) {
            // Exact cancellation/rescheduling removes the matching ordered
            // entry. Treat an invariant violation as one bounded stale pop;
            // the scheduler will revisit the next due item on its next turn.
            return None;
        }
        self.active.remove(&id);
        Some((id, generation))
    }
}

#[derive(Default)]
struct ReadyQueue {
    queue: VecDeque<u64>,
    enqueued: HashSet<u64>,
    generations: HashMap<u64, u64>,
}

impl ReadyQueue {
    fn push(&mut self, id: u64, generation: u64) {
        if self.enqueued.insert(id) {
            self.queue.push_back(id);
        }
        self.generations.insert(id, generation);
    }

    fn cancel(&mut self, id: u64) {
        self.generations.remove(&id);
    }

    fn pop(&mut self) -> Option<(u64, Option<u64>)> {
        let id = self.queue.pop_front()?;
        self.enqueued.remove(&id);
        Some((id, self.generations.remove(&id)))
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl UtpDeadlineScheduler {
    pub(crate) fn new(stop: watch::Receiver<bool>, max_clients: usize) -> Self {
        // Each admitted client holds one owned permit for its eventual Cancel.
        // Schedule commands can use the remaining 1024 slots, so cancellation
        // remains guaranteed even when the schedule queue is saturated.
        let command_capacity = DEADLINE_COMMAND_CAPACITY.saturating_add(max_clients);
        let (tx, rx) = mpsc::channel(command_capacity.max(1));
        let task = tokio::spawn(run_deadline_scheduler(rx, stop));
        Self {
            tx,
            next_id: Arc::new(AtomicU64::new(1)),
            max_clients,
            task: Arc::new(Mutex::new(Some(task))),
        }
    }

    pub(crate) fn client(&self) -> Result<UtpDeadlineClient, UtpError> {
        let cancel_permit = self
            .tx
            .clone()
            .try_reserve_owned()
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => UtpError::ConnectionLimit {
                    max: self.max_clients,
                },
                mpsc::error::TrySendError::Closed(_) => UtpError::Closed,
            })?;
        let (ready_tx, ready_rx) = watch::channel(None);
        Ok(UtpDeadlineClient {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            generation: 0,
            tx: self.tx.clone(),
            cancel_permit: Some(cancel_permit),
            ready_tx,
            ready_rx,
        })
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
        rx: &mut mpsc::Receiver<UtpBufferedPacket>,
        wait: Duration,
    ) -> Result<UtpBufferedPacket, UtpError> {
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
        if let Some(permit) = self.cancel_permit.take() {
            permit.send(DeadlineCommand::Cancel {
                id: self.id,
                generation: self.generation,
            });
        }
    }
}

async fn run_deadline_scheduler(
    mut rx: mpsc::Receiver<DeadlineCommand>,
    mut stop: watch::Receiver<bool>,
) {
    let mut deadlines = DeadlineQueue::default();
    let mut registered = HashMap::<u64, (u64, watch::Sender<Option<u64>>)>::new();
    let mut ready = ReadyQueue::default();

    loop {
        if *stop.borrow() {
            close_waiters(&registered);
            break;
        }
        let now = Instant::now();
        for _ in 0..READY_QUEUE_QUANTUM {
            let Some((id, generation)) = deadlines.pop_due(now) else {
                break;
            };
            if registered
                .get(&id)
                .is_some_and(|(active_generation, _)| *active_generation == generation)
            {
                ready.push(id, generation);
            }
        }
        if !ready.is_empty() {
            for _ in 0..READY_QUEUE_QUANTUM {
                let Some((id, generation)) = ready.pop() else {
                    break;
                };
                let Some(generation) = generation else {
                    continue;
                };
                let Some((registered_generation, signal)) = registered.get(&id) else {
                    continue;
                };
                if *registered_generation == generation {
                    signal.send_replace(Some(generation));
                }
            }
            // Keep cancellation and reschedule latency bounded while ready
            // timers are being drained. Both command types share FIFO order.
            for _ in 0..READY_QUEUE_QUANTUM {
                let Ok(command) = rx.try_recv() else {
                    break;
                };
                apply_command(command, &mut deadlines, &mut registered, &mut ready);
            }
            // Let stream tasks consume one ready turn before the next batch.
            tokio::task::yield_now().await;
            continue;
        }

        let next_deadline = deadlines.next_deadline();
        tokio::select! {
            command = rx.recv() => match command {
                Some(command) => apply_command(command, &mut deadlines, &mut registered, &mut ready),
                None => break,
            },
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    close_waiters(&registered);
                    break;
                }
            }
            _ = wait_until(next_deadline) => {
                // The bounded FIFO is populated at the top of the next turn.
            }
        }
    }
}

fn close_waiters(ready: &HashMap<u64, (u64, watch::Sender<Option<u64>>)>) {
    for (_, signal) in ready.values() {
        signal.send_replace(None);
    }
}

fn apply_command(
    command: DeadlineCommand,
    deadlines: &mut DeadlineQueue,
    registered: &mut HashMap<u64, (u64, watch::Sender<Option<u64>>)>,
    ready: &mut ReadyQueue,
) {
    match command {
        DeadlineCommand::Schedule {
            id,
            generation,
            deadline,
            ready: signal,
        } => {
            ready.cancel(id);
            if let Some(deadline) = deadline {
                deadlines.schedule(id, generation, deadline);
                registered.insert(id, (generation, signal));
            } else {
                deadlines.cancel(id, None);
                registered.remove(&id);
            }
        }
        DeadlineCommand::Cancel { id, generation } => {
            deadlines.cancel(id, Some(generation));
            if registered
                .get(&id)
                .is_some_and(|(active_generation, _)| *active_generation == generation)
            {
                registered.remove(&id);
                ready.cancel(id);
            }
        }
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
    use super::{
        apply_command, DeadlineCommand, DeadlineQueue, ReadyQueue, DEADLINE_COMMAND_CAPACITY,
    };
    use std::collections::HashMap;
    use std::time::{Duration, Instant};
    use tokio::sync::{mpsc, watch};

    #[test]
    fn reserved_cancel_slot_survives_a_full_schedule_queue_in_fifo_order() {
        let (tx, mut rx) = mpsc::channel(DEADLINE_COMMAND_CAPACITY + 1);
        let cancel_permit = tx
            .clone()
            .try_reserve_owned()
            .expect("one slot is reserved for cancellation");
        let (ready, _receiver) = watch::channel(None);

        for generation in 0..DEADLINE_COMMAND_CAPACITY as u64 {
            assert!(tx
                .try_send(DeadlineCommand::Schedule {
                    id: 7,
                    generation,
                    deadline: None,
                    ready: ready.clone(),
                })
                .is_ok());
        }
        cancel_permit.send(DeadlineCommand::Cancel {
            id: 7,
            generation: DEADLINE_COMMAND_CAPACITY as u64,
        });
        assert!(matches!(
            tx.try_send(DeadlineCommand::Cancel {
                id: 8,
                generation: 1,
            }),
            Err(mpsc::error::TrySendError::Full(_))
        ));

        for generation in 0..DEADLINE_COMMAND_CAPACITY as u64 {
            match rx.try_recv().expect("scheduled command remains queued") {
                DeadlineCommand::Schedule {
                    id: 7,
                    generation: actual,
                    ..
                } => assert_eq!(actual, generation),
                DeadlineCommand::Schedule { .. } | DeadlineCommand::Cancel { .. } => {
                    panic!("unexpected command order")
                }
            }
        }
        match rx
            .try_recv()
            .expect("reserved cancel follows earlier schedules")
        {
            DeadlineCommand::Cancel { id, generation } => {
                assert_eq!(id, 7);
                assert_eq!(generation, DEADLINE_COMMAND_CAPACITY as u64);
            }
            DeadlineCommand::Schedule { .. } => panic!("cancel must follow schedules"),
        }
    }

    #[test]
    fn cancel_removes_registered_deadline_and_invalidates_pending_ready_work() {
        let now = Instant::now();
        let mut deadlines = DeadlineQueue::default();
        deadlines.schedule(7, 3, now + Duration::from_secs(1));
        let mut registered = HashMap::new();
        let (signal, _receiver) = watch::channel(None);
        registered.insert(7, (3, signal));
        let mut ready = ReadyQueue::default();
        ready.push(7, 3);

        apply_command(
            DeadlineCommand::Cancel {
                id: 7,
                generation: 3,
            },
            &mut deadlines,
            &mut registered,
            &mut ready,
        );

        assert!(registered.is_empty());
        assert!(deadlines.next_deadline().is_none());
        assert_eq!(ready.pop(), Some((7, None)));
    }

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
            Some((2, 1))
        );
        assert_eq!(deadlines.pop_due(now + Duration::from_secs(2)), None);
        deadlines.cancel(1, Some(1));
        assert_eq!(
            deadlines.next_deadline(),
            Some(now + Duration::from_secs(3))
        );
        assert_eq!(
            deadlines.pop_due(now + Duration::from_secs(3)),
            Some((1, 2))
        );
        assert_eq!(deadlines.pop_due(now + Duration::from_secs(3)), None);
        assert_eq!(deadlines.next_deadline(), None);
    }

    #[test]
    fn ready_queue_is_fifo_deduplicated_and_dispatches_in_bounded_turns() {
        let mut ready = ReadyQueue::default();
        for id in 0..5 {
            ready.push(id, 1);
        }
        ready.push(0, 2);

        let first_turn = (0..2)
            .map(|_| ready.pop().expect("first ready turn"))
            .collect::<Vec<_>>();
        assert_eq!(first_turn, vec![(0, Some(2)), (1, Some(1))]);
        assert_eq!(ready.pop(), Some((2, Some(1))));
        assert_eq!(ready.pop(), Some((3, Some(1))));
        assert_eq!(ready.pop(), Some((4, Some(1))));
        assert!(ready.is_empty());
    }

    #[test]
    fn canceled_ready_work_is_skipped_without_blocking_later_clients() {
        let mut ready = ReadyQueue::default();
        ready.push(1, 1);
        ready.push(2, 1);
        ready.cancel(1);

        assert_eq!(ready.pop(), Some((1, None)));
        assert_eq!(ready.pop(), Some((2, Some(1))));
    }
}
