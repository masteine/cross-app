//! The macOS menu bar. Custom items are forwarded to the page as a `menu`
//! event carrying the item id; the page decides what to show.

use tauri::menu::{AboutMetadata, Menu, MenuBuilder, MenuEvent, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Runtime};

/// Menu item ids, also the values of the `menu` event.
pub const IDS: [&str; 9] = [
    "nav:library",
    "nav:bottles",
    "nav:engines",
    "nav:logs",
    "settings",
    "new-bottle",
    "add-program",
    "wizard",
    "reload",
];

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let item = |id: &str, text: &str, accel: Option<&str>| {
        let mut b = MenuItemBuilder::with_id(id, text);
        if let Some(a) = accel {
            b = b.accelerator(a);
        }
        b.build(app)
    };

    let about = AboutMetadata {
        name: Some("cross-app".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some("Windows games and apps on Apple Silicon, through Wine.".into()),
        license: Some("MIT".into()),
        ..Default::default()
    };
    let app_menu = SubmenuBuilder::new(app, "cross-app")
        .about(Some(about))
        .separator()
        .item(&item(
            "settings",
            "Engines and Settings…",
            Some("CmdOrCtrl+,"),
        )?)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&item("new-bottle", "New Bottle…", Some("CmdOrCtrl+N"))?)
        .item(&item(
            "add-program",
            "Add Program to Bottle…",
            Some("CmdOrCtrl+O"),
        )?)
        .separator()
        .close_window()
        .build()?;
    // Without the predefined Edit items, copy/paste does not work in text fields.
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let view = SubmenuBuilder::new(app, "View")
        .item(&item("nav:library", "Library", Some("CmdOrCtrl+1"))?)
        .item(&item("nav:bottles", "Bottles", Some("CmdOrCtrl+2"))?)
        .item(&item("nav:engines", "Engines", Some("CmdOrCtrl+3"))?)
        .item(&item("nav:logs", "Logs", Some("CmdOrCtrl+4"))?)
        .separator()
        .item(&item("reload", "Refresh", Some("CmdOrCtrl+R"))?)
        .separator()
        .fullscreen()
        .build()?;
    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .build()?;
    let help = SubmenuBuilder::new(app, "Help")
        .item(&item("wizard", "First-Run Setup…", None)?)
        .build()?;

    MenuBuilder::new(app)
        .items(&[&app_menu, &file, &edit, &view, &window, &help])
        .build()
}

pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    if IDS.contains(&id) {
        let _ = app.emit("menu", id);
    }
}
