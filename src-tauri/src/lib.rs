mod ai;
pub mod storage;

use ai::*;
use storage::Storage;
use tauri::{Manager, State, WebviewWindowBuilder};

#[tauri::command]
fn load_state(storage: State<'_, Storage>, key: String) -> Result<Option<String>, String> {
    storage.load_state(&key)
}

#[tauri::command]
fn save_state(storage: State<'_, Storage>, key: String, value: String) -> Result<(), String> {
    storage.save_state(&key, &value)
}

#[tauri::command]
fn save_document(storage: State<'_, Storage>, id: String, document: String) -> Result<(), String> {
    storage.save_document(&id, &document)
}

#[tauri::command]
fn load_document(storage: State<'_, Storage>, id: String) -> Result<Option<String>, String> {
    storage.load_document(&id)
}

#[tauri::command]
fn save_pdf(storage: State<'_, Storage>, id: String, data: Vec<u8>) -> Result<(), String> {
    storage.save_pdf(&id, &data)
}

#[tauri::command]
fn load_pdf(storage: State<'_, Storage>, id: String) -> Result<Option<Vec<u8>>, String> {
    storage.load_pdf(&id)
}

#[tauri::command]
fn delete_document(storage: State<'_, Storage>, id: String) -> Result<(), String> {
    storage.delete_document(&id)
}

#[tauri::command]
fn forget_document(storage: State<'_, Storage>, id: String) -> Result<(), String> {
    storage.forget_document(&id)
}

fn local_navigation(url: &tauri::Url) -> bool {
    let packaged = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https")
            && url.host_str() == Some("tauri.localhost")
            && url.port().is_none());
    let development = cfg!(debug_assertions)
        && url.scheme() == "http"
        && url.host_str() == Some("localhost")
        && url.port() == Some(1420);
    url.username().is_empty() && url.password().is_none() && (packaged || development)
}

pub fn run() {
    tauri::Builder::default()
        // One process owns rotating credentials for this application/host.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&directory)?;
            let storage =
                Storage::open(&directory.join("gloss.sqlite3")).map_err(std::io::Error::other)?;
            app.manage(Ai::new(&storage).map_err(std::io::Error::other)?);
            app.manage(storage);
            // Construct from config so the navigation guard exists before the first load.
            WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .on_navigation(local_navigation)
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_state,
            save_state,
            save_document,
            load_document,
            save_pdf,
            load_pdf,
            delete_document,
            forget_document,
            ai_status,
            ai_login,
            ai_cancel_login,
            ai_models,
            ai_select,
            ai_disconnect,
            ai_ask,
            ai_cancel
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Gloss desktop")
        .run(|app, event| {
            // Cmd+Q must use the same draft/state flush as closing the window.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if let Some(window) = app.get_webview_window("main") {
                    api.prevent_exit();
                    let _ = window.close();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_is_local_only() {
        for url in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/",
            "https://tauri.localhost/",
        ] {
            assert!(local_navigation(&url.parse().unwrap()), "{url}");
        }
        for url in [
            "https://example.com/",
            "https://tauri.localhost.evil.test/",
            "file:///tmp/index.html",
            "javascript:alert(1)",
            "http://localhost:3000/",
            "http://user@localhost:1420/",
        ] {
            assert!(!local_navigation(&url.parse().unwrap()), "{url}");
        }
        assert_eq!(
            local_navigation(&"http://localhost:1420/".parse().unwrap()),
            cfg!(debug_assertions)
        );
    }
}
