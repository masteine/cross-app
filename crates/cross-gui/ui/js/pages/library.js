// Library: Steam games and program shortcuts from all bottles, as cards.

import { call } from "../api.js";
import { bottlePath, go } from "../nav.js";
import { bytes, clear, compatBadge, fail, h, toast } from "../ui.js";

const STATE_TEXT = {
  installed: null,
  "update-required": "Update required",
  updating: "Updating",
  other: "Not ready",
};

/** Covers already turned into data URLs during this session. */
const coverCache = new Map();

async function loadCover(img, appid) {
  if (coverCache.has(appid)) {
    img.src = coverCache.get(appid);
    return;
  }
  try {
    // Cache first; the network only when the cache has nothing.
    const url = (await call("cover", { appid, fetch: false })) || (await call("cover", { appid, fetch: true }));
    if (url) {
      coverCache.set(appid, url);
      img.src = url;
    }
  } catch (err) {
    console.warn(`cover ${appid}:`, err);
  }
}

async function launch(item, button) {
  button.disabled = true;
  try {
    if (item.kind === "steam-game") {
      const r = await call("launch_game", { bottle: item.bottle, appid: item.appid });
      if (r.steam_was_running && r.has_profile) {
        toast("Steam was already running, so this game's own settings were not applied.");
      } else {
        toast(`Starting ${item.title} through Steam…`);
      }
    } else if (item.kind === "steam-client") {
      await call("start_steam", { bottle: item.bottle });
      toast("Starting Steam…");
    } else {
      await call("run_program", { bottle: item.bottle, target: item.path });
      toast(`Starting ${item.title}…`);
    }
  } catch (err) {
    fail(err);
  } finally {
    setTimeout(() => (button.disabled = false), 1500);
  }
}

function card(item) {
  const img = h("img", { alt: "", hidden: true, onload: (e) => (e.target.hidden = false) });
  if (item.appid) loadCover(img, item.appid);
  const play = h("button", { class: "primary", onclick: () => launch(item, play) }, "Play");
  const state = item.state && STATE_TEXT[item.state];
  const meta = [item.bottle_name, item.size_on_disk ? bytes(item.size_on_disk) : null, state].filter(Boolean);
  return h(
    "div",
    { class: "card" },
    h("div", { class: "cover" }, img, h("span", {}, item.title), h("span", { class: "badge-wrap" }, compatBadge(item.compat))),
    h(
      "div",
      { class: "card-body" },
      h("div", { class: "card-title", title: item.title }, item.title),
      h("div", { class: "muted small" }, meta.join(" · ")),
      h(
        "div",
        { class: "row" },
        play,
        h("button", { onclick: () => go(bottlePath(item.bottle)) }, "Bottle"),
      ),
    ),
  );
}

export async function renderLibrary(page) {
  const body = h("div", {});
  const filter = h("input", { type: "text", placeholder: "Filter", oninput: () => draw() });
  page.append(
    h(
      "div",
      { class: "header" },
      h("h1", {}, "Library"),
      h("span", { class: "spacer" }),
      filter,
      h("button", { onclick: () => load() }, "Refresh"),
    ),
    body,
  );

  let library = { items: [], warnings: [] };

  function draw() {
    clear(body);
    for (const w of library.warnings) body.append(h("p", { class: "muted small" }, `Could not read a Steam library: ${w}`));
    const q = filter.value.trim().toLowerCase();
    const items = library.items.filter((i) => !q || i.title.toLowerCase().includes(q) || i.bottle.includes(q));
    if (!library.items.length) {
      body.append(
        h(
          "div",
          { class: "empty" },
          h("p", {}, "Nothing here yet."),
          h("p", {}, "Install Steam into a bottle, or drop a Windows program (.exe) onto this window."),
          h("button", { class: "primary", onclick: () => go("bottles") }, "Go to bottles"),
        ),
      );
      return;
    }
    body.append(h("div", { class: "grid" }, items.map(card)));
  }

  async function load() {
    try {
      library = await call("library");
      draw();
    } catch (err) {
      fail(err);
    }
  }

  await load();
  return { refresh: load };
}
