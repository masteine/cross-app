//! Long operations (engine and Steam installs, dependencies, bottle creation):
//! run off the main thread, forward `cross_engine::Progress` as events, mirror
//! the progress on the Dock icon and notify when the window is in the background.
//!
//! Events, all with a `task` id chosen by the page:
//! - `task://started`  `{ task, title }`
//! - `task://progress` [`ProgressEvent`]
//! - `task://finished` `{ task, title, ok, error }`

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;

use cross_engine::Progress;
use serde::Serialize;
use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::api::progress::{ProgressEvent, Throttle};
use crate::{Error, Result};

pub const MAIN_WINDOW: &str = "main";

/// Running tasks and their last known percentage, for the Dock indicator.
#[derive(Debug, Default)]
pub struct TaskBoard {
    running: Mutex<BTreeMap<String, Option<u8>>>,
}

/// What the Dock icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dock {
    Hidden,
    Indeterminate,
    Percent(u64),
}

impl Dock {
    fn state(self) -> ProgressBarState {
        let (status, progress) = match self {
            Dock::Hidden => (ProgressBarStatus::None, None),
            Dock::Indeterminate => (ProgressBarStatus::Indeterminate, None),
            Dock::Percent(p) => (ProgressBarStatus::Normal, Some(p)),
        };
        ProgressBarState {
            status: Some(status),
            progress,
        }
    }
}

impl TaskBoard {
    /// Hidden when idle, the mean of the known percentages otherwise
    /// (indeterminate while nothing is measurable yet).
    fn dock(&self) -> Dock {
        let running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        let known: Vec<u64> = running.values().flatten().map(|p| u64::from(*p)).collect();
        if running.is_empty() {
            Dock::Hidden
        } else if known.is_empty() {
            Dock::Indeterminate
        } else {
            Dock::Percent(known.iter().sum::<u64>() / known.len() as u64)
        }
    }

    fn set(&self, task: &str, percent: Option<u8>) {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(task.to_owned(), percent);
    }

    fn remove(&self, task: &str) {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(task);
    }
}

#[derive(Clone, Serialize)]
struct Started<'a> {
    task: &'a str,
    title: &'a str,
}

#[derive(Clone, Serialize)]
struct Finished<'a> {
    task: &'a str,
    title: &'a str,
    ok: bool,
    error: Option<String>,
}

/// On macOS the progress bar belongs to the Dock tile (app-wide), so any
/// window will do. Errors only mean there is no window to show it on.
fn update_dock(app: &AppHandle) {
    let state = app.state::<TaskBoard>().dock().state();
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        if let Err(e) = window.set_progress_bar(state) {
            tracing::debug!(error = %e, "could not update the Dock progress");
        }
    }
}

fn window_focused(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN_WINDOW)
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false)
}

/// Tell the user about a finished task when they are looking elsewhere.
pub fn notify(app: &AppHandle, title: &str, body: &str) {
    if window_focused(app) {
        return;
    }
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(error = %e, "could not show a notification");
    }
}

/// Run `work` on a blocking thread as task `task`, with progress events.
pub async fn run<T, F>(app: AppHandle, task: String, title: String, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut dyn FnMut(Progress<'_>)) -> Result<T> + Send + 'static,
{
    let _ = app.emit(
        "task://started",
        Started {
            task: &task,
            title: &title,
        },
    );
    app.state::<TaskBoard>().set(&task, None);
    update_dock(&app);

    let worker_app = app.clone();
    let worker_task = task.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut throttle = Throttle::default();
        let mut on_progress = |p: Progress<'_>| {
            let event = ProgressEvent::new(&worker_task, p);
            if throttle.admit(&event, Instant::now()) {
                worker_app
                    .state::<TaskBoard>()
                    .set(&worker_task, event.percent);
                update_dock(&worker_app);
                let _ = worker_app.emit("task://progress", event);
            }
        };
        work(&mut on_progress)
    })
    .await
    .map_err(|e| Error::Internal(format!("{title}: {e}")))
    .and_then(|r| r);

    app.state::<TaskBoard>().remove(&task);
    update_dock(&app);
    let error = result.as_ref().err().map(ToString::to_string);
    let _ = app.emit(
        "task://finished",
        Finished {
            task: &task,
            title: &title,
            ok: error.is_none(),
            error: error.clone(),
        },
    );
    match &error {
        None => notify(&app, &title, "Done."),
        Some(message) => notify(&app, &title, &format!("Failed: {message}")),
    }
    result
}

/// Run a short blocking call off the main thread (no events).
pub async fn blocking<T, F>(work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_follows_running_tasks() {
        let board = TaskBoard::default();
        assert_eq!(board.dock(), Dock::Hidden);
        board.set("a", None);
        assert_eq!(board.dock(), Dock::Indeterminate);
        board.set("a", Some(20));
        board.set("b", Some(60));
        assert_eq!(board.dock(), Dock::Percent(40));
        board.set("c", None);
        assert_eq!(
            board.dock(),
            Dock::Percent(40),
            "unknown sizes do not count"
        );
        for t in ["a", "b", "c"] {
            board.remove(t);
        }
        assert_eq!(board.dock(), Dock::Hidden);
        assert_eq!(Dock::Percent(5).state().progress, Some(5));
    }
}
