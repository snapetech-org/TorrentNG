//! Panels. Layout mirrors the WebUI: toolbar, status sidebar, virtualized
//! table, detail dock and status bar.

use eframe::egui::{self, Align, Color32, Frame, Layout, Margin, RichText, Sense, Stroke};
use egui_extras::{Column, TableBuilder};

use crate::app::{AddDialog, App, Conn, DetailTab};
use crate::config::BackendMode;
use crate::fmt;
use crate::model::{self, Row, SortKey, Status};
use crate::sync::Command;
use crate::theme::{self, Palette, ThemeChoice};

const ROW_HEIGHT: f32 = 36.0;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let p = theme::palette(ctx);
    if app.conn == Conn::Setup {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg))
            .show(ctx, |ui| setup_card(app, ui, &p));
        toasts(app, ctx, &p);
        return;
    }
    toolbar(app, ctx, &p);
    status_bar(app, ctx, &p);
    if app.cfg.show_detail && app.conn == Conn::Online {
        detail_dock(app, ctx, &p);
    }
    if app.cfg.show_sidebar {
        sidebar(app, ctx, &p);
    }
    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg))
        .show(ctx, |ui| match app.conn.clone() {
            Conn::Online => table(app, ui, &p),
            Conn::Starting => notice(ui, &p, "Starting the TorrentNG engine…", None, true),
            Conn::Connecting => notice(ui, &p, "Connecting…", None, true),
            Conn::Offline(m) => notice(ui, &p, "Server unreachable — retrying", Some(&m), true),
            Conn::AuthFailed => {
                if notice_button(
                    ui,
                    &p,
                    "Authentication failed",
                    "The API token was rejected.",
                    "Open connection settings",
                ) {
                    open_settings(app);
                }
            }
            Conn::Failed(m) => {
                if notice_button(
                    ui,
                    &p,
                    "Not connected",
                    &m,
                    if app.is_local() {
                        "Start engine"
                    } else {
                        "Retry"
                    },
                ) {
                    app.connect(ui.ctx());
                }
            }
            Conn::Setup => {}
        });
    add_dialog(app, ctx, &p);
    remove_dialog(app, ctx);
    settings_dialog(app, ctx);
    toasts(app, ctx, &p);
}

fn panel_frame(fill: Color32, p: &Palette) -> Frame {
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, p.border))
        .inner_margin(Margin::symmetric(10, 6))
}

fn open_settings(app: &mut App) {
    app.settings = Some(crate::app::SettingsDraft {
        cfg: app.cfg.clone(),
    });
}

// ---- toolbar ------------------------------------------------------------

fn toolbar(app: &mut App, ctx: &egui::Context, p: &Palette) {
    egui::TopBottomPanel::top("toolbar")
        .frame(panel_frame(p.surface, p))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let online = app.conn == Conn::Online;
                if ui
                    .add_enabled(
                        online,
                        egui::Button::new(RichText::new("＋ Add").color(p.accent_text))
                            .fill(p.accent_soft),
                    )
                    .clicked()
                {
                    app.add = Some(AddDialog {
                        start: true,
                        ..Default::default()
                    });
                }
                ui.separator();
                let has = online && !app.selected.is_empty();
                ui.add_enabled_ui(has, |ui| {
                    if ui.button("▶ Start").clicked() {
                        app.act("start");
                    }
                    if ui.button("⏹ Stop").clicked() {
                        app.act("stop");
                    }
                    if ui.button("Recheck").clicked() {
                        app.act("recheck");
                    }
                    if ui.button("Reannounce").clicked() {
                        app.act("reannounce");
                    }
                    if ui
                        .button(RichText::new("Remove…").color(p.danger))
                        .clicked()
                    {
                        app.ask_remove();
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("⚙").on_hover_text("Settings").clicked() {
                        open_settings(app);
                    }
                    let mut d = app.cfg.show_detail;
                    if ui
                        .toggle_value(&mut d, "Details")
                        .on_hover_text("Show or hide the detail dock")
                        .changed()
                    {
                        app.cfg.show_detail = d;
                        app.save_config();
                    }
                    let mut s = app.cfg.show_sidebar;
                    if ui
                        .toggle_value(&mut s, "Filters")
                        .on_hover_text("Show or hide the status filters")
                        .changed()
                    {
                        app.cfg.show_sidebar = s;
                        app.save_config();
                    }
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut app.search)
                            .hint_text("Search name or hash…  (Ctrl+F)")
                            .desired_width(240.0),
                    );
                    if std::mem::take(&mut app.focus_search) {
                        resp.request_focus();
                    }
                });
            });
        });
}

