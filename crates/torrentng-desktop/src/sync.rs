//! Background controller: one task that owns the network conversation.
//!
//! The UI thread never blocks on HTTP. It sends [`Command`]s in and drains
//! [`Event`]s out; every outgoing event wakes the UI with a repaint request,
//! and nothing runs while the window is minimized.
//!
//! Update strategy (no full-refresh polling loops):
//! * page fetch -> its `snapshot` is the registry revision;
//! * SSE `/events?last_known_revision=<that revision>` delivers only deltas;
//! * in-window deltas patch rows in place; membership changes trigger one
//!   debounced page refetch;
//! * `/torrents/live` supplies rates for the visible rows only;
//! * if SSE is unavailable the controller falls back to a slow page refetch.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Duration;

use eframe::egui;
use futures_util::StreamExt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep_until, Instant, MissedTickBehavior};

use crate::api::{AddSource, ApiError, Client, SseParser};
use crate::model::{Delta, Facets, LiveRow, TorrentDetail, TorrentSummary, TransferInfo, ViewSpec};

const TICK: Duration = Duration::from_secs(2);
const FACET_EVERY_TICKS: u32 = 5;
/// The daemon may serve a list snapshot up to 750 ms old, so a refetch right
/// after a change can miss it. Follow-up refetches wait past that window.
const SNAPSHOT_STALE_MAX: Duration = Duration::from_millis(900);
/// Debounce for membership-changing deltas (removals).
const REFETCH_FAST: Duration = SNAPSHOT_STALE_MAX;
/// Debounce for "something outside/inside the window changed".
const REFETCH_SLOW: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum Command {
    /// (Re)define the window of rows the table wants.
    Window {
        epoch: u64,
        spec: ViewSpec,
        offset: usize,
        limit: usize,
    },
    /// Hashes currently on screen; their rates are polled.
    Visible(Vec<String>),
    /// `false` while the window is minimized.
    Active(bool),
    Action {
        hashes: Vec<String>,
        action: &'static str,
    },
    Remove {
        hashes: Vec<String>,
        delete_files: bool,
    },
    Add {
        source: AddSource,
        save_path: String,
        start: bool,
    },
    Detail(String),
    Refresh,
    Shutdown,
}

#[derive(Debug)]
pub enum Event {
    Connected,
    /// Server unreachable; the controller keeps retrying.
    Offline(String),
    /// The token was rejected; the controller stops until reconfigured.
    AuthFailed,
    Page {
        epoch: u64,
        offset: usize,
        total: usize,
        revision: u64,
        rows: Vec<TorrentSummary>,
    },
    Patch(Vec<TorrentSummary>),
    Live(Vec<LiveRow>),
    Transfer(TransferInfo),
    Facets(Facets),
    Detail(Box<TorrentDetail>),
    Notice(String),
    // Not produced by the controller: results of UI-spawned async work that
    // share its channel so the UI has a single place to drain.
    EngineReady {
        url: String,
        token: String,
    },
    EngineFailed(String),
    FilePicked(Option<(String, Vec<u8>)>),
}

/// Event sink that wakes the UI.
#[derive(Clone)]
pub struct Out {
    tx: Sender<Event>,
    ctx: egui::Context,
}

impl Out {
    pub fn new(tx: Sender<Event>, ctx: egui::Context) -> Self {
        Self { tx, ctx }
    }

    pub fn send(&self, ev: Event) {
        if self.tx.send(ev).is_ok() {
            self.ctx.request_repaint();
        }
    }
}

enum SseMsg {
    Up,
    Down,
    Delta(Delta),
}

