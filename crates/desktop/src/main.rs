// Prevent a console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicUsize, Ordering};

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Url, WebviewUrl, WebviewWindowBuilder};

static NEXT_WINDOW_ID: AtomicUsize = AtomicUsize::new(1);

fn main() {
    corvin::init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![print, save_file])
        .setup(|app| {
            let handle = app.handle().clone();

            // Build the router + bind the listener *before* opening the window,
            // so the webview never races an unbound port. The server then runs
            // in-process on Tauri's tokio runtime for the app's lifetime.
            let (router, addr) = tauri::async_runtime::block_on(corvin::build_app())?;
            let listener = tauri::async_runtime::block_on(tokio::net::TcpListener::bind(addr))?;
            let local = listener.local_addr()?;

            tauri::async_runtime::spawn(async move {
                if let Err(e) = axum::serve(listener, router).await {
                    tracing::error!("corvin server stopped: {e}");
                }
            });

            let local_host = local.ip().to_string();
            let url: tauri::Url = format!("http://{local}").parse()?;

            let nav_host = local_host.clone();
            let nw_handle = handle.clone();
            let nw_host = local_host.clone();

            // Used below for the Linux-only WebKitGTK setup; unused on other targets.
            #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
            let window = WebviewWindowBuilder::new(&handle, "main", WebviewUrl::External(url))
                .title("Corvin")
                .inner_size(1280.0, 832.0)
                .min_inner_size(900.0, 600.0)
                // Same-frame navigations: hand off-origin pages to the system
                // browser. (Corvin's own routing is client-side and never hits
                // this; reload/internal loads pass through.)
                .on_navigation(move |url| {
                    if is_off_origin(url, &nav_host) {
                        open_external(url);
                        return false;
                    }
                    true
                })
                // target=_blank / window.open requests. WebKitGTK routes these
                // here, not through on_navigation.
                .on_new_window(move |url, _features| {
                    if is_http(&url)
                        && !is_off_origin(&url, &nw_host)
                        && !url.path().ends_with(".pdf")
                    {
                        // Same-origin in-app page (e.g. /help): the desktop
                        // analog of "open in a new tab" is a second window.
                        let label =
                            format!("ext-{}", NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed));
                        match WebviewWindowBuilder::new(
                            &nw_handle,
                            label,
                            WebviewUrl::External(url),
                        )
                        .title("Corvin")
                        .inner_size(1000.0, 760.0)
                        .build()
                        {
                            Ok(window) => return NewWindowResponse::Create { window },
                            Err(e) => {
                                tracing::warn!("failed to open child window: {e}");
                                return NewWindowResponse::Deny;
                            }
                        }
                    }
                    if is_http(&url) {
                        // Off-origin link or a bundled PDF the webview can't
                        // render inline: open it in the system browser.
                        open_external(&url);
                    }
                    // about:/data:/empty (e.g. the seed print popup) has nothing
                    // to hand off; deny so window.open() returns null.
                    NewWindowResponse::Deny
                })
                .build()?;

            // QR scanning needs getUserMedia, which WebKitGTK gates behind both
            // a settings flag and the permission-request signal. Grant only
            // camera/mic; deny everything else (geolocation, notifications, …).
            #[cfg(target_os = "linux")]
            {
                let dp_host = local_host.clone();
                let dp_handle = handle.clone();
                window.with_webview(move |webview| {
                    use webkit2gtk::glib::prelude::Cast;
                    use webkit2gtk::{
                        NavigationPolicyDecisionExt, PermissionRequestExt, PolicyDecisionExt,
                        PolicyDecisionType, PrintOperationExt, SettingsExt, URIRequestExt,
                        WebViewExt,
                    };

                    let wv = webview.inner();
                    if let Some(settings) = WebViewExt::settings(&wv) {
                        settings.set_enable_media_stream(true);
                        settings.set_enable_mediasource(true);
                    }
                    // Intercept new-window requests (target=_blank / window.open) at
                    // policy time and handle them ourselves, so WebKitGTK never enters
                    // its window-creation path. That path dereferences an empty
                    // optional<WindowFeatures>, which Fedora's hardened libstdc++ turns
                    // into an abort; ignoring the policy decision sidesteps it. This is
                    // the Linux equivalent of (and pre-empts) the `on_new_window`
                    // handler above, which still serves macOS / Windows.
                    wv.connect_decide_policy(move |_wv, decision, kind| {
                        if kind != PolicyDecisionType::NewWindowAction {
                            return false; // let WebKit handle normal navigation/responses
                        }
                        let target = decision
                            .dynamic_cast_ref::<webkit2gtk::NavigationPolicyDecision>()
                            .and_then(|d| d.navigation_action())
                            .and_then(|a| a.request())
                            .and_then(|r| r.uri())
                            .and_then(|u| Url::parse(u.as_str()).ok());
                        if let Some(target) = target {
                            if is_http(&target)
                                && !is_off_origin(&target, &dp_host)
                                && !target.path().ends_with(".pdf")
                            {
                                // Same-origin in-app page (e.g. /help) → a second
                                // window. Defer so we don't build it mid-signal.
                                let h = dp_handle.clone();
                                let _ = dp_handle.run_on_main_thread(move || {
                                    let label = format!(
                                        "ext-{}",
                                        NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed)
                                    );
                                    if let Err(e) = WebviewWindowBuilder::new(
                                        &h,
                                        label,
                                        WebviewUrl::External(target),
                                    )
                                    .title("Corvin")
                                    .inner_size(1000.0, 760.0)
                                    .build()
                                    {
                                        tracing::warn!("failed to open child window: {e}");
                                    }
                                });
                            } else if is_http(&target) {
                                open_external(&target);
                            }
                        }
                        decision.ignore();
                        true
                    });
                    wv.connect_permission_request(|_, req| {
                        if req
                            .dynamic_cast_ref::<webkit2gtk::UserMediaPermissionRequest>()
                            .is_some()
                        {
                            req.allow();
                        } else {
                            req.deny();
                        }
                        true
                    });
                    // window.print() (e.g. the seed backup) only emits this signal in
                    // WebKitGTK — nothing runs the dialog unless we do it ourselves.
                    wv.connect_print(|web_view, op| {
                        use gtk::prelude::WidgetExt;
                        let parent = web_view
                            .toplevel()
                            .and_then(|w| w.downcast::<gtk::Window>().ok());
                        op.run_dialog(parent.as_ref());
                        true
                    });
                })?;
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Corvin");
}