// ---- sidebar ------------------------------------------------------------

const STATUS_FILTERS: [(&str, &str); 8] = [
    ("All", ""),
    ("Downloading", "downloading"),
    ("Seeding", "seeding"),
    ("Completed", "completed"),
    ("Active", "active"),
    ("Inactive", "inactive"),
    ("Stopped", "stopped"),
    ("Error", "error"),
];

fn sidebar(app: &mut App, ctx: &egui::Context, p: &Palette) {
    egui::SidePanel::left("sidebar")
        .resizable(true)
        .default_width(176.0)
        .width_range(130.0..=320.0)
        .frame(panel_frame(p.panel, p))
        .show(ctx, |ui| {
            ui.label(RichText::new("STATUS").small().strong().color(p.faint));
            ui.add_space(4.0);
            let current = app.view.spec.status.clone();
            for (label, key) in STATUS_FILTERS {
                let count = if key.is_empty() {
                    None
                } else {
                    app.status_counts_label(key)
                };
                let text = match count {
                    Some(n) => format!("{label}  ·  {n}"),
                    None => label.to_owned(),
                };
                let selected = current == key;
                let resp = ui.add_sized(
                    [ui.available_width(), 26.0],
                    egui::Button::selectable(selected, text),
                );
                if resp.clicked() && !selected {
                    app.set_status_filter(key);
                }
            }
        });
}

// ---- table --------------------------------------------------------------

fn status_color(s: Status, p: &Palette) -> Color32 {
    match s {
        Status::Downloading => p.accent,
        Status::Seeding => p.success,
        Status::Stopped => p.faint,
        Status::Checking | Status::Finalizing => p.warning,
        Status::Error => p.danger,
        Status::Queued | Status::Metadata => p.muted,
    }
}

#[derive(Clone, Copy)]
enum MenuAction {
    Start,
    Stop,
    Recheck,
    Reannounce,
    Remove,
    CopyHash,
}

