mod ai;
mod aliases;
mod commands;
mod files;
mod icons;
mod instant;
mod providers;
mod ranking;
mod reminders;
mod semantic;
mod timeparse;
mod window;

use parking_lot::RwLock;
use std::sync::Arc;
use tauri::Manager;

/// Everything search needs, kept in RAM so each keystroke is just a memory scan.
pub struct AppState {
    pub apps: RwLock<Vec<providers::apps::AppEntry>>,
    pub usage: ranking::Usage,
    pub aliases: aliases::Aliases,
    pub icon_dir: std::path::PathBuf,
    pub ai: ai::AiConfig,
    pub reminders: reminders::Reminders,
    pub semantic: Arc<semantic::Semantic>,
    pub files: Arc<files::FileIndex>,
}

pub fn run() {
    // Load keys from the project's .env file (searched from the current folder upward).
    let _ = dotenvy::dotenv();

    tauri::Builder::default()
        // Only one Spotlight at a time: opening it again just shows the search bar.
        // (Must be the first plugin registered.)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::toggle(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let _ = std::fs::create_dir_all(&data_dir);

            // Installed app: turn on "start with Windows" the first time it runs.
            // After that the Settings switch decides. (Skipped in dev builds.)
            #[cfg(not(debug_assertions))]
            {
                use tauri_plugin_autostart::ManagerExt;
                let marker = data_dir.join("autostart-initialized");
                if !marker.exists() {
                    let _ = app.autolaunch().enable();
                    let _ = std::fs::write(&marker, "1");
                }
            }
            let icon_dir = data_dir.join("icons");
            let state = Arc::new(AppState {
                apps: RwLock::new(Vec::new()),
                usage: ranking::Usage::load(data_dir.clone()),
                aliases: aliases::Aliases::load(data_dir.clone()),
                ai: ai::AiConfig::load(data_dir.clone()),
                reminders: reminders::Reminders::load(data_dir.clone()),
                semantic: Arc::new(semantic::Semantic::load(data_dir.clone())),
                files: Arc::new(files::FileIndex::new(data_dir)),
                icon_dir: icon_dir.clone(),
            });
            app.manage(state.clone());
            reminders::start(app.handle().clone(), state.clone());
            // Index documents for search-by-meaning in the background.
            state.semantic.clone().start();
            // Index file and folder names on all drives (saved, so restarts are instant).
            state.files.clone().start();

            // Load the AI model in the background so the first sentence isn't slow.
            let ai_cfg = state.ai.get();
            if ai_cfg.enabled {
                std::thread::spawn(move || ai::warmup(&ai_cfg.model));
            }

            // Build the app index off the main thread so startup stays instant,
            // then refresh it every few minutes to pick up newly installed apps.
            std::thread::spawn(move || loop {
                let apps = providers::apps::scan();
                println!("Indexed {} apps", apps.len());
                let targets: Vec<String> = apps.iter().map(|a| a.path.clone()).collect();
                *state.apps.write() = apps;
                // Warm the icon cache so results appear with icons immediately.
                for t in targets {
                    icons::get(&t, &icon_dir);
                }
                std::thread::sleep(std::time::Duration::from_secs(300));
            });

            window::setup_hotkey(app)?;
            window::setup_tray(app)?;
            window::hide_on_blur(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::execute,
            commands::hide_window,
            commands::open_settings,
            commands::list_aliases,
            commands::set_alias,
            commands::remove_alias,
            commands::get_icon,
            commands::ai_interpret,
            commands::ai_answer,
            commands::snooze_reminder,
            commands::ai_models,
            commands::get_ai_settings,
            commands::set_ai_settings,
            commands::semantic_search,
            commands::get_semantic_settings,
            commands::set_semantic_settings,
            commands::semantic_status,
            commands::semantic_reindex,
            commands::pick_folder,
            commands::file_search,
            commands::reveal_file,
            commands::file_index_status,
            commands::file_index_refresh,
            commands::get_autostart,
            commands::set_autostart,
            commands::instant_answer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Spotlight");
}
