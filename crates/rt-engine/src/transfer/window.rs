//! Adaptive request windows and refill deadlines (NET-108).
//!
//! A fixed window caps a peer at `window / response time` however fast it
//! can deliver. This controller sizes each peer's window from what the peer
//! has *demonstrated*: sustainable delivery rate times the response delay
//! measured from the moment a request was actually put on the wire, plus a
//! margin, and then clamps it by every constraint that bounds useful work:
//! the remote's request-queue limit, blocks still worth requesting, rate
//! credit at each level, and buffer commitments.
//!
//! Disk backlog is deliberately **not** part of the delay estimate. A queue
//! that grows because storage is slow must shrink admission; if it instead
//! inflated the latency estimate, the bandwidth-delay product would grow and
//! the client would request even more data into the backlog. Backlog is a
//! separate input that only ever reduces the window.

use super::Nanos;

#[derive(Debug, Clone, Copy)]
pub struct WindowConfig {
    pub block_bytes: u32,
    /// Window before any rate has been observed.
    pub initial: usize,
    pub min: usize,
    pub max: usize,
    /// Extra response time budgeted on top of the measured delay.
    pub margin_nanos: Nanos,
    /// Shortest interval over which a delivery rate is sampled.
    pub sample_nanos: Nanos,
    /// Backlog ratio (device queue / its budget) where shrinking starts.
    pub backlog_high: f64,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            block_bytes: 16 * 1024,
            initial: 8,
            min: 4,
            max: 256,
            margin_nanos: 20_000_000,
            sample_nanos: 100_000_000,
            backlog_high: 0.5,
        }
    }
}

impl WindowConfig {
    fn normalized(self) -> Self {
        let max = self.max.max(1);
        let min = self.min.clamp(1, max);
        let backlog_high = if self.backlog_high.is_finite() {
            self.backlog_high.clamp(0.0, 1.0 - f64::EPSILON)
        } else {
            0.5
        };
        Self {
            block_bytes: self.block_bytes.max(1),
            initial: self.initial.clamp(min, max),
            min,
            max,
            sample_nanos: self.sample_nanos.max(1),
            backlog_high,
            ..self
        }
    }
}

/// Everything besides the peer's own behaviour that bounds the window, in
/// blocks (or a ratio for backlog).
#[derive(Debug, Clone, Copy)]
pub struct WindowInputs {
    /// Blocks this peer could usefully be asked for.
    pub useful_missing: usize,
    /// Requests already outstanding to this peer.
    pub outstanding: usize,
    /// Blocks the rate scheduler would currently grant (min over levels).
    pub credit_blocks: usize,
    /// Blocks of buffer capacity that can still be committed.
    pub buffer_blocks: usize,
    /// Device service backlog as a fraction of its budget (0 empty, 1 full).
    pub device_backlog: f64,
}