fn header_cell(
    ui: &mut egui::Ui,
    label: &str,
    key: Option<SortKey>,
    spec: &model::ViewSpec,
    out: &mut Option<SortKey>,
) {
    let Some(k) = key else {
        ui.label(RichText::new(label).strong());
        return;
    };
    let arrow = if spec.sort == k {
        if spec.descending {
            " ▼"
        } else {
            " ▲"
        }
    } else {
        ""
    };
    let resp = ui
        .add(
            egui::Label::new(RichText::new(format!("{label}{arrow}")).strong())
                .sense(Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        *out = Some(k);
    }
}

fn table(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    if app.view.loaded && app.view.total == 0 {
        let filtered = !app.view.spec.status.is_empty() || !app.view.spec.filter.trim().is_empty();
        let msg = if filtered {
            "No torrents match the current filters."
        } else {
            "No torrents yet — use Add, or drop a .torrent file here."
        };
        notice(ui, p, msg, None, false);
        return;
    }

    let total = app.view.total;
    let mut sort_click: Option<SortKey> = None;
    let mut clicked: Option<(usize, egui::Modifiers)> = None;
    let mut context: Option<usize> = None;
    let mut menu: Option<MenuAction> = None;
    let (mut vis_min, mut vis_max) = (usize::MAX, 0usize);
    let mods = ui.input(|i| i.modifiers);

    {
        let view = &app.view;
        let selected = &app.selected;
        let spec = &view.spec;
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(26.0))
            .column(Column::remainder().at_least(180.0).clip(true))
            .column(Column::initial(96.0).at_least(70.0))
            .column(Column::initial(84.0).at_least(60.0))
            .column(Column::initial(130.0).at_least(80.0))
            .column(Column::initial(92.0).at_least(60.0))
            .column(Column::initial(92.0).at_least(60.0))
            .column(Column::initial(76.0).at_least(50.0))
            .column(Column::initial(56.0).at_least(40.0))
            .column(Column::initial(56.0).at_least(40.0))
            .column(Column::initial(60.0).at_least(40.0))
            .column(Column::initial(92.0).at_least(70.0))
            .header(28.0, |mut h| {
                h.col(|_| {});
                h.col(|ui| header_cell(ui, "Name", Some(SortKey::Name), spec, &mut sort_click));
                h.col(|ui| header_cell(ui, "Status", None, spec, &mut sort_click));
                h.col(|ui| header_cell(ui, "Size", Some(SortKey::Size), spec, &mut sort_click));
                h.col(|ui| {
                    header_cell(
                        ui,
                        "Progress",
                        Some(SortKey::Progress),
                        spec,
                        &mut sort_click,
                    )
                });
                h.col(|ui| header_cell(ui, "↓", None, spec, &mut sort_click));
                h.col(|ui| header_cell(ui, "↑", None, spec, &mut sort_click));
                h.col(|ui| header_cell(ui, "ETA", None, spec, &mut sort_click));
                h.col(|ui| header_cell(ui, "Seeds", None, spec, &mut sort_click));
                h.col(|ui| header_cell(ui, "Peers", None, spec, &mut sort_click));
                h.col(|ui| header_cell(ui, "Ratio", Some(SortKey::Ratio), spec, &mut sort_click));
                h.col(|ui| header_cell(ui, "Added", Some(SortKey::Added), spec, &mut sort_click));
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, total, |mut row| {
                    let idx = row.index();
                    vis_min = vis_min.min(idx);
                    vis_max = vis_max.max(idx);
                    let Some(r) = view.row(idx) else {
                        for _ in 0..12 {
                            row.col(|ui| {
                                ui.label(RichText::new("…").color(p.faint));
                            });
                        }
                        return;
                    };
                    row_cells(
                        &mut row,
                        r,
                        selected.contains(&r.t.info_hash),
                        p,
                        &mut clicked,
                        idx,
                        &mut menu,
                    );
                    let resp = row.response();
                    if resp.clicked() {
                        clicked = Some((idx, mods));
                    }
                    if resp.secondary_clicked() {
                        context = Some(idx);
                    }
                    resp.context_menu(|ui| {
                        for (label, act) in [
                            ("Start", MenuAction::Start),
                            ("Stop", MenuAction::Stop),
                            ("Recheck", MenuAction::Recheck),
                            ("Reannounce", MenuAction::Reannounce),
                            ("Copy info hash", MenuAction::CopyHash),
                            ("Remove…", MenuAction::Remove),
                        ] {
                            if ui.button(label).clicked() {
                                menu = Some(act);
                                ui.close();
                            }
                        }
                    });
                });
            });
    }

    if vis_min != usize::MAX {
        app.viewport_rows(vis_min, vis_max);
    }
    if let Some(k) = sort_click {
        app.toggle_sort(k);
    }
    if let Some((i, m)) = clicked {
        app.click_row(i, m);
    }
    if let Some(i) = context {
        app.context_row(i);
    }
    match menu {
        Some(MenuAction::Start) => app.act("start"),
        Some(MenuAction::Stop) => app.act("stop"),
        Some(MenuAction::Recheck) => app.act("recheck"),
        Some(MenuAction::Reannounce) => app.act("reannounce"),
        Some(MenuAction::Remove) => app.ask_remove(),
        Some(MenuAction::CopyHash) => {
            if let Some(h) = app.selected.iter().next() {
                ui.ctx().copy_text(h.clone());
            }
        }
        None => {}
    }
}

