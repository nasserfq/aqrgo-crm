//! AQRGO for Mac, Windows and Linux: a window on aqrgo.com, with Google
//! sign-in through the system browser and the app's own link.

mod mac;
mod nav;

use std::sync::Mutex;

use nav::{site, Link, Nav};
use tauri::{
    webview::{DownloadEvent, PageLoadEvent},
    AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;

/// The one Google sign-in in progress.
#[derive(Default)]
struct SignIn {
    /// Kept here, never sent through the browser.
    verifier: Mutex<Option<String>>,
    /// Posts the code once the window is back on aqrgo.com.
    pending: Mutex<Option<String>>,
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn go(app: &AppHandle, to: &str) {
    if let (Some(w), Ok(url)) = (app.get_webview_window("main"), Url::parse(to)) {
        let _ = w.navigate(url);
    }
}

fn start_google(app: &AppHandle) {
    let (verifier, challenge) = nav::new_verifier();
    *app.state::<SignIn>().verifier.lock().unwrap() = Some(verifier);
    let _ = app
        .opener()
        .open_url(nav::google_url(&challenge), None::<&str>);
}

fn handle_link(app: &AppHandle, url: &Url) {
    match nav::parse_link(url) {
        Some(Link::Code(code)) => {
            let state = app.state::<SignIn>();
            let verifier = state.verifier.lock().unwrap().take();
            let Some(script) = verifier.and_then(|v| nav::handoff_script(&code, &v)) else {
                go(app, &format!("{}/login?error=google", site()));
                return show(app);
            };
            *state.pending.lock().unwrap() = Some(script);
            go(app, &format!("{}/login", site()));
        }
        Some(Link::Error(e)) => go(app, &format!("{}/login?error={e}", site())),
        None => return,
    }
    show(app);
}

fn build_window(app: &AppHandle) -> tauri::Result<()> {
    let nav_app = app.clone();
    // The start page waits for a connection to aqrgo.com; a local test
    // server (debug builds only) opens directly.
    let start = match Url::parse(&format!("{}/login", site())) {
        Ok(url) if site() != nav::SITE => WebviewUrl::External(url),
        _ => WebviewUrl::App("index.html".into()),
    };
    let builder = WebviewWindowBuilder::new(app, "main", start)
        .title("AQRGO")
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0)
        .initialization_script(nav::LINKS_SCRIPT);
    // On Mac: no title bar, the traffic lights sit in the page's toolbar and
    // the sidebar shows the desktop through it, like Finder and Mail.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            18.0, 22.0,
        )))
        .initialization_script(mac::SHELL_SCRIPT);
    // The see-through sidebar needs a private window API, which the Mac App
    // Store refuses; the store build keeps a solid sidebar instead.
    #[cfg(all(target_os = "macos", not(feature = "appstore")))]
    let builder = builder.transparent(true).effects(mac::sidebar_effect());
    #[cfg(all(target_os = "macos", feature = "appstore"))]
    let builder = builder.initialization_script(mac::SOLID_SCRIPT);
    builder
        .on_navigation(move |url| match nav::decide(url) {
            Nav::Stay => true,
            Nav::Browser => {
                let _ = nav_app.opener().open_url(url.as_str(), None::<&str>);
                false
            }
            Nav::Google => {
                start_google(&nav_app);
                false
            }
            Nav::Block => false,
        })
        .on_page_load(|window, payload| {
            if payload.event() != PageLoadEvent::Finished || !nav::is_site(payload.url()) {
                return;
            }
            let script = window.state::<SignIn>().pending.lock().unwrap().take();
            if let Some(js) = script {
                let _ = window.eval(&js);
            }
        })
        .on_download(|webview, event| {
            match event {
                // Exports go to the Downloads folder…
                DownloadEvent::Requested { url, destination } => {
                    let name = destination
                        .file_name()
                        .map(|n| n.to_os_string())
                        .filter(|n| !n.is_empty())
                        .or_else(|| {
                            url.path_segments()
                                .and_then(|mut s| s.next_back().map(Into::into))
                        })
                        .unwrap_or_else(|| "aqrgo.csv".into());
                    if let Ok(dir) = webview.app_handle().path().download_dir() {
                        *destination = dir.join(name);
                    }
                }
                // …and the folder opens with the file selected.
                DownloadEvent::Finished {
                    path: Some(path),
                    success: true,
                    ..
                } => {
                    let _ = webview.app_handle().opener().reveal_item_in_dir(path);
                }
                _ => {}
            }
            true
        })
        .build()?;
    Ok(())
}

pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();
    // Windows and Linux start a second copy for a link; it hands the link
    // to the running app and quits.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)));
    }
    // The Mac menu bar; Windows and Linux windows keep no menu.
    #[cfg(target_os = "macos")]
    {
        builder = builder.menu(|app| mac::menu(app));
    }
    builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(SignIn::default())
        .on_menu_event(|app, event| match mac::action(event.id().as_ref()) {
            Some(mac::Action::Open(path)) => go(app, &format!("{}{path}", site())),
            Some(mac::Action::Run(js)) => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.eval(js);
                }
            }
            None => {}
        })
        .setup(|app| {
            // Installers register the link; this also covers the AppImage.
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            let _ = app.deep_link().register_all();
            build_window(app.handle())?;
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    handle_link(&handle, &url);
                }
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for url in urls {
                    handle_link(app.handle(), &url);
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("AQRGO failed to start");
}
