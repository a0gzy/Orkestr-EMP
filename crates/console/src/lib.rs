mod app;
mod deployment_view;
mod details;
mod discovery_view;
mod editor;
mod import_view;
mod remote;
mod theme;

pub use app::Console;

#[cfg(test)]
mod theme_tests;

pub fn run() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let mut database = None;
    let mut capture = None;
    let mut capture_view = String::new();
    let mut forced_theme = None;
    let mut zoom = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--database" => database = args.next().map(std::path::PathBuf::from),
            "--capture" => capture = args.next().map(std::path::PathBuf::from),
            "--capture-view" => capture_view = args.next().unwrap_or_default(),
            "--theme" => forced_theme = args.next(),
            "--zoom" => zoom = args.next().and_then(|v| v.parse::<f32>().ok()),
            "--help" => {
                println!("Orkestr EMP local / HTTPS server console\n--database <EXISTING.sqlite>\n--capture <NEW.png> (explicit smoke screenshot, closes afterward)\nWithout --database: choose/create a local database or explicitly connect to M2 server in UI. No default data, automatic connections or installs.");
                return Ok(());
            }
            _ => {
                eprintln!("Unknown arguments; use --help");
                return Ok(());
            }
        }
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([840.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Orkestr EMP",
        options,
        Box::new(move |cc| {
            let mut app = Console::new(cc, database, capture);
            app.capture_view = capture_view;
            if let Some(theme) = forced_theme {
                app.preferences.theme = match theme.as_str() {
                    "light" => theme::ThemeChoice::Light,
                    "dark" => theme::ThemeChoice::Dark,
                    _ => theme::ThemeChoice::System,
                };
            }
            if let Some(zoom) = zoom {
                app.preferences.zoom = zoom;
            }
            Ok(Box::new(app))
        }),
    )
}