fn row_cells(
    row: &mut egui_extras::TableRow<'_, '_>,
    r: &Row,
    is_selected: bool,
    p: &Palette,
    clicked: &mut Option<(usize, egui::Modifiers)>,
    idx: usize,
    _menu: &mut Option<MenuAction>,
) {
    let t = &r.t;
    let st = model::status(t);
    row.set_selected(is_selected);
    row.col(|ui| {
        let mut checked = is_selected;
        if ui.checkbox(&mut checked, "").changed() {
            // Checkbox toggles like ctrl-click: usable without a keyboard.
            *clicked = Some((
                idx,
                egui::Modifiers {
                    command: true,
                    ..Default::default()
                },
            ));
        }
    });
    row.col(|ui| {
        ui.add(egui::Label::new(&t.name).truncate())
            .on_hover_text(format!("{}\n{}", t.name, t.save_path));
    });
    row.col(|ui| {
        let text = RichText::new(st.label()).color(status_color(st, p));
        let resp = ui.label(text);
        if let Some(m) = t
            .tracker_message
            .as_deref()
            .filter(|m| !m.trim().is_empty())
        {
            resp.on_hover_text(format!("Tracker: {m}"));
        }
    });
    row.col(|ui| {
        ui.label(RichText::new(fmt::size(t.total_length)).color(p.muted));
    });
    row.col(|ui| {
        let prog = model::progress(t);
        let color = if model::is_complete(t) {
            p.success
        } else {
            p.accent
        };
        ui.add(
            egui::ProgressBar::new(prog)
                .desired_height(14.0)
                .fill(color)
                .text(RichText::new(format!("{:.1}%", prog * 100.0)).small()),
        );
    });
    row.col(|ui| {
        let c = if r.down_rate > 0 { p.accent } else { p.faint };
        ui.label(RichText::new(fmt::speed(r.down_rate)).color(c));
    });
    row.col(|ui| {
        let c = if r.up_rate > 0 { p.success } else { p.faint };
        ui.label(RichText::new(fmt::speed(r.up_rate)).color(c));
    });
    row.col(|ui| {
        ui.label(RichText::new(fmt::eta(r.eta_secs())).color(p.muted));
    });
    row.col(|ui| {
        let c = if t.num_seeds > 0 { p.success } else { p.faint };
        ui.label(RichText::new(t.num_seeds.to_string()).color(c));
    });
    row.col(|ui| {
        ui.label(RichText::new(t.num_peers.to_string()).color(p.muted));
    });
    row.col(|ui| {
        let c = if t.ratio >= 1.0 { p.success } else { p.muted };
        ui.label(RichText::new(format!("{:.2}", t.ratio)).color(c));
    });
    row.col(|ui| {
        ui.label(RichText::new(fmt::date(t.added_at)).color(p.muted));
    });
}

// ---- detail dock --------------------------------------------------------

fn detail_dock(app: &mut App, ctx: &egui::Context, p: &Palette) {
    let default_h = if app.cfg.detail_height >= 100.0 {
        app.cfg.detail_height
    } else {
        220.0
    };
    let resp = egui::TopBottomPanel::bottom("detail")
        .resizable(true)
        .default_height(default_h)
        .height_range(110.0..=600.0)
        .frame(panel_frame(p.panel, p))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (DetailTab::General, "General"),
                    (DetailTab::Files, "Files"),
                    (DetailTab::Trackers, "Trackers"),
                ] {
                    ui.selectable_value(&mut app.detail_tab, tab, label);
                }
            });
            ui.separator();
            if app.selected.len() != 1 {
                ui.label(
                    RichText::new(if app.selected.is_empty() {
                        "Select a torrent to see its details."
                    } else {
                        "Select a single torrent to see its details."
                    })
                    .color(p.faint),
                );
                return;
            }
            let Some(d) = app.detail.clone() else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading…").color(p.faint));
                });
                return;
            };
            match app.detail_tab {
                DetailTab::General => detail_general(ui, &d, p),
                DetailTab::Files => detail_files(ui, &d, p),
                DetailTab::Trackers => detail_trackers(ui, &d, p),
            }
        });
    let h = resp.response.rect.height();
    if (h - app.cfg.detail_height).abs() > 8.0 && !ctx.input(|i| i.pointer.any_down()) {
        app.cfg.detail_height = h;
        app.save_config();
    }
}

fn kv(ui: &mut egui::Ui, p: &Palette, k: &str, v: impl Into<String>) {
    ui.label(RichText::new(k).color(p.faint));
    ui.label(v.into());
    ui.end_row();
}

