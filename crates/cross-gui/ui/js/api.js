// Thin layer over the Tauri globals (`app.withGlobalTauri` in tauri.conf.json).
// Every backend call goes through `call`, every backend event through `on`.

const tauri = window.__TAURI__;

export const insideTauri = Boolean(tauri && tauri.core);

/** Invoke a Rust command; rejects with the error message as a string. */
export function call(command, args = {}) {
  if (!insideTauri) {
    return Promise.reject("The cross-app backend is not available (open this page in the app window).");
  }
  return tauri.core.invoke(command, args);
}

/** Listen to a backend event; resolves to an unlisten function. */
export function on(event, handler) {
  if (!insideTauri) return Promise.resolve(() => {});
  return tauri.event.listen(event, (e) => handler(e.payload));
}

let taskCounter = 0;

/** A fresh id for a long operation; the backend echoes it in `task://*` events. */
export function newTaskId(kind) {
  taskCounter += 1;
  return `${kind}-${Date.now()}-${taskCounter}`;
}

/** Run a long command that reports progress: `args.task` is filled in. */
export function runTask(command, kind, args = {}) {
  return call(command, { ...args, task: newTaskId(kind) });
}
