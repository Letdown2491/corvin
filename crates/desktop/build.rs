fn main() {
    // Declare our app commands so the ACL generates `allow-*` permissions for
    // them — required to grant the localhost (remote-origin) page access to
    // `print` and `save_file` via the capability.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["print", "save_file"])),
    )
    .expect("failed to run tauri-build");
}