fn detail_general(ui: &mut egui::Ui, d: &model::TorrentDetail, p: &Palette) {
    let s = &d.summary;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("general")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    kv(ui, p, "Name", s.name.clone());
                    kv(ui, p, "Info hash", s.info_hash.clone());
                    kv(ui, p, "Status", model::status(s).label());
                    kv(ui, p, "Size", fmt::size(s.total_length));
                    kv(ui, p, "Remaining", fmt::size(s.amount_left));
                    kv(
                        ui,
                        p,
                        "Progress",
                        format!("{:.2}%", model::progress(s) * 100.0),
                    );
                    kv(ui, p, "Uploaded", fmt::size(s.uploaded));
                    kv(ui, p, "Ratio", format!("{:.3}", s.ratio));
                    kv(
                        ui,
                        p,
                        "Peers / seeds",
                        format!("{} / {}", s.num_peers, s.num_seeds),
                    );
                    kv(ui, p, "Save path", s.save_path.clone());
                    kv(
                        ui,
                        p,
                        "Category",
                        s.category.clone().unwrap_or_else(|| "—".into()),
                    );
                    kv(
                        ui,
                        p,
                        "Tags",
                        if s.tags.is_empty() {
                            "—".into()
                        } else {
                            s.tags.join(", ")
                        },
                    );
                    kv(ui, p, "Added", fmt::date(s.added_at));
                    kv(
                        ui,
                        p,
                        "Completed",
                        s.completed_at.map_or("—".into(), fmt::date),
                    );
                    kv(
                        ui,
                        p,
                        "Pieces",
                        format!("{} × {}", d.piece_count, fmt::size(d.piece_length)),
                    );
                    kv(ui, p, "Private", if d.is_private { "yes" } else { "no" });
                    if let Some(m) = s
                        .tracker_message
                        .as_deref()
                        .filter(|m| !m.trim().is_empty())
                    {
                        kv(ui, p, "Tracker message", m);
                    }
                    if s.finalizing {
                        kv(
                            ui,
                            p,
                            "Finalizing",
                            "Waiting for storage to confirm the data is on disk",
                        );
                    }
                });
        });
}

fn detail_files(ui: &mut egui::Ui, d: &model::TorrentDetail, p: &Palette) {
    TableBuilder::new(ui)
        .striped(true)
        .column(Column::remainder().at_least(160.0).clip(true))
        .column(Column::exact(96.0))
        .column(Column::exact(70.0))
        .header(22.0, |mut h| {
            h.col(|ui| {
                ui.label(RichText::new("Path").strong());
            });
            h.col(|ui| {
                ui.label(RichText::new("Size").strong());
            });
            h.col(|ui| {
                ui.label(RichText::new("Priority").strong());
            });
        })
        .body(|body| {
            body.rows(20.0, d.files.len(), |mut row| {
                let f = &d.files[row.index()];
                row.col(|ui| {
                    ui.add(egui::Label::new(&f.path).truncate());
                });
                row.col(|ui| {
                    ui.label(RichText::new(fmt::size(f.length)).color(p.muted));
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(if f.priority == 0 {
                            "skip".to_owned()
                        } else {
                            f.priority.to_string()
                        })
                        .color(p.muted),
                    );
                });
            });
        });
}

fn detail_trackers(ui: &mut egui::Ui, d: &model::TorrentDetail, p: &Palette) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if d.trackers.is_empty() {
                ui.label(RichText::new("No trackers (DHT/PEX only).").color(p.faint));
            }
            for t in &d.trackers {
                ui.label(t);
            }
        });
}

// ---- status bar ---------------------------------------------------------

fn status_bar(app: &mut App, ctx: &egui::Context, p: &Palette) {
    egui::TopBottomPanel::bottom("status")
        .frame(panel_frame(p.surface, p))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (color, text) = match &app.conn {
                    Conn::Online => (
                        p.success,
                        if app.is_local() {
                            "Local engine"
                        } else {
                            "Connected"
                        }
                        .to_owned(),
                    ),
                    Conn::Starting | Conn::Connecting => (p.warning, "Connecting…".to_owned()),
                    Conn::Offline(_) => (p.warning, "Offline — retrying".to_owned()),
                    Conn::AuthFailed => (p.danger, "Authentication failed".to_owned()),
                    Conn::Failed(_) | Conn::Setup => (p.danger, "Not connected".to_owned()),
                };
                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, color);
                ui.label(RichText::new(text).color(p.muted));
                if app.conn == Conn::Online
                    && app.is_local()
                    && ui.small_button("Stop engine").clicked()
                {
                    app.stop_engine();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if app.conn == Conn::Online {
                        ui.label(
                            RichText::new(format!("{} torrents", app.view.total)).color(p.muted),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(format!("DHT {}", app.transfer.dht_nodes)).color(p.muted),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(format!("↑ {}", fmt::speed(app.transfer.up_info_speed)))
                                .color(p.success),
                        );
                        ui.label(
                            RichText::new(format!("↓ {}", fmt::speed(app.transfer.dl_info_speed)))
                                .color(p.accent),
                        );
                    }
                });
            });
        });
}