impl WindowInputs {
    pub fn unconstrained(useful_missing: usize) -> Self {
        Self {
            useful_missing,
            outstanding: 0,
            credit_blocks: usize::MAX,
            buffer_blocks: usize::MAX,
            device_backlog: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WindowController {
    cfg: WindowConfig,
    rate_bytes_per_sec: Option<f64>,
    delay_nanos: Option<f64>,
    sample_start: Option<Nanos>,
    sample_bytes: u64,
    remote_reqq: Option<usize>,
}

impl WindowController {
    pub fn new(cfg: WindowConfig) -> Self {
        Self {
            cfg: cfg.normalized(),
            rate_bytes_per_sec: None,
            delay_nanos: None,
            sample_start: None,
            sample_bytes: 0,
            remote_reqq: None,
        }
    }

    pub fn set_remote_reqq(&mut self, reqq: Option<usize>) {
        self.remote_reqq = reqq.map(|limit| limit.max(1));
    }

    /// Useful payload delivered (verified-valid block bytes, not duplicates).
    pub fn on_delivery(&mut self, bytes: u64, now: Nanos) {
        let start = *self.sample_start.get_or_insert(now);
        self.sample_bytes = self.sample_bytes.saturating_add(bytes);
        let elapsed = now.saturating_sub(start);
        if elapsed >= self.cfg.sample_nanos {
            let sample = self.sample_bytes as f64 * 1e9 / elapsed as f64;
            self.rate_bytes_per_sec = Some(match self.rate_bytes_per_sec {
                Some(rate) => rate * 0.7 + sample * 0.3,
                None => sample,
            });
            self.sample_start = Some(now);
            self.sample_bytes = 0;
        }
    }

    /// Delay from the request being written to the response arriving. The
    /// caller measures this to *receipt*, never including disk queueing.
    pub fn on_response_delay(&mut self, delay: Nanos) {
        let delay = delay as f64;
        self.delay_nanos = Some(match self.delay_nanos {
            Some(current) => current * 0.8 + delay * 0.2,
            None => delay,
        });
    }

    /// Bandwidth-delay product in blocks, if both quantities are known.
    pub fn bdp_blocks(&self) -> Option<usize> {
        let rate = self.rate_bytes_per_sec?;
        let delay = self.delay_nanos? + self.cfg.margin_nanos as f64;
        Some((rate * delay / 1e9 / f64::from(self.cfg.block_bytes)).ceil() as usize)
    }

    pub fn rate_bytes_per_sec(&self) -> Option<f64> {
        self.rate_bytes_per_sec
    }

    /// The window to maintain now: how many requests may be outstanding.
    pub fn window(&self, inputs: &WindowInputs) -> usize {
        let base = match self.bdp_blocks() {
            Some(bdp) => bdp.clamp(self.cfg.min, self.cfg.max),
            None => self.cfg.initial,
        };
        // A growing device backlog only ever shrinks admission.
        let backlog = if inputs.device_backlog.is_finite() {
            inputs.device_backlog.clamp(0.0, 1.0)
        } else {
            // An unmeasurable backlog must never increase admission.
            1.0
        };
        let shaped = if backlog > self.cfg.backlog_high {
            let over =
                (backlog - self.cfg.backlog_high) / (1.0 - self.cfg.backlog_high).max(f64::EPSILON);
            let scale = (1.0 - over).max(0.0);
            ((base as f64 * scale).floor() as usize).max(1)
        } else {
            base
        };
        let mut window = shaped;
        if let Some(reqq) = self.remote_reqq {
            window = window.min(reqq);
        }
        // Never ask for more than there is left worth asking, or can be paid
        // for by rate credit and held in buffers.
        window
            .min(inputs.outstanding.saturating_add(inputs.useful_missing))
            .min(inputs.outstanding.saturating_add(inputs.credit_blocks))
            .min(inputs.outstanding.saturating_add(inputs.buffer_blocks))
    }

    /// How many new requests to issue now.
    pub fn to_request(&self, inputs: &WindowInputs) -> usize {
        self.window(inputs).saturating_sub(inputs.outstanding)
    }
}

/// The single timer deadline for refill: the earliest of the wakeups the
/// rate schedulers reported, so nothing polls on a fixed tick.
pub fn next_refill_deadline(wakes: impl IntoIterator<Item = Option<Nanos>>) -> Option<Nanos> {
    wakes.into_iter().flatten().min()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Nanos = 1_000_000;

    fn warmed(rate_bytes_per_sec: u64, delay_ms: u64) -> WindowController {
        let mut controller = WindowController::new(WindowConfig::default());
        let mut now = 0;
        let step = 100 * MS;
        for _ in 0..30 {
            now += step;
            controller.on_delivery(rate_bytes_per_sec / 10, now);
            controller.on_response_delay(delay_ms * MS);
        }
        controller
    }

    #[test]
    fn window_follows_the_bandwidth_delay_product() {
        // 50 MiB/s at 50 ms (+20 ms margin) is about 224 blocks of 16 KiB.
        let controller = warmed(50 * 1024 * 1024, 50);
        let bdp = controller.bdp_blocks().unwrap();
        assert!((200..=240).contains(&bdp), "bdp {bdp}");
        assert_eq!(
            controller.window(&WindowInputs::unconstrained(10_000)),
            bdp.min(256)
        );
    }

    #[test]
    fn a_slow_peer_keeps_a_small_window_and_an_unknown_peer_the_initial_one() {
        let fresh = WindowController::new(WindowConfig::default());
        assert_eq!(fresh.window(&WindowInputs::unconstrained(100)), 8);
        let slow = warmed(64 * 1024, 40);
        assert_eq!(
            slow.window(&WindowInputs::unconstrained(100)),
            4,
            "clamped to the floor"
        );
    }

    #[test]
    fn disk_backlog_shrinks_the_window_but_not_the_delay_estimate() {
        let controller = warmed(50 * 1024 * 1024, 50);
        let free = controller.window(&WindowInputs::unconstrained(10_000));
        let delay_before = controller.delay_nanos;
        let half = controller.window(&WindowInputs {
            device_backlog: 0.75,
            ..WindowInputs::unconstrained(10_000)
        });
        let full = controller.window(&WindowInputs {
            device_backlog: 1.0,
            ..WindowInputs::unconstrained(10_000)
        });
        assert!(half < free && full < half, "{free} > {half} > {full}");
        assert_eq!(full, 1, "a full backlog admits one request at a time");
        assert_eq!(
            controller.delay_nanos, delay_before,
            "backlog never feeds the delay estimate"
        );
    }

    #[test]
    fn every_constraint_bounds_the_window() {
        let mut controller = warmed(50 * 1024 * 1024, 50);
        let open = WindowInputs::unconstrained(10_000);
        assert!(controller.window(&open) > 100);
        controller.set_remote_reqq(Some(30));
        assert_eq!(controller.window(&open), 30, "remote request-queue limit");
        controller.set_remote_reqq(None);
        assert_eq!(
            controller.window(&WindowInputs::unconstrained(5)),
            5,
            "only 5 useful blocks remain"
        );
        assert_eq!(
            controller.window(&WindowInputs {
                credit_blocks: 3,
                ..WindowInputs::unconstrained(10_000)
            }),
            3,
            "rate credit pays for 3 blocks"
        );
        assert_eq!(
            controller.window(&WindowInputs {
                buffer_blocks: 2,
                ..WindowInputs::unconstrained(10_000)
            }),
            2,
            "only 2 blocks of buffer can be committed"
        );
    }

    #[test]
    fn outstanding_requests_count_against_the_window() {
        let controller = warmed(50 * 1024 * 1024, 50);
        let window = controller.window(&WindowInputs::unconstrained(10_000));
        let inputs = WindowInputs {
            outstanding: window - 5,
            ..WindowInputs::unconstrained(10_000)
        };
        assert_eq!(controller.to_request(&inputs), 5);
        let full = WindowInputs {
            outstanding: window,
            ..WindowInputs::unconstrained(10_000)
        };
        assert_eq!(controller.to_request(&full), 0);
    }

    #[test]
    fn refill_uses_the_earliest_scheduler_wakeup() {
        assert_eq!(
            next_refill_deadline([None, Some(900), Some(400), None]),
            Some(400)
        );
        assert_eq!(next_refill_deadline([None, None]), None);
    }

    #[test]
    fn invalid_window_configuration_is_normalized_to_safe_bounds() {
        let controller = WindowController::new(WindowConfig {
            block_bytes: 0,
            initial: 0,
            min: 8,
            max: 2,
            margin_nanos: 0,
            sample_nanos: 0,
            backlog_high: f64::NAN,
        });

        assert_eq!(controller.cfg.block_bytes, 1);
        assert_eq!(controller.cfg.min, 2);
        assert_eq!(controller.cfg.max, 2);
        assert_eq!(controller.cfg.initial, 2);
        assert_eq!(controller.cfg.sample_nanos, 1);
        assert_eq!(controller.cfg.backlog_high, 0.5);
        assert_eq!(
            controller.window(&WindowInputs::unconstrained(100)),
            2,
            "the initial window respects the normalized maximum"
        );
        assert_eq!(
            controller.window(&WindowInputs {
                device_backlog: f64::NAN,
                ..WindowInputs::unconstrained(100)
            }),
            1,
            "unknown backlog shrinks admission conservatively"
        );
    }
}
