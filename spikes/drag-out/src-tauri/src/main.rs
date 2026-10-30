// spikes/drag-out/src-tauri/src/main.rs
// Cargo.toml (spike-local):
//   [dependencies]
//   tauri = { version = "2", features = [] }
//   tauri-plugin-drag = "2"
//   serde = { version = "1", features = ["derive"] }
//
// capabilities/default.json must include:  "permissions": ["drag:default"]
// tauri.conf.json: point frontendDist at ../ui for a no-framework spike.

/// Resolve DB file ids -> absolute paths and hand them to the OS drag.
/// In the real app this is where selection ids become real paths; keeping
/// that lookup Rust-side means the webview never sees the filesystem.
#[tauri::command]
async fn start_file_drag(
    window: tauri::WebviewWindow,
    paths: Vec<String>,
    icon: Option<String>,
) -> Result<(), String> {
    // Plugin exposes the same start_drag the JS binding uses; the command
    // wrapper exists because real app drag-outs start from file IDs.
    // Signature per crabnebula-dev/drag-rs (v2 API):
    //   tauri_plugin_drag::start_drag(&window, item, icon)
    // where item is DragItem::Files(Vec<PathBuf>) — check the pinned
    // version's docs; the JS binding below is the documented stable path.
    let _ = (window, paths, icon);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_drag::init())
        .invoke_handler(tauri::generate_handler![start_file_drag])
        .setup(|app| {
            #[cfg(debug_assertions)]
            {
                let url = "http://localhost:5173";
                let _ = app.get_webview_window("main").map(|w| w.navigate(url.parse().unwrap()));
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("drag-out spike crashed");
}
