//! The Mac look: a see-through sidebar over the desktop, the traffic lights
//! inside the toolbar, and a real menu bar with the usual shortcuts.
//!
//! The site styles itself for the app when `<html data-shell="mac">` is set
//! (see `crates/common/static/mac.css`); the script below sets it.

use tauri::{
    menu::{AboutMetadata, Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder},
    utils::config::WindowEffectsConfig,
    window::{Effect, EffectState, EffectsBuilder},
    AppHandle, Runtime,
};

/// Runs in every page: marks the page as shown in the Mac app, adds the
/// toolbar strip that moves the window, and lets the sidebar and page
/// headers move it too (buttons and links inside them still click).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const SHELL_SCRIPT: &str = r#"(function () {
  function mark() {
    if (document.documentElement) document.documentElement.setAttribute("data-shell", "mac");
  }
  mark();
  function ready() {
    mark();
    var b = document.body;
    if (!b) return;
    if (!b.querySelector(":scope > .mac-titlebar")) {
      var t = document.createElement("div");
      t.className = "mac-titlebar";
      t.setAttribute("data-tauri-drag-region", "deep");
      t.setAttribute("aria-hidden", "true");
      var s = document.createElement("span");
      s.textContent = (document.title || "").split(" · ")[0];
      t.appendChild(s);
      b.insertBefore(t, b.firstChild);
    }
    document.querySelectorAll(".topbar, .page-head, .card-head, .hero").forEach(function (el) {
      el.setAttribute("data-tauri-drag-region", "deep");
    });
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", ready);
  else ready();
})();"#;

/// The Mac App Store build has no see-through window: the page paints the
/// sidebar itself (`html[data-solid]` in mac.css).
#[cfg_attr(not(all(target_os = "macos", feature = "appstore")), allow(dead_code))]
pub const SOLID_SCRIPT: &str = r#"(function () {
  function mark() {
    if (document.documentElement) document.documentElement.setAttribute("data-solid", "");
  }
  mark();
  document.addEventListener("DOMContentLoaded", mark);
})();"#;

/// The sidebar material, as Finder and Mail use it.
#[cfg_attr(
    not(all(target_os = "macos", not(feature = "appstore"))),
    allow(dead_code)
)]
pub fn sidebar_effect() -> WindowEffectsConfig {
    EffectsBuilder::new()
        .effect(Effect::Sidebar)
        .state(EffectState::FollowsWindowActiveState)
        .build()
}

/// What a menu item does, by its id.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Open this page of the site.
    Open(&'static str),
    /// Run this script in the page.
    Run(&'static str),
}

pub fn action(id: &str) -> Option<Action> {
    Some(match id {
        "new" => Action::Open("/listings/new"),
        "settings" => Action::Open("/account"),
        "listings" => Action::Open("/"),
        "site" => Action::Open("/site"),
        "account" => Action::Open("/account"),
        "find" => Action::Run(
            "(function(){var q=document.getElementById('q');\
             if(q){q.focus();q.select();}else{location.href='/';}})();",
        ),
        "export" => Action::Run(
            "(function(){var a=document.querySelector('a[href^=\"/export.csv\"]');\
             if(a)a.click();else location.href='/export.csv';})();",
        ),
        "print" => Action::Run("window.print();"),
        "reload" => Action::Run("location.reload();"),
        "back" => Action::Run("history.back();"),
        "forward" => Action::Run("history.forward();"),
        _ => return None,
    })
}

/// The menu bar, in Arabic like the rest of the app.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let item = |id: &str, text: &str, keys: &str| {
        MenuItemBuilder::with_id(id, text)
            .accelerator(keys)
            .build(app)
    };
    let about = AboutMetadata {
        name: Some("AQRGO".into()),
        copyright: Some("© AQRGO".into()),
        website: Some("https://aqrgo.com".into()),
        ..Default::default()
    };

    let app_menu = SubmenuBuilder::new(app, "AQRGO")
        .about_with_text("حول AQRGO", Some(about))
        .separator()
        .item(&item("settings", "الإعدادات…", "CmdOrCtrl+,")?)
        .separator()
        .services_with_text("الخدمات")
        .separator()
        .hide_with_text("إخفاء AQRGO")
        .hide_others_with_text("إخفاء الآخرين")
        .show_all_with_text("إظهار الكل")
        .separator()
        .quit_with_text("إنهاء AQRGO")
        .build()?;

    let file = SubmenuBuilder::new(app, "ملف")
        .item(&item("new", "عرض جديد", "CmdOrCtrl+N")?)
        .separator()
        .item(&item("export", "تصدير Excel", "CmdOrCtrl+Shift+E")?)
        .item(&item("print", "طباعة…", "CmdOrCtrl+P")?)
        .separator()
        .close_window_with_text("إغلاق النافذة")
        .build()?;

    // Without these, copy and paste do nothing in the page.
    let edit = SubmenuBuilder::new(app, "تحرير")
        .undo_with_text("تراجع")
        .redo_with_text("إعادة")
        .separator()
        .cut_with_text("قص")
        .copy_with_text("نسخ")
        .paste_with_text("لصق")
        .select_all_with_text("تحديد الكل")
        .separator()
        .item(&item("find", "بحث في العروض", "CmdOrCtrl+F")?)
        .build()?;

    let view = SubmenuBuilder::new(app, "عرض")
        .item(&item("listings", "العروض", "CmdOrCtrl+1")?)
        .item(&item("site", "موقعي", "CmdOrCtrl+2")?)
        .item(&item("account", "حسابي", "CmdOrCtrl+3")?)
        .separator()
        .item(&item("back", "رجوع", "CmdOrCtrl+[")?)
        .item(&item("forward", "تقدم", "CmdOrCtrl+]")?)
        .item(&item("reload", "إعادة التحميل", "CmdOrCtrl+R")?)
        .separator()
        .fullscreen_with_text("ملء الشاشة")
        .build()?;

    let window = SubmenuBuilder::new(app, "نافذة")
        .minimize_with_text("تصغير")
        .maximize_with_text("تكبير")
        .separator()
        .bring_all_to_front_with_text("إحضار الكل إلى المقدمة")
        .build()?;

    MenuBuilder::new(app)
        .item(&app_menu)
        .item(&file)
        .item(&edit)
        .item(&view)
        .item(&window)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_items_open_pages_of_the_site() {
        assert_eq!(action("new"), Some(Action::Open("/listings/new")));
        assert_eq!(action("settings"), Some(Action::Open("/account")));
        assert_eq!(action("listings"), Some(Action::Open("/")));
        assert_eq!(action("site"), Some(Action::Open("/site")));
    }

    #[test]
    fn menu_items_that_act_on_the_page_run_a_script() {
        for id in ["find", "export", "print", "reload", "back", "forward"] {
            assert!(matches!(action(id), Some(Action::Run(_))), "{id}");
        }
        assert_eq!(action("quit"), None);
    }

    #[test]
    fn the_page_is_marked_and_the_toolbar_moves_the_window() {
        assert!(SHELL_SCRIPT.contains(r#"setAttribute("data-shell", "mac")"#));
        assert!(SHELL_SCRIPT.contains(r#"data-tauri-drag-region", "deep""#));
    }

    #[test]
    fn the_store_build_paints_its_own_sidebar() {
        assert!(SOLID_SCRIPT.contains(r#"setAttribute("data-solid", "")"#));
    }
}