fn spawn_sse(client: Client, revision: u64, tx: UnboundedSender<SseMsg>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut rev = revision;
        let mut backoff = Duration::from_secs(1);
        loop {
            match client.events(rev).await {
                Ok(resp) => {
                    let _ = tx.send(SseMsg::Up);
                    let mut parser = SseParser::default();
                    let mut stream = resp.bytes_stream();
                    while let Some(chunk) = stream.next().await {
                        let Ok(bytes) = chunk else { break };
                        for ev in parser.feed(&bytes) {
                            if ev.event != "torrent_delta" {
                                continue;
                            }
                            if let Ok(d) = serde_json::from_str::<Delta>(&ev.data) {
                                rev = rev.max(d.cursor);
                                backoff = Duration::from_secs(1);
                                if tx.send(SseMsg::Delta(d)).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
                Err(ApiError::Unauthorized) => {
                    let _ = tx.send(SseMsg::Down);
                    return;
                }
                Err(_) => {}
            }
            if tx.send(SseMsg::Down).is_err() {
                return;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    })
}

#[derive(Clone)]
struct Window {
    epoch: u64,
    spec: ViewSpec,
    offset: usize,
    limit: usize,
}

/// Last value pushed to the UI for each stream. Unchanged data is not resent:
/// every event wakes the UI for a repaint, so an idle library must be silent.
#[derive(Default)]
struct Seen {
    transfer: Option<TransferInfo>,
    live: Vec<LiveRow>,
    facets: Option<Facets>,
    page: Option<(u64, usize, usize, Vec<TorrentSummary>)>,
}

struct Controller {
    client: Client,
    out: Out,
    window: Option<Window>,
    /// Last known summary of each row in the window, so deltas that change
    /// nothing visible are dropped instead of waking the UI.
    window_rows: HashMap<String, TorrentSummary>,
    visible: Vec<String>,
    online: bool,
    active: bool,
    dead: bool,
    sse: Option<JoinHandle<()>>,
    sse_rx: Option<UnboundedReceiver<SseMsg>>,
    sse_up: bool,
    refetch_at: Option<Instant>,
    ticks: u32,
    seen: Seen,
}

impl Controller {
    fn request_refetch(&mut self, delay: Duration) {
        let at = Instant::now() + delay;
        self.refetch_at = Some(self.refetch_at.map_or(at, |cur| cur.min(at)));
    }

    fn handle_err(&mut self, e: ApiError) {
        match e {
            ApiError::Unauthorized => {
                self.dead = true;
                self.out.send(Event::AuthFailed);
            }
            ApiError::Network(m) => {
                if self.online {
                    self.online = false;
                    self.out.send(Event::Offline(m));
                }
            }
            ApiError::Http { status: 410, .. } => self.request_refetch(Duration::ZERO),
            other => self.out.send(Event::Notice(other.to_string())),
        }
    }

    async fn go_online(&mut self) {
        self.online = true;
        self.out.send(Event::Connected);
        self.fetch_page().await;
        self.fetch_side().await;
    }

    /// Always deliver a page (the UI is waiting for the answer).
    async fn fetch_page(&mut self) {
        self.fetch_page_inner(true).await;
    }

    /// Periodic/debounced refresh: deliver only if something changed.
    async fn fetch_page_if_changed(&mut self) {
        self.fetch_page_inner(false).await;
    }

    async fn fetch_page_inner(&mut self, force: bool) {
        self.refetch_at = None;
        let Some(w) = self.window.clone() else { return };
        match self.client.list(&w.spec, w.offset, w.limit, None).await {
            Ok(r) => {
                self.window_rows = r
                    .torrents
                    .iter()
                    .map(|t| (t.info_hash.clone(), t.clone()))
                    .collect();
                if self.sse.is_none() {
                    let (tx, rx) = unbounded_channel();
                    self.sse = Some(spawn_sse(self.client.clone(), r.snapshot, tx));
                    self.sse_rx = Some(rx);
                }
                let key = (w.epoch, w.offset, r.total);
                let unchanged = self
                    .seen
                    .page
                    .as_ref()
                    .is_some_and(|(e, o, t, rows)| (*e, *o, *t) == key && *rows == r.torrents);
                if force || !unchanged {
                    self.seen.page = Some((key.0, key.1, key.2, r.torrents.clone()));
                    self.out.send(Event::Page {
                        epoch: w.epoch,
                        offset: w.offset,
                        total: r.total,
                        revision: r.snapshot,
                        rows: r.torrents,
                    });
                }
            }
            Err(e) => self.handle_err(e),
        }
    }

    async fn fetch_side(&mut self) {
        match self.client.transfer_info().await {
            Ok(t) => self.send_transfer(t),
            Err(e) => return self.handle_err(e),
        }
        match self.client.facets().await {
            Ok(f) => self.send_facets(f),
            Err(e) => self.handle_err(e),
        }
    }

    fn send_transfer(&mut self, t: TransferInfo) {
        if self.seen.transfer.as_ref() != Some(&t) {
            self.seen.transfer = Some(t.clone());
            self.out.send(Event::Transfer(t));
        }
    }

    fn send_facets(&mut self, f: Facets) {
        if self.seen.facets.as_ref() != Some(&f) {
            self.seen.facets = Some(f.clone());
            self.out.send(Event::Facets(f));
        }
    }

    async fn fetch_live(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        match self.client.live(&self.visible).await {
            Ok(rows) => {
                if rows != self.seen.live {
                    self.seen.live = rows.clone();
                    self.out.send(Event::Live(rows));
                }
            }
            Err(e) => self.handle_err(e),
        }
    }

    fn on_delta(&mut self, d: Delta) {
        if !self.active {
            return;
        }
        if d.snapshot {
            // Cursor expired and the server is resyncing: do not ingest the
            // chunked snapshot, just reload our window once.
            self.request_refetch(REFETCH_FAST);
            return;
        }
        if !d.removed.is_empty() {
            self.request_refetch(REFETCH_FAST);
        }
        if d.torrents.is_empty() {
            return;
        }
        let spec_sensitive = self.window.as_ref().is_some_and(|w| {
            !w.spec.status.is_empty()
                || matches!(
                    w.spec.sort,
                    crate::model::SortKey::Progress | crate::model::SortKey::Ratio
                )
        });
        let mut changed = Vec::new();
        let mut outside = false;
        for t in d.torrents {
            match self.window_rows.get_mut(&t.info_hash) {
                Some(known) if *known == t => {}
                Some(known) => {
                    *known = t.clone();
                    changed.push(t);
                }
                None => outside = true,
            }
        }
        if outside || (!changed.is_empty() && spec_sensitive) {
            self.request_refetch(REFETCH_SLOW);
        }
        if !changed.is_empty() {
            self.out.send(Event::Patch(changed));
        }
    }

    async fn on_command(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Window {
                epoch,
                spec,
                offset,
                limit,
            } => {
                self.window = Some(Window {
                    epoch,
                    spec,
                    offset,
                    limit,
                });
                if self.online {
                    self.fetch_page().await;
                }
            }
            Command::Visible(h) => {
                self.visible = h;
                if self.online && self.active {
                    self.fetch_live().await;
                }
            }
            Command::Active(a) => {
                self.active = a;
                if a && self.online {
                    self.fetch_page().await;
                    self.fetch_live().await;
                    self.fetch_side().await;
                }
            }
            Command::Action { hashes, action } => {
                let mut failed = 0;
                for h in &hashes {
                    if let Err(e) = self.client.action(h, action).await {
                        if matches!(e, ApiError::Unauthorized) {
                            return self.finish_err(e);
                        }
                        failed += 1;
                    }
                }
                if failed > 0 {
                    self.out.send(Event::Notice(format!(
                        "{action}: {failed} of {} failed",
                        hashes.len()
                    )));
                }
                self.after_mutation().await;
            }
            Command::Remove {
                hashes,
                delete_files,
            } => {
                let mut failed = 0;
                for h in &hashes {
                    if let Err(e) = self.client.remove(h, delete_files).await {
                        if matches!(e, ApiError::Unauthorized) {
                            return self.finish_err(e);
                        }
                        failed += 1;
                    }
                }
                self.out.send(Event::Notice(if failed == 0 {
                    format!("Removed {} torrent(s)", hashes.len())
                } else {
                    format!("Remove: {failed} of {} failed", hashes.len())
                }));
                self.after_mutation().await;
            }
            Command::Add {
                source,
                save_path,
                start,
            } => match self.client.add(source, &save_path, start).await {
                Ok(()) => {
                    self.out.send(Event::Notice("Torrent added".into()));
                    self.fetch_page().await;
                    self.fetch_side().await;
                }
                Err(e) => self.out.send(Event::Notice(format!("Add failed: {e}"))),
            },
            Command::Detail(hash) => match self.client.detail(&hash).await {
                Ok(d) => self.out.send(Event::Detail(Box::new(d))),
                Err(e) => self.handle_err(e),
            },
            Command::Refresh => {
                self.fetch_page().await;
                self.fetch_side().await;
            }
            Command::Shutdown => return false,
        }
        !self.dead
    }

    /// Refresh now, and again once the daemon's snapshot cache has expired.
    async fn after_mutation(&mut self) {
        self.fetch_page().await;
        self.fetch_side().await;
        self.request_refetch(SNAPSHOT_STALE_MAX);
    }

    fn finish_err(&mut self, e: ApiError) -> bool {
        self.handle_err(e);
        !self.dead
    }

    async fn on_tick(&mut self) {
        if !self.online {
            match self.client.verify().await {
                Ok(()) => self.go_online().await,
                Err(e) => self.handle_err(e),
            }
            return;
        }
        if !self.active {
            return;
        }
        self.ticks = self.ticks.wrapping_add(1);
        if !self.sse_up {
            // No delta stream: slow full-page refresh instead.
            self.fetch_page_if_changed().await;
        }
        self.fetch_live().await;
        match self.client.transfer_info().await {
            Ok(t) => self.send_transfer(t),
            Err(e) => return self.handle_err(e),
        }
        if self.ticks.is_multiple_of(FACET_EVERY_TICKS) {
            match self.client.facets().await {
                Ok(f) => self.send_facets(f),
                Err(e) => self.handle_err(e),
            }
        }
    }
}

/// Run the controller until `Shutdown`, a dropped command channel, or an
/// authentication failure.
pub async fn run(client: Client, mut cmds: UnboundedReceiver<Command>, out: Out) {
    let mut c = Controller {
        client,
        out,
        window: None,
        window_rows: HashMap::new(),
        visible: Vec::new(),
        online: false,
        active: true,
        dead: false,
        sse: None,
        sse_up: false,
        refetch_at: None,
        ticks: 0,
        seen: Seen::default(),
        sse_rx: None,
    };
    let mut tick = interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        let refetch = c.refetch_at;
        tokio::select! {
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { break };
                if !c.on_command(cmd).await { break }
            }
            msg = async {
                match c.sse_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => match msg {
                Some(SseMsg::Up) => c.sse_up = true,
                Some(SseMsg::Down) => c.sse_up = false,
                Some(SseMsg::Delta(d)) => c.on_delta(d),
                None => { c.sse_rx = None; c.sse = None; c.sse_up = false; }
            },
            _ = tick.tick() => {
                c.on_tick().await;
                if c.dead { break }
            }
            _ = async {
                match refetch {
                    Some(at) => sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                if c.online && c.active { c.fetch_page_if_changed().await } else { c.refetch_at = None }
                if c.dead { break }
            }
        }
    }
    if let Some(h) = c.sse.take() {
        h.abort();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{channel, Receiver};

    use super::*;
    use crate::model::tests::summary;

    fn controller() -> (Controller, Receiver<Event>) {
        let (tx, rx) = channel();
        let c = Controller {
            client: Client::new("http://127.0.0.1:9", "t").unwrap(),
            out: Out::new(tx, egui::Context::default()),
            window: Some(Window {
                epoch: 1,
                spec: ViewSpec::default(),
                offset: 0,
                limit: 200,
            }),
            window_rows: HashMap::new(),
            visible: Vec::new(),
            online: true,
            active: true,
            dead: false,
            sse: None,
            sse_rx: None,
            sse_up: true,
            refetch_at: None,
            ticks: 0,
            seen: Seen::default(),
        };
        (c, rx)
    }

    fn delta(torrents: Vec<TorrentSummary>, removed: Vec<String>) -> Delta {
        Delta {
            cursor: 2,
            torrents,
            removed,
            ..Delta::default()
        }
    }

    #[test]
    fn unchanged_delta_for_a_visible_row_is_silent() {
        let (mut c, rx) = controller();
        let t = summary("seeding", 10, 0);
        c.window_rows.insert(t.info_hash.clone(), t.clone());
        c.on_delta(delta(vec![t.clone()], vec![]));
        assert!(rx.try_recv().is_err(), "no event, so no repaint");
        assert!(c.refetch_at.is_none());

        let mut changed = t;
        changed.state = "stopped".into();
        c.on_delta(delta(vec![changed.clone()], vec![]));
        match rx.try_recv() {
            Ok(Event::Patch(p)) => assert_eq!(p, vec![changed]),
            other => panic!("expected a patch, got {other:?}"),
        }
        // Re-sending the same change is now a no-op too.
        c.on_delta(delta(
            vec![c.window_rows.values().next().unwrap().clone()],
            vec![],
        ));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn rows_outside_the_window_and_removals_schedule_one_refetch() {
        let (mut c, rx) = controller();
        c.on_delta(delta(vec![summary("seeding", 10, 0)], vec![]));
        assert!(
            rx.try_recv().is_err(),
            "outside the window: nothing to patch"
        );
        assert!(c.refetch_at.is_some());

        c.refetch_at = None;
        c.on_delta(delta(vec![], vec!["a".repeat(40)]));
        assert!(c.refetch_at.is_some(), "removal reloads the window");
    }

    #[test]
    fn snapshot_resync_is_not_ingested() {
        let (mut c, rx) = controller();
        let mut d = delta(vec![summary("seeding", 10, 0)], vec![]);
        d.snapshot = true;
        c.on_delta(d);
        assert!(rx.try_recv().is_err());
        assert!(c.refetch_at.is_some());
    }

    #[test]
    fn minimized_window_ignores_deltas() {
        let (mut c, rx) = controller();
        c.active = false;
        c.on_delta(delta(vec![summary("seeding", 10, 0)], vec!["x".into()]));
        assert!(rx.try_recv().is_err());
        assert!(c.refetch_at.is_none());
    }

    #[test]
    fn transfer_and_facets_resend_only_on_change() {
        let (mut c, rx) = controller();
        let t = TransferInfo::default();
        c.send_transfer(t.clone());
        c.send_transfer(t.clone());
        assert!(matches!(rx.try_recv(), Ok(Event::Transfer(_))));
        assert!(rx.try_recv().is_err());
        let mut t2 = t;
        t2.dl_info_speed = 5;
        c.send_transfer(t2);
        assert!(matches!(rx.try_recv(), Ok(Event::Transfer(_))));

        c.send_facets(Facets::default());
        c.send_facets(Facets::default());
        assert!(matches!(rx.try_recv(), Ok(Event::Facets(_))));
        assert!(rx.try_recv().is_err());
    }
}
