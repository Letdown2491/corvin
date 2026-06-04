# Corvin desktop app (`corvin-desktop`)

The desktop build wraps Corvin in a [Tauri 2](https://tauri.app) shell. This
documents how it's put together, how to build it, and the platform-specific
gotchas we hit along the way.

## Architecture

The desktop app does **not** reimplement the backend as Tauri commands. It runs
the existing axum server **in-process** and points a webview at it:

1. On startup (`src/main.rs` `setup`), it calls `corvin::build_app()` (the
   server crate's library entry — see `crates/server/src/lib.rs`), which loads
   config, starts the background subscribers, and returns the axum `Router` +
   the address to bind.
2. It **binds the `TcpListener` before opening the window** (so the webview can
   never race an unbound port), then spawns `axum::serve` on Tauri's tokio
   runtime for the app's lifetime.
3. It opens a `WebviewWindow` pointed at `http://127.0.0.1:<port>` as an
   **external URL**.

The Svelte SPA is built to `frontend/dist` and embedded into the server binary
via `rust_embed`; the in-process axum server serves it. So the webview loads the
real UI over loopback HTTP, and the SPA's relative `/api` + `/events` calls are
same-origin — no CORS, no IPC needed for normal wallet operations.

```
crates/desktop/
  src/main.rs           Tauri shell: setup, window, IPC commands, link handling
  build.rs              tauri-build + declares app commands (print, save_file)
  tauri.conf.json       window/bundle/security config (withGlobalTauri, etc.)
  capabilities/         ACL: grants the localhost origin access to commands
  splash/               placeholder frontendDist (real UI is the embedded SPA)
  Info.plist            macOS camera/mic usage strings (merged by mac bundler)
  Entitlements.plist    macOS hardened-runtime camera entitlement
  icons/                app icons (.png/.icns/.ico)
```

### IPC (only for native OS actions)

Most of the app uses plain HTTP. A few things the webview can't do itself go
through Tauri commands:

- `print` — runs the native print dialog (WebKitGTK `PrintOperation` on Linux,
  `window.print()` elsewhere). `window.print()` from JS is a no-op in WebKitGTK.
- `save_file(name, contents)` — native Save dialog (`tauri-plugin-dialog`) +
  write. The webview won't persist `<a download>` blob clicks, so
  `downloadBlob` (frontend `lib/utils.ts`) routes here on desktop.

For these to be callable from the localhost page (a *remote* origin to Tauri):

- `app.withGlobalTauri: true` exposes `window.__TAURI__.core.invoke`.
- `capabilities/default.json` sets `remote.urls: ["http://127.0.0.1:*", ...]`
  and lists `allow-print` / `allow-save-file` (generated from the app-command
  declaration in `build.rs`).
- The server's CSP (`crates/server/src/lib.rs` `SECURITY_HEADERS`) allows the
  IPC transport: `connect-src 'self' ipc: http://ipc.localhost`.

The frontend detects the desktop shell via `isDesktop()` / `invokeDesktop()` in
`frontend/src/lib/utils.ts` and falls back to browser behaviour otherwise.

## Prerequisites

- **Rust** (stable) and **Node + npm**.
- **GTK3 + WebKit development libraries** (the `-sys` crates link against them).
  On Fedora:
  ```sh
  sudo dnf install gtk3-devel glib2-devel cairo-devel pango-devel \
    gdk-pixbuf2-devel atk-devel webkit2gtk4.1-devel libsoup3-devel
  ```
  Missing these shows up as `pkg-config` errors like `gio-2.0 was not found` —
  install the matching `-devel` package.
- **Tauri CLI** (for bundling): `npm install -g @tauri-apps/cli` (provides
  `tauri`) or `cargo install tauri-cli` (provides `cargo tauri`).
- **For bundling installers** (optional): Linux needs `rpm-build` / `dpkg`;
  AppImage tooling is auto-downloaded by Tauri (needs network + FUSE).

## Develop

```sh
just dev-desktop      # builds the frontend, then `cargo run -p corvin-desktop`
```

The frontend is **embedded**, not hot-reloaded. In debug builds `rust_embed`
reads `frontend/dist` from disk at request time, so after changing frontend code
rebuild it (`just build-frontend`) **and reload/restart the app** — see the
caching note below.

## Build a binary

```sh
just build-desktop    # frontend + `cargo build --release -p corvin-desktop`
# -> target/release/corvin-desktop  (self-contained; SPA embedded at compile time)
```

Note: **release** builds embed `frontend/dist` at compile time, so it must exist
and be current before the release `cargo build` (the just targets handle this).

## Bundle installers

```sh
just bundle-desktop   # frontend + `tauri build` from crates/desktop
```

- The Tauri CLI resolves `crates/desktop/tauri.conf.json` directly (no
  `src-tauri/` dir, no `--config` flag needed).
- `tauri build --no-bundle` compiles the release binary only (useful to validate
  without packaging tools).
- `tauri build --bundles rpm` (or `deb`, `appimage`) scopes the output.

Bundling must run **natively per OS** — no cross-compiling (native webview
toolchains). Build the macOS app on macOS, the Windows app on Windows.

## Platform notes

### Linux
Primary target. Inside a **toolbox/flatpak sandbox** there's no `xdg-open` and no
packaging tools, so:
- External links are opened via `flatpak-spawn --host xdg-open` (see
  `open_external` in `main.rs`); outside a sandbox it uses the `open` crate.
- For installers, run on the host or install the packaging tools in the sandbox.

### macOS
- `Info.plist` (`NSCameraUsageDescription` / `NSMicrophoneUsageDescription`) is
  **required** — WKWebView kills the app on `getUserMedia` without it. It's
  auto-merged by the macOS bundler.
- `Entitlements.plist` grants the camera entitlement for the hardened runtime;
  wired via `bundle.macOS.entitlements`. Needed for notarized distribution.
- Camera permission is auto-granted by wry's WKUIDelegate; no extra code.

### Windows
- Uses WebView2. Camera triggers WebView2's native permission prompt (works; no
  extra code). `windows_subsystem = "windows"` suppresses the console window in
  release.

## Troubleshooting

- **Blank app / `'text/html' is not a valid JavaScript MIME type` / 404s for
  `_app/immutable/*.js` with mismatched hashes** — a stale page is referencing
  chunks a rebuild deleted. The service worker is now network-first and the
  server sends `Cache-Control: no-store`, but to clear an already-installed old
  worker: stop the app, `rm -rf ~/.local/share/dev.corvin.app` (webview cache /
  SW / localStorage only — wallets live in `~/.config/corvin/`), relaunch.
  Always reload/restart after a frontend rebuild.
- **A modal opens but is a ~2px sliver** — WebKitGTK collapses an auto-height
  column flex container whose child has `flex-basis: 0` (the `flex: 1`
  shorthand). Use `flex: 1 1 auto` on the modal's scroll child (or give the
  dialog an explicit height). Affected SendModal/Consolidate/FeeBump.
- **Print/Download/external links do nothing** — these need the IPC + native
  wiring above. Check: CSP allows `ipc:`, the capability has `remote.urls` +
  `allow-print`/`allow-save-file`, and `withGlobalTauri` is on.
- **`pkg-config` / `gio-2.0 not found` at build** — install the GTK/WebKit
  `-devel` packages (see Prerequisites).
