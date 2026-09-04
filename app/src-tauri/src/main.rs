// Minimal Tauri shell scaffold for THE FOUNDRY's V-01 visual floor.
//
// NOT built or exercised in this sandbox (no webkit2gtk available) — this
// file exists only so the eventual desktop shell has a starting point.
// The web build (`npm run build` + `npm run preview`) is the thing that is
// actually required to work here; see app/README.md.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running the-foundry tauri application");
}