// ---- central notices ----------------------------------------------------

fn notice(ui: &mut egui::Ui, p: &Palette, title: &str, detail: Option<&str>, spinner: bool) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.35);
            if spinner {
                ui.spinner();
            }
            ui.label(RichText::new(title).heading().color(p.text));
            if let Some(d) = detail {
                ui.label(RichText::new(d).color(p.faint));
            }
        });
    });
}

fn notice_button(ui: &mut egui::Ui, p: &Palette, title: &str, detail: &str, button: &str) -> bool {
    let mut pressed = false;
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.3);
        ui.label(RichText::new(title).heading().color(p.text));
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(detail).color(p.faint)).wrap());
        ui.add_space(10.0);
        pressed = ui.button(button).clicked();
    });
    pressed
}

// ---- dialogs ------------------------------------------------------------

enum Flow {
    Keep,
    Submit,
    Cancel,
}

fn add_dialog(app: &mut App, ctx: &egui::Context, p: &Palette) {
    let Some(mut dlg) = app.add.take() else {
        return;
    };
    let mut flow = Flow::Keep;
    egui::Window::new("Add torrent")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(460.0)
        .show(ctx, |ui| {
            ui.label("Magnet links (one per line)");
            ui.add(
                egui::TextEdit::multiline(&mut dlg.text)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .hint_text("magnet:?xt=urn:btih:…"),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Choose .torrent file…").clicked() {
                    app.open_add_file_picker(ctx);
                }
                match &dlg.file {
                    Some((name, bytes)) => ui.label(
                        RichText::new(format!("{name} ({})", fmt::size(bytes.len() as i64)))
                            .color(p.success),
                    ),
                    None => ui.label(RichText::new("or drop a file on the window").color(p.faint)),
                };
            });
            ui.add_space(6.0);
            ui.label("Save to (on the engine's machine)");
            ui.add(
                egui::TextEdit::singleline(&mut dlg.save_path)
                    .desired_width(f32::INFINITY)
                    .hint_text("engine default"),
            );
            ui.checkbox(&mut dlg.start, "Start immediately");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let can = !dlg.text.trim().is_empty() || dlg.file.is_some();
                if ui
                    .add_enabled(can, egui::Button::new("Add").fill(p.accent_soft))
                    .clicked()
                {
                    flow = Flow::Submit;
                }
                if ui.button("Cancel").clicked() {
                    flow = Flow::Cancel;
                }
            });
        });
    // A file picked while the dialog is open arrives through the event queue.
    match flow {
        Flow::Keep => app.add = Some(dlg),
        Flow::Submit => {
            app.add = Some(dlg);
            app.submit_add();
        }
        Flow::Cancel => {}
    }
}

fn remove_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(mut dlg) = app.remove.take() else {
        return;
    };
    let mut flow = Flow::Keep;
    egui::Window::new("Remove torrents")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!(
                "Remove {} torrent(s) from the engine?",
                dlg.hashes.len()
            ));
            ui.checkbox(
                &mut dlg.delete_files,
                "Also delete downloaded data from disk",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Remove").clicked() {
                    flow = Flow::Submit;
                }
                if ui.button("Cancel").clicked() {
                    flow = Flow::Cancel;
                }
            });
        });
    match flow {
        Flow::Keep => app.remove = Some(dlg),
        Flow::Submit => {
            app.send(Command::Remove {
                hashes: dlg.hashes,
                delete_files: dlg.delete_files,
            });
            app.selected.clear();
            app.detail = None;
        }
        Flow::Cancel => {}
    }
}

