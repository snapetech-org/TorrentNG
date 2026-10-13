#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use torrentng_desktop::app::App;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("TorrentNG")
        .with_app_id("torrentng-desktop")
        .with_inner_size([1280.0, 780.0])
        .with_min_inner_size([760.0, 420.0])
        .with_drag_and_drop(true);

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        // Software-vsync'd, repaint-on-demand: no idle redraw loop.
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "TorrentNG",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
