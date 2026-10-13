//! Application state, connection lifecycle and per-frame orchestration.
//! Panels live in `ui.rs`.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

use crate::api::{AddSource, Client};
use crate::backend::{self, LocalEngine};
use crate::config::{self, BackendMode, Config};
use crate::model::{status, SortKey, TorrentDetail, TransferInfo, ViewSpec};
use crate::sync::{self, Command, Event, Out};
use crate::theme;
use crate::view::{ViewState, PAGE};

#[derive(Debug, Clone, PartialEq)]
pub enum Conn {
    /// First run: no connection chosen yet.
    Setup,
    /// Launching the bundled engine.
    Starting,
    Connecting,
    Online,
    Offline(String),
    AuthFailed,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    General,
    Files,
    Trackers,
}

#[derive(Default)]
pub struct AddDialog {
    pub text: String,
    pub file: Option<(String, Vec<u8>)>,
    pub save_path: String,
    pub start: bool,
}

pub struct SettingsDraft {
    pub cfg: Config,
}

pub struct RemoveDialog {
    pub hashes: Vec<String>,
    pub delete_files: bool,
}

pub struct App {
    pub rt: tokio::runtime::Runtime,
    pub cfg: Config,
    pub cfg_path: PathBuf,
    pub conn: Conn,
    pub cmd: Option<UnboundedSender<Command>>,
    ctrl: Option<JoinHandle<()>>,
    events: Receiver<Event>,
    ev_tx: Sender<Event>,
    engine: Arc<AsyncMutex<Option<LocalEngine>>>,

    pub view: ViewState,
    pub facets: HashMap<String, u64>,
    pub transfer: TransferInfo,
    pub selected: BTreeSet<String>,
    pub anchor: Option<usize>,
    pub detail: Option<TorrentDetail>,
    pub detail_tab: DetailTab,
    pub toasts: Vec<(String, Instant)>,

    pub search: String,
    search_applied: String,
    search_changed: Option<Instant>,
    pub focus_search: bool,

    pub add: Option<AddDialog>,
    pub settings: Option<SettingsDraft>,
    pub remove: Option<RemoveDialog>,

