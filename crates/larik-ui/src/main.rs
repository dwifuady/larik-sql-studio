#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod completion;
mod connection_dialog;
mod grid_actions_ui;
mod history_page;
mod mcp_bridge;
mod mcp_page;
mod notes_ui;
mod results_table;
mod settings_page;
mod snippets_page;
mod studio;
mod theme;
mod window_title_bar;

use std::sync::Arc;

use gpui_kit::assets::AllAssets;
use gpui_kit::component::TitleBar;
use gpui_kit::{
    App, AppContext as _, Bounds, DisplayId, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, point, px, size,
};
use larik_core::session::SessionManager;
use larik_core::store::StudioStore;
use larik_core::{WindowMode, WindowPlacementState};
use larik_data::ProviderCatalog;
use larik_provider_demo::DemoProvider;
use larik_provider_mongodb::MongoProvider;
use larik_provider_mssql::SqlServerProvider;
use larik_provider_postgres::PostgresProvider;

use crate::studio::{StudioView, bind_studio_keys};

const DEFAULT_WINDOW_WIDTH: f32 = 1360.0;
const DEFAULT_WINDOW_HEIGHT: f32 = 860.0;
const MIN_WINDOW_WIDTH: f32 = 900.0;
const MIN_WINDOW_HEIGHT: f32 = 600.0;

fn resolve_initial_window_bounds(
    saved: Option<&WindowPlacementState>,
    cx: &App,
) -> (WindowBounds, Option<DisplayId>) {
    let default_bounds = || {
        (
            WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(DEFAULT_WINDOW_WIDTH), px(DEFAULT_WINDOW_HEIGHT)),
                cx,
            )),
            None,
        )
    };

    let Some(placement) = saved else {
        return default_bounds();
    };
    if !placement.x.is_finite()
        || !placement.y.is_finite()
        || !placement.width.is_finite()
        || !placement.height.is_finite()
        || placement.width < 200.0
        || placement.height < 200.0
    {
        return default_bounds();
    }

    let wrap_mode = |bounds| match placement.mode {
        WindowMode::Windowed => WindowBounds::Windowed(bounds),
        WindowMode::Maximized => WindowBounds::Maximized(bounds),
        WindowMode::Fullscreen => WindowBounds::Fullscreen(bounds),
    };

    let restored_size = size(
        px(placement.width.max(MIN_WINDOW_WIDTH)),
        px(placement.height.max(MIN_WINDOW_HEIGHT)),
    );
    let restored_bounds = Bounds {
        origin: point(px(placement.x), px(placement.y)),
        size: restored_size,
    };
    let center = restored_bounds.center();

    let displays = cx.displays();
    // Prefer the exact saved display if its bounds still contain the window center;
    // otherwise fall back to any connected display containing the center.
    let matched_display = placement
        .display_uuid
        .as_deref()
        .and_then(|uuid| {
            displays.iter().find(|d| {
                d.uuid().ok().map(|u| u.to_string()).as_deref() == Some(uuid)
                    && d.bounds().contains(&center)
            })
        })
        .or_else(|| displays.iter().find(|d| d.bounds().contains(&center)));

    if let Some(display) = matched_display {
        return (wrap_mode(restored_bounds), Some(display.id()));
    }

    // Monitor was unplugged or coordinates are off-screen: preserve preferred size
    // (clamped to primary display if available) and center on the primary monitor.
    let clamped_size = if let Some(primary) = cx.primary_display() {
        let vis = primary.visible_bounds().size;
        let max_w = f32::from(vis.width).max(MIN_WINDOW_WIDTH);
        let max_h = f32::from(vis.height).max(MIN_WINDOW_HEIGHT);
        size(
            px(placement.width.clamp(MIN_WINDOW_WIDTH, max_w)),
            px(placement.height.clamp(MIN_WINDOW_HEIGHT, max_h)),
        )
    } else {
        restored_size
    };
    let centered = Bounds::centered(None, clamped_size, cx);
    (wrap_mode(centered), None)
}

fn main() {
    let mut catalog = ProviderCatalog::new();
    catalog.register(Arc::new(DemoProvider));
    catalog.register(Arc::new(SqlServerProvider));
    catalog.register(Arc::new(PostgresProvider));
    catalog.register(Arc::new(MongoProvider));

    let store = StudioStore::open_default().expect("failed to initialize Larik SQL Studio store");
    let saved_placement = store.load_window_placement();
    let sessions =
        Arc::new(SessionManager::new(catalog.clone()).expect("failed to start Tokio runtime"));

    gpui_kit::application()
        .with_assets(AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            bind_studio_keys(cx);

            let (window_bounds, display_id) =
                resolve_initial_window_bounds(saved_placement.as_ref(), cx);
            let options = WindowOptions {
                window_bounds: Some(window_bounds),
                display_id,
                window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT))),
                window_background: WindowBackgroundAppearance::MicaBackdrop,
                ..TitleBar::window_options()
            };

            gpui_kit::open_window(options, cx, |window, cx| {
                window.set_window_title("Larik SQL Studio");
                cx.new(|cx| {
                    StudioView::new(catalog.clone(), store, sessions.clone(), window, cx)
                })
            })
            .expect("failed to open Larik SQL Studio window");

            cx.activate(true);
        });
}
