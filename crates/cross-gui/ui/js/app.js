// Entry point: hash router, menu and drag-and-drop handling, the task tray.

import { call, insideTauri, on } from "./api.js";
import { clear, fail, h, toast } from "./ui.js";
import { renderLibrary } from "./pages/library.js";
import { renderBottles, newBottleDialog } from "./pages/bottles.js";
import { renderBottle } from "./pages/bottle.js";
import { renderEngines } from "./pages/engines.js";
import { renderLogs } from "./pages/logs.js";
import { openWizard } from "./pages/wizard.js";
import { addProgramsDialog } from "./pages/drop.js";
import { go } from "./nav.js";

const page = document.getElementById("page");
let current = null; // { cleanup } of the page on screen

const routes = {
  library: () => renderLibrary(page),
  bottles: () => renderBottles(page),
  bottle: (id) => renderBottle(page, id),
  engines: () => renderEngines(page),
  logs: (bottle, file) => renderLogs(page, bottle, file),
};

async function route() {
  const [name, ...params] = location.hash.replace(/^#\/?/, "").split("/").map(decodeURIComponent);
  const render = routes[name] || routes.library;
  if (current && current.cleanup) current.cleanup();
  current = null;
  for (const a of document.querySelectorAll("[data-nav]")) {
    const nav = a.dataset.nav;
    a.classList.toggle("active", nav === name || (nav === "bottles" && name === "bottle"));
  }
  clear(page);
  try {
    current = (await render(...params)) || null;
  } catch (err) {
    fail(err);
  }
}

/** Re-render the page on screen (after a task finished, a program exited, ...). */
function refresh() {
  if (current && current.refresh) {
    current.refresh();
  } else {
    route();
  }
}

// --- long-running tasks: a card per task with live progress -------------------

const tray = document.getElementById("tasks");
const taskCards = new Map();

const STAGE_TEXT = {
  downloading: "Downloading",
  "cache-hit": "Using cached download",
  verifying: "Verifying",
  extracting: "Extracting",
};

function taskCard(task, title) {
  let card = taskCards.get(task);
  if (!card) {
    const bar = h("progress", {});
    const detail = h("div", { class: "muted small" }, "Starting…");
    const el = h("div", { class: "task" }, h("div", { class: "task-title" }, title || "Working…"), bar, detail);
    card = { el, bar, detail };
    taskCards.set(task, card);
    tray.append(el);
  }
  return card;
}

function wireTasks() {
  on("task://started", ({ task, title }) => taskCard(task, title));
  on("task://progress", (p) => {
    const card = taskCard(p.task);
    const stage = STAGE_TEXT[p.stage] || p.stage;
    if (p.percent !== null && p.percent !== undefined) {
      card.bar.max = 100;
      card.bar.value = p.percent;
      card.detail.textContent = `${p.label}: ${stage} ${p.percent}%`;
    } else {
      card.bar.removeAttribute("value"); // indeterminate
      card.detail.textContent = `${p.label}: ${stage}`;
    }
  });
  on("task://finished", ({ task, title, ok, error }) => {
    const card = taskCard(task, title);
    if (ok) {
      card.bar.max = 100;
      card.bar.value = 100;
      card.detail.textContent = "Done";
      setTimeout(() => card.el.remove(), 2500);
    } else {
      card.el.classList.add("failed");
      card.detail.textContent = error || "Failed";
      card.el.append(h("button", { class: "link", onclick: () => card.el.remove() }, "Dismiss"));
    }
    taskCards.delete(task);
    refresh();
  });
  on("process://exited", ({ label, code, error }) => {
    if (error) toast(`${label}: ${error}`, "error");
    else if (code !== null && code !== 0) toast(`${label} exited with code ${code}`);
    refresh();
  });
}

// --- drag and drop: .exe onto the window -> shortcut in a bottle ---------------

function wireDragDrop() {
  const overlay = document.getElementById("drop-overlay");
  on("tauri://drag-enter", () => (overlay.hidden = false));
  on("tauri://drag-leave", () => (overlay.hidden = true));
  on("tauri://drag-drop", ({ paths }) => {
    overlay.hidden = true;
    if (paths && paths.length) addProgramsDialog(paths, currentBottle());
  });
}

/** The bottle on screen, so a drop on its page targets it by default. */
function currentBottle() {
  const [name, id] = location.hash.replace(/^#\/?/, "").split("/");
  return name === "bottle" ? decodeURIComponent(id || "") : null;
}

// --- menu bar ------------------------------------------------------------------

async function onMenu(id) {
  switch (id) {
    case "nav:library":
    case "nav:bottles":
    case "nav:engines":
    case "nav:logs":
      go(id.slice(4));
      break;
    case "settings":
      go("engines");
      break;
    case "new-bottle":
      newBottleDialog();
      break;
    case "add-program": {
      try {
        const path = await call("pick_file", { kind: "program" });
        if (path) addProgramsDialog([path], currentBottle());
      } catch (err) {
        fail(err);
      }
      break;
    }
    case "wizard":
      openWizard();
      break;
    case "reload":
      route();
      break;
    default:
      break;
  }
}

async function start() {
  if (!insideTauri) {
    page.append(
      h("div", { class: "empty" }, "This page is the cross-app GUI. Start it with the cross-app app, not a browser."),
    );
    return;
  }
  wireTasks();
  wireDragDrop();
  on("menu", onMenu);
  document.getElementById("open-wizard").addEventListener("click", () => openWizard());
  window.addEventListener("hashchange", route);
  if (!location.hash) location.hash = "#/library";
  await route();
  try {
    const status = await call("system_status");
    let skipped = false;
    try {
      skipped = localStorage.getItem("wizard-skipped") === "1";
    } catch (_) {
      skipped = false;
    }
    if (status.needs_setup && !skipped) openWizard(status);
  } catch (err) {
    fail(err);
  }
}

start();