    pending_offset: Option<usize>,
    last_visible: Option<(usize, usize, usize)>,
    visible_dirty: bool,
    sent_active: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg_path = config::config_path();
        let cfg = Config::load(&cfg_path);
        theme::apply(&cc.egui_ctx, cfg.theme);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tng-net")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (ev_tx, events) = channel();
        let mut app = Self {
            rt,
            conn: Conn::Setup,
            cfg_path,
            cmd: None,
            ctrl: None,
            events,
            ev_tx,
            engine: Arc::new(AsyncMutex::new(None)),
            view: ViewState {
                spec: ViewSpec {
                    descending: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            facets: HashMap::new(),
            transfer: TransferInfo::default(),
            selected: BTreeSet::new(),
            anchor: None,
            detail: None,
            detail_tab: DetailTab::General,
            toasts: Vec::new(),
            search: String::new(),
            search_applied: String::new(),
            search_changed: None,
            focus_search: false,
            add: None,
            settings: None,
            remove: None,
            pending_offset: None,
            last_visible: None,
            visible_dirty: true,
            sent_active: true,
            cfg,
        };
        if app.cfg.setup_done && (app.cfg.mode == BackendMode::Remote || app.cfg.local.autostart) {
            app.connect(&cc.egui_ctx);
        } else if app.cfg.setup_done {
            app.conn = Conn::Failed("Engine stopped. Use Start engine to launch it.".into());
        }
        app
    }

    pub fn save_config(&self) {
        if let Err(e) = self.cfg.save(&self.cfg_path) {
            tracing::warn!(error = %e, "could not save desktop config");
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toasts.push((msg.into(), Instant::now()));
        self.toasts.truncate(6);
    }

    pub fn send(&self, c: Command) {
        if let Some(tx) = &self.cmd {
            let _ = tx.send(c);
        }
    }

    pub fn is_local(&self) -> bool {
        self.cfg.mode == BackendMode::Local
    }

    // ---- connection lifecycle -------------------------------------------

    fn stop_controller(&mut self) {
        self.send(Command::Shutdown);
        self.cmd = None;
        if let Some(h) = self.ctrl.take() {
            h.abort();
        }
    }

    /// (Re)establish the backend from the current config.
    pub fn connect(&mut self, ctx: &egui::Context) {
        self.stop_controller();
        self.view.loaded = false;
        self.view.rows.clear();
        self.view.total = 0;
        self.view.epoch += 1;
        self.pending_offset = None;
        self.selected.clear();
        self.detail = None;
        self.facets.clear();
        let out = Out::new(self.ev_tx.clone(), ctx.clone());
        let slot = self.engine.clone();
        match self.cfg.mode {
            BackendMode::Local => {
                self.conn = Conn::Starting;
                let local = self.cfg.local.clone();
                self.rt.spawn(async move {
                    if let Some(old) = slot.lock().await.take() {
                        old.stop().await;
                    }
                    match backend::start(&local).await {
                        Ok(engine) => {
                            let (url, token) = (engine.url.clone(), engine.token.clone());
                            *slot.lock().await = Some(engine);
                            out.send(Event::EngineReady { url, token });
                        }
                        Err(m) => out.send(Event::EngineFailed(m)),
                    }
                });
            }
            BackendMode::Remote => {
                self.rt.spawn(async move {
                    if let Some(old) = slot.lock().await.take() {
                        old.stop().await;
                    }
                });
                let (url, token) = (self.cfg.remote.url.clone(), self.cfg.remote.token.clone());
                self.start_controller(ctx, &url, &token);
            }
        }
    }

    fn start_controller(&mut self, ctx: &egui::Context, url: &str, token: &str) {
        match Client::new(url, token) {
            Ok(client) => {
                let (tx, rx) = unbounded_channel();
                self.cmd = Some(tx);
                self.conn = Conn::Connecting;
                let out = Out::new(self.ev_tx.clone(), ctx.clone());
                self.ctrl = Some(self.rt.spawn(sync::run(client, rx, out)));
                self.send_window(0);
            }
            Err(e) => self.conn = Conn::Failed(e.to_string()),
        }
    }

    pub fn stop_engine(&mut self) {
        self.stop_controller();
        let slot = self.engine.clone();
        self.rt.spawn(async move {
            if let Some(e) = slot.lock().await.take() {
                e.stop().await;
            }
        });
        self.conn = Conn::Failed("Engine stopped. Use Start engine to launch it.".into());
    }

    fn send_window(&mut self, offset: usize) {
        self.pending_offset = Some(offset);
        self.send(Command::Window {
            epoch: self.view.epoch,
            spec: self.view.spec.clone(),
            offset,
            limit: PAGE,
        });
    }

    // ---- view changes ---------------------------------------------------

    pub fn set_status_filter(&mut self, key: &str) {
        let mut spec = self.view.spec.clone();
        spec.status = key.to_owned();
        self.apply_spec(spec);
    }

    pub fn toggle_sort(&mut self, key: SortKey) {
        let mut spec = self.view.spec.clone();
        if spec.sort == key {
            spec.descending = !spec.descending;
        } else {
            spec.sort = key;
            spec.descending = key == SortKey::Added;
        }
        self.apply_spec(spec);
    }

    fn apply_spec(&mut self, spec: ViewSpec) {
        if self.view.set_spec(spec) {
            self.send_window(0);
        }
    }

    // ---- selection ------------------------------------------------------

    pub fn click_row(&mut self, idx: usize, mods: egui::Modifiers) {
        let Some(hash) = self.view.row(idx).map(|r| r.t.info_hash.clone()) else {
            return;
        };
        if mods.command {
            if !self.selected.remove(&hash) {
                self.selected.insert(hash);
            }
            self.anchor = Some(idx);
        } else if mods.shift {
            let a = self.anchor.unwrap_or(idx);
            let (lo, hi) = (a.min(idx), a.max(idx));
            self.selected.clear();
            for i in lo..=hi {
                if let Some(r) = self.view.row(i) {
                    self.selected.insert(r.t.info_hash.clone());
                }
            }
        } else {
            self.selected.clear();
            self.selected.insert(hash);
            self.anchor = Some(idx);
        }
        self.selection_changed();
    }

    /// Right-click on a row outside the selection selects it first.
    pub fn context_row(&mut self, idx: usize) {
        if let Some(hash) = self.view.row(idx).map(|r| r.t.info_hash.clone()) {
            if !self.selected.contains(&hash) {
                self.selected.clear();
                self.selected.insert(hash);
                self.anchor = Some(idx);
                self.selection_changed();
            }
        }
    }

    pub fn select_all_loaded(&mut self) {
        self.selected = self
            .view
            .rows
            .iter()
            .map(|r| r.t.info_hash.clone())
            .collect();
        self.selection_changed();
    }

    fn selection_changed(&mut self) {
        self.detail = None;
        if self.selected.len() == 1 {
            let h = self.selected.iter().next().cloned().unwrap_or_default();
            self.send(Command::Detail(h));
        }
    }

    pub fn selected_hashes(&self) -> Vec<String> {
        self.selected.iter().cloned().collect()
    }

    pub fn act(&mut self, action: &'static str) {
        let hashes = self.selected_hashes();
        if !hashes.is_empty() {
            self.send(Command::Action { hashes, action });
        }
    }

    pub fn ask_remove(&mut self) {
        let hashes = self.selected_hashes();
        if !hashes.is_empty() {
            self.remove = Some(RemoveDialog {
                hashes,
                delete_files: false,
            });
        }
    }

    // ---- per-frame ------------------------------------------------------

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                Event::Connected => self.conn = Conn::Online,
                Event::Offline(m) => self.conn = Conn::Offline(m),
                Event::AuthFailed => self.conn = Conn::AuthFailed,
                Event::Page {
                    epoch,
                    offset,
                    total,
                    revision,
                    rows,
                } => {
                    if self.view.apply_page(epoch, offset, total, revision, rows) {
                        self.pending_offset = None;
                        self.visible_dirty = true;
                        // Drop selections for torrents that no longer exist in a small list.
                        if self.view.total <= self.view.rows.len() {
                            let present: std::collections::HashSet<_> = self
                                .view
                                .rows
                                .iter()
                                .map(|r| r.t.info_hash.as_str())
                                .collect();
                            self.selected.retain(|h| present.contains(h.as_str()));
                        }
                    }
                }
                Event::Patch(p) => {
                    self.view.apply_patch(&p);
                    if let Some(d) = &mut self.detail {
                        if let Some(u) = p.iter().find(|t| t.info_hash == d.summary.info_hash) {
                            d.summary = u.clone();
                        }
                    }
                }
                Event::Live(l) => self.view.apply_live(&l),
                Event::Transfer(t) => self.transfer = t,
                Event::Facets(f) => self.facets = f.status,
                Event::Detail(d) => {
                    if self.selected.len() == 1 && self.selected.contains(&d.summary.info_hash) {
                        self.detail = Some(*d);
                    }
                }
                Event::Notice(m) => self.toast(m),
                Event::EngineReady { url, token } => {
                    self.start_controller(ctx, &url, &token);
                }
                Event::EngineFailed(m) => self.conn = Conn::Failed(m),
                Event::FilePicked(f) => {
                    if let (Some(add), Some(f)) = (&mut self.add, f) {
                        add.file = Some(f);
                    }
                }
            }
        }
    }

    fn debounce_search(&mut self, ctx: &egui::Context) {
        if self.search != self.search_applied {
            let at = *self.search_changed.get_or_insert_with(Instant::now);
            let wait = Duration::from_millis(300);
            if at.elapsed() >= wait {
                self.search_applied = self.search.clone();
                self.search_changed = None;
                let mut spec = self.view.spec.clone();
                spec.filter = self.search.clone();
                self.apply_spec(spec);
            } else {
                ctx.request_repaint_after(wait - at.elapsed());
            }
        } else {
            self.search_changed = None;
        }
    }

    /// Called by the table once it knows which absolute rows are on screen.
    pub fn viewport_rows(&mut self, first: usize, last: usize) {
        if self.pending_offset.is_none() {
            if let Some(off) = self.view.wanted_window(first, last) {
                self.send_window(off);
            }
        }
        let key = (first, last, self.view.offset);
        if self.visible_dirty || self.last_visible != Some(key) {
            self.last_visible = Some(key);
            self.visible_dirty = false;
            let hashes = self.view.visible_hashes(first, last);
            self.send(Command::Visible(hashes));
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            let Some(path) = f.path else { continue };
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("torrent"))
            {
                match std::fs::read(&path) {
                    Ok(bytes) if bytes.len() <= 16 * 1024 * 1024 => {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let dlg = self.add.get_or_insert_with(|| AddDialog {
                            start: true,
                            ..Default::default()
                        });
                        dlg.file = Some((name, bytes));
                    }
                    Ok(_) => self.toast("Ignored: .torrent file is larger than 16 MiB"),
                    Err(e) => self.toast(format!("Cannot read {}: {e}", path.display())),
                }
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        if ctx.wants_keyboard_input() {
            return;
        }
        let (del, new, find, all) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Delete),
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.consume_key(Modifiers::COMMAND, Key::F),
                i.consume_key(Modifiers::COMMAND, Key::A),
            )
        });
        if del {
            self.ask_remove();
        }
        if new {
            self.add = Some(AddDialog {
                start: true,
                ..Default::default()
            });
        }
        if find {
            self.focus_search = true;
        }
        if all {
            self.select_all_loaded();
        }
    }

    pub fn open_add_file_picker(&self, ctx: &egui::Context) {
        let out = Out::new(self.ev_tx.clone(), ctx.clone());
        self.rt.spawn(async move {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("Torrent", &["torrent"])
                .pick_file()
                .await;
            let result = match picked {
                Some(h) => {
                    let name = h.file_name();
                    let bytes = h.read().await;
                    (bytes.len() <= 16 * 1024 * 1024).then_some((name, bytes))
                }
                None => None,
            };
            out.send(Event::FilePicked(result));
        });
    }

    pub fn submit_add(&mut self) {
        let Some(dlg) = self.add.take() else { return };
        let mut n = 0;
        if let Some((name, bytes)) = dlg.file {
            self.send(Command::Add {
                source: AddSource::File { name, bytes },
                save_path: dlg.save_path.clone(),
                start: dlg.start,
            });
            n += 1;
        }
        for line in dlg.text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            self.send(Command::Add {
                source: AddSource::Magnet(line.to_owned()),
                save_path: dlg.save_path.clone(),
                start: dlg.start,
            });
            n += 1;
        }
        if n == 0 {
            self.toast("Nothing to add");
        }
    }

    pub fn status_counts_label(&self, key: &str) -> Option<u64> {
        self.facets.get(key).copied()
    }

    pub fn selected_status_summary(&self) -> Option<&'static str> {
        let r = self
            .selected
            .iter()
            .next()
            .and_then(|h| self.view.rows.iter().find(|r| &r.t.info_hash == h))?;
        Some(status(&r.t).label())
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events(ctx);
        self.debounce_search(ctx);
        self.handle_drops(ctx);
        self.handle_shortcuts(ctx);

        let minimized = ctx.input(|i| i.viewport().minimized).unwrap_or(false);
        if self.sent_active == minimized {
            self.sent_active = !minimized;
            self.send(Command::Active(!minimized));
        }

        crate::ui::show(self, ctx);

        self.toasts
            .retain(|(_, at)| at.elapsed() < Duration::from_secs(6));
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_controller();
        // The engine is a child of this window: stop it cleanly so fastresume
        // state is flushed before the process exits.
        let slot = self.engine.clone();
        self.rt.block_on(async move {
            if let Some(e) = slot.lock().await.take() {
                e.stop().await;
            }
        });
    }
}
