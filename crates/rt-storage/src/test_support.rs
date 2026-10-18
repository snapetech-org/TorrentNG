//! Deterministic backend fault injection for storage contract tests.
//!
//! This module is only compiled for tests. It delegates real operations to
//! the portable backend, then can fail them, fail after a partial write, or
//! hold completion notification until the test explicitly releases it.

use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use bytes::Bytes;
use tokio::sync::{oneshot, Notify};

use crate::backend::{DiskBackend, PreadBackend};
use crate::frame::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Read,
    Write,
    Sync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Effect {
    Fail(io::ErrorKind),
    HoldCompletion,
    WritePrefixThenFail {
        prefix_bytes: usize,
        kind: io::ErrorKind,
    },
}

#[derive(Debug, Clone, Copy)]
struct FaultRule {
    operation: Operation,
    effect: Effect,
}

struct Control {
    rules: Mutex<VecDeque<FaultRule>>,
    releases: Mutex<VecDeque<mpsc::Sender<()>>>,
    ready_count: AtomicUsize,
    ready: Notify,
}

/// Portable worker pool with a FIFO script of deterministic faults.
pub(crate) struct ScriptedDiskBackend {
    delegate: PreadBackend,
    control: Arc<Control>,
}

impl ScriptedDiskBackend {
    pub(crate) fn new(worker_threads: usize, queue_depth: usize) -> Self {
        Self {
            delegate: PreadBackend::new_with_queue_depth(worker_threads, queue_depth),
            control: Arc::new(Control {
                rules: Mutex::new(VecDeque::new()),
                releases: Mutex::new(VecDeque::new()),
                ready_count: AtomicUsize::new(0),
                ready: Notify::new(),
            }),
        }
    }

    pub(crate) fn inject(&self, operation: Operation, effect: Effect) {
        self.control
            .rules
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(FaultRule { operation, effect });
    }

    fn take_effect(&self, operation: Operation) -> Option<Effect> {
        let mut rules = self
            .control
            .rules
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if rules
            .front()
            .is_some_and(|rule| rule.operation == operation)
        {
            rules.pop_front().map(|rule| rule.effect)
        } else {
            None
        }
    }

    fn fail<T>(&self, kind: io::ErrorKind) -> oneshot::Receiver<io::Result<T>> {
        let (reply, receiver) = oneshot::channel();
        let _ = reply.send(Err(io::Error::new(kind, "injected storage fault")));
        receiver
    }

    fn hold<T: Send + 'static>(
        &self,
        inner: oneshot::Receiver<io::Result<T>>,
        reply: oneshot::Sender<io::Result<T>>,
        forced_error: Option<io::ErrorKind>,
    ) {
        let (release, wait) = mpsc::channel();
        self.control
            .releases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(release);
        let control = self.control.clone();
        thread::Builder::new()
            .name("tng-storage-test-completion".to_string())
            .spawn(move || {
                let result = inner.blocking_recv().unwrap_or_else(|_| {
                    Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "portable backend dropped its test reply",
                    ))
                });
                control.ready_count.fetch_add(1, Ordering::Release);
                control.ready.notify_one();
                let _ = wait.recv();
                let result = forced_error.map_or(result, |kind| {
                    Err(io::Error::new(
                        kind,
                        "injected storage fault after partial write",
                    ))
                });
                let _ = reply.send(result);
            })
            .expect("test completion worker should start");
    }

    pub(crate) async fn wait_until_completion_is_held(&self) {
        loop {
            let notified = self.control.ready.notified();
            let count = self.control.ready_count.load(Ordering::Acquire);
            if count > 0
                && self
                    .control
                    .ready_count
                    .compare_exchange(count, count - 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn pending_releases(&self) -> usize {
        self.control
            .releases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub(crate) fn release_next(&self) -> bool {
        self.control
            .releases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .is_some_and(|release| release.send(()).is_ok())
    }
}

impl Drop for ScriptedDiskBackend {
    fn drop(&mut self) {
        let mut releases = self
            .control
            .releases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for release in releases.drain(..) {
            let _ = release.send(());
        }
    }
}

impl DiskBackend for ScriptedDiskBackend {
    fn pread(
        &self,
        file: Arc<File>,
        frame: Frame,
        offset: u64,
    ) -> oneshot::Receiver<io::Result<Frame>> {
        match self.take_effect(Operation::Read) {
            Some(Effect::Fail(kind)) => self.fail(kind),
            Some(Effect::HoldCompletion) => {
                let inner = self.delegate.pread(file, frame, offset);
                let (reply, receiver) = oneshot::channel();
                self.hold(inner, reply, None);
                receiver
            }
            Some(Effect::WritePrefixThenFail { .. }) => self.fail(io::ErrorKind::InvalidInput),
            None => self.delegate.pread(file, frame, offset),
        }
    }

    fn pwrite(
        &self,
        file: Arc<File>,
        data: Bytes,
        offset: u64,
    ) -> oneshot::Receiver<io::Result<()>> {
        match self.take_effect(Operation::Write) {
            Some(Effect::Fail(kind)) => self.fail(kind),
            Some(Effect::HoldCompletion) => {
                let inner = self.delegate.pwrite(file, data, offset);
                let (reply, receiver) = oneshot::channel();
                self.hold(inner, reply, None);
                receiver
            }
            Some(Effect::WritePrefixThenFail { prefix_bytes, kind }) => {
                let prefix_len = prefix_bytes.min(data.len());
                let inner = self.delegate.pwrite(file, data.slice(..prefix_len), offset);
                let (reply, receiver) = oneshot::channel();
                self.hold(inner, reply, Some(kind));
                receiver
            }
            None => self.delegate.pwrite(file, data, offset),
        }
    }

    fn fdatasync(&self, file: Arc<File>) -> oneshot::Receiver<io::Result<()>> {
        match self.take_effect(Operation::Sync) {
            Some(Effect::Fail(kind)) => self.fail(kind),
            Some(Effect::HoldCompletion) => {
                let inner = self.delegate.fdatasync(file);
                let (reply, receiver) = oneshot::channel();
                self.hold(inner, reply, None);
                receiver
            }
            Some(Effect::WritePrefixThenFail { .. }) => self.fail(io::ErrorKind::InvalidInput),
            None => self.delegate.fdatasync(file),
        }
    }
}
