//! `utilify-workbench`: Utilify's data workbench, in its own process. The
//! app starts it from the menu, the dashboard or `utilifyctl workbench
//! open` (see `utilify_app::workbench`), and it lives next to the app's
//! executable. A second copy brings the first forward and quits.

mod workbench;

use gpui_kit::*;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // Held until the process exits, which is when the system lets go of it.
    let Some(_claim) = utilify_app::workbench::claim() else {
        log::info!("A workbench is already open; brought it forward");
        return;
    };
    log::info!(
        "Starting the {} workbench v{}",
        utilify_core::PRODUCT_NAME,
        env!("CARGO_PKG_VERSION")
    );

    gpui_kit::application()
        .with_assets(utilify_app::assets::AppAssets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            // A normal app while it runs: Dock icon, app switcher, menu bar.
            // The bundle's LSUIElement would otherwise keep it out of them.
            #[cfg(target_os = "macos")]
            cx.set_activation_policy(ActivationPolicy::Regular);
            gpui_kit::init(cx);
            utilify_app::theme::init(cx);
            workbench::init(cx);
            workbench::open(cx);
        });
}
