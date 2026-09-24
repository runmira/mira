fn main() {
    // Declaring the app's commands generates an `allow-<command>`
    // permission for each; the window's pages come from the local server
    // (a remote origin to Tauri), which only gets explicitly allowed ones.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["open_external"])),
    )
    .expect("failed to run tauri-build");
}