fn is_http(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

/// An http(s) URL whose host isn't our loopback server.
fn is_off_origin(url: &Url, local_host: &str) -> bool {
    if !is_http(url) {
        return false;
    }
    match url.host_str() {
        Some(h) => {
            !(h == local_host || h == "localhost" || h == "127.0.0.1" || h == "::1" || h == "[::1]")
        }
        None => false,
    }
}

/// Open a URL in the host's default browser. Done in Rust (not the opener
/// plugin, whose injected JS hijacked link clicks and whose xdg-open is absent
/// in a toolbox).
fn open_external(url: &Url) {
    let url = url.as_str();
    // Inside a toolbox/flatpak sandbox there's no xdg-open — bridge to the host.
    #[cfg(target_os = "linux")]
    if std::path::Path::new("/run/.containerenv").exists()
        || std::path::Path::new("/.flatpak-info").exists()
    {
        if let Err(e) = std::process::Command::new("flatpak-spawn")
            .args(["--host", "xdg-open", url])
            .spawn()
        {
            tracing::warn!("failed to open external url {url}: {e}");
        }
        return;
    }
    // Linux (host), macOS, Windows: the OS opener (xdg-open / open / start).
    if let Err(e) = open::that_detached(url) {
        tracing::warn!("failed to open external url {url}: {e}");
    }
}

/// Print the current page. Runs the WebKitGTK print dialog with the window as
/// parent (a null parent doesn't reliably show). The frontend sets up its
/// print-only DOM (e.g. the seed backup) before invoking this.
#[tauri::command]
fn print(window: tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        window
            .with_webview(|webview| {
                use gtk::prelude::WidgetExt;
                use webkit2gtk::glib::prelude::Cast;
                use webkit2gtk::PrintOperationExt;
                let wv = webview.inner();
                let op = webkit2gtk::PrintOperation::new(&wv);
                let parent = wv.toplevel().and_then(|w| w.downcast::<gtk::Window>().ok());
                op.run_dialog(parent.as_ref());
            })
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        window.print().map_err(|e| e.to_string())
    }
}

/// Save bytes to a user-chosen path via the native Save dialog. Replaces
/// browser blob downloads, which the desktop webview doesn't persist.
#[tauri::command]
fn save_file(app: AppHandle, name: String, contents: Vec<u8>) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(path) = app
        .dialog()
        .file()
        .set_file_name(&name)
        .blocking_save_file()
    else {
        return Ok(false); // user cancelled
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, &contents).map_err(|e| e.to_string())?;
    Ok(true)
}