fn settings_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(mut draft) = app.settings.take() else {
        return;
    };
    let mut flow = Flow::Keep;
    egui::Window::new("Settings")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(480.0)
        .show(ctx, |ui| {
            connection_form(ui, &mut draft.cfg);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Theme");
                for c in ThemeChoice::ALL {
                    if ui
                        .selectable_value(&mut draft.cfg.theme, c, c.label())
                        .clicked()
                    {
                        theme::apply(ctx, c);
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    flow = Flow::Submit;
                }
                if ui.button("Cancel").clicked() {
                    flow = Flow::Cancel;
                }
            });
        });
    match flow {
        Flow::Keep => app.settings = Some(draft),
        Flow::Submit => {
            let reconnect = draft.cfg.mode != app.cfg.mode
                || draft.cfg.local != app.cfg.local
                || draft.cfg.remote != app.cfg.remote;
            app.cfg = draft.cfg;
            app.cfg.setup_done = true;
            app.save_config();
            theme::apply(ctx, app.cfg.theme);
            if reconnect {
                app.connect(ctx);
            }
        }
        Flow::Cancel => theme::apply(ctx, app.cfg.theme),
    }
}

/// Connection fields shared by first-run setup and Settings.
fn connection_form(ui: &mut egui::Ui, cfg: &mut crate::config::Config) {
    ui.label(RichText::new("Backend").strong());
    ui.radio_value(
        &mut cfg.mode,
        BackendMode::Local,
        "This computer — run the bundled TorrentNG engine",
    );
    ui.radio_value(
        &mut cfg.mode,
        BackendMode::Remote,
        "Another machine — connect to a TorrentNG server",
    );
    ui.add_space(6.0);
    match cfg.mode {
        BackendMode::Local => {
            egui::Grid::new("local")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Download folder");
                    ui.add(
                        egui::TextEdit::singleline(&mut cfg.local.download_dir)
                            .hint_text("~/Downloads/TorrentNG")
                            .desired_width(280.0),
                    );
                    ui.end_row();
                    ui.label("Peer port");
                    ui.add(egui::DragValue::new(&mut cfg.local.listen_port).range(1..=65535));
                    ui.end_row();
                    ui.label("Engine memory cap (MB)");
                    ui.add(egui::DragValue::new(&mut cfg.local.memory_cap_mb).range(64..=16384));
                    ui.end_row();
                    ui.label("Engine binary");
                    ui.add(
                        egui::TextEdit::singleline(&mut cfg.local.binary)
                            .hint_text("auto-detect")
                            .desired_width(280.0),
                    );
                    ui.end_row();
                });
            ui.checkbox(
                &mut cfg.local.autostart,
                "Start the engine when the app opens",
            );
        }
        BackendMode::Remote => {
            egui::Grid::new("remote")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Server URL");
                    ui.add(
                        egui::TextEdit::singleline(&mut cfg.remote.url)
                            .hint_text("http://nas:8080")
                            .desired_width(280.0),
                    );
                    ui.end_row();
                    ui.label("API token");
                    ui.add(
                        egui::TextEdit::singleline(&mut cfg.remote.token)
                            .password(true)
                            .desired_width(280.0),
                    );
                    ui.end_row();
                });
        }
    }
}

fn setup_card(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.12);
        Frame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1.0_f32, p.border))
            .corner_radius(10)
            .inner_margin(Margin::same(24))
            .show(ui, |ui| {
                ui.set_max_width(520.0);
                ui.label(
                    RichText::new("Welcome to TorrentNG")
                        .heading()
                        .color(p.text),
                );
                ui.label(RichText::new("Choose where your torrents will run.").color(p.faint));
                ui.add_space(12.0);
                ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                    let mut cfg = app.cfg.clone();
                    connection_form(ui, &mut cfg);
                    app.cfg = cfg;
                    ui.add_space(12.0);
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Continue").color(p.accent_text))
                                .fill(p.accent_soft),
                        )
                        .clicked()
                    {
                        app.cfg.setup_done = true;
                        app.save_config();
                        app.connect(ui.ctx());
                    }
                });
            });
    });
}

fn toasts(app: &mut App, ctx: &egui::Context, p: &Palette) {
    if app.toasts.is_empty() {
        return;
    }
    egui::Area::new("toasts".into())
        .anchor(egui::Align2::RIGHT_BOTTOM, [-14.0, -40.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            for (msg, _) in &app.toasts {
                Frame::new()
                    .fill(p.surface2)
                    .stroke(Stroke::new(1.0_f32, p.border_strong))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.label(RichText::new(msg).color(p.text));
                    });
                ui.add_space(4.0);
            }
        });
}
