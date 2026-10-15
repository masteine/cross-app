// Bottles list and the "new bottle" dialog.

import { call, runTask } from "../api.js";
import { bottlePath, go } from "../nav.js";
import { WINDOWS_VERSIONS, closeModal, fail, h, modal, windowsLabel } from "../ui.js";

const ID_PATTERN = /^[a-z0-9][a-z0-9_-]{0,63}$/;

/** Suggest a valid bottle id from a display name: "My Games" -> "my-games". */
export function idFromName(name) {
  const id = name
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^[-_]+|[-_]+$/g, "")
    .slice(0, 64);
  return id || "bottle";
}

/**
 * Ask for name, engine and Windows version, then create the bottle as a task
 * (progress in the tray). `afterCreate(bottle)` runs once it exists; without
 * it the bottle page opens.
 */
export async function newBottleDialog({ suggestedName = "", afterCreate } = {}) {
  let engines;
  try {
    engines = (await call("list_engines")).engines.filter((e) => e.installed);
  } catch (err) {
    fail(err);
    return;
  }
  if (!engines.length) {
    await modal(
      h(
        "div",
        { class: "form" },
        h("h2", {}, "No engine installed"),
        h("p", {}, "A bottle needs a Wine engine. Install one on the Engines page first."),
        h(
          "div",
          { class: "actions" },
          h("button", { onclick: () => closeModal() }, "Close"),
          h("button", { class: "primary", onclick: () => (closeModal(), go("engines")) }, "Open Engines"),
        ),
      ),
    );
    return;
  }

  const name = h("input", { type: "text", value: suggestedName, placeholder: "Games" });
  const id = h("input", { type: "text", value: suggestedName ? idFromName(suggestedName) : "" });
  let idEdited = false;
  name.addEventListener("input", () => {
    if (!idEdited) id.value = idFromName(name.value);
  });
  id.addEventListener("input", () => (idEdited = true));
  const engine = h(
    "select",
    {},
    engines.map((e) => h("option", { value: e.id, selected: e.is_default }, `${e.name} (Wine ${e.wine_version})`)),
  );
  const windows = h(
    "select",
    {},
    WINDOWS_VERSIONS.map((v) => h("option", { value: v }, windowsLabel(v))),
  );
  const error = h("p", { class: "small", style: "color: var(--danger)" });
  const create = h("button", { class: "primary", type: "submit" }, "Create");

  const form = h(
    "form",
    {
      class: "form",
      method: "dialog",
      onsubmit: async (e) => {
        e.preventDefault();
        if (!ID_PATTERN.test(id.value)) {
          error.textContent = "Id: 1-64 characters a-z, 0-9, - and _, starting with a letter or digit.";
          return;
        }
        create.disabled = true;
        closeModal();
        try {
          const result = await runTask("create_bottle", "create-bottle", {
            options: { id: id.value, name: name.value || null, engine: engine.value, windows: windows.value },
          });
          if (afterCreate) afterCreate(result);
          else go(bottlePath(result.id));
        } catch (err) {
          fail(err);
        }
      },
    },
    h("h2", {}, "New bottle"),
    h("p", { class: "muted small" }, "A bottle is a separate Windows environment (Wine prefix) with its own C: drive and registry. Creating one takes about half a minute."),
    h("label", {}, "Name", name),
    h("label", {}, "Id (folder name)", id),
    h("label", {}, "Engine", engine),
    h("label", {}, "Windows version", windows),
    error,
    h("div", { class: "actions" }, h("button", { type: "button", onclick: () => closeModal() }, "Cancel"), create),
  );
  await modal(form);
}

export async function renderBottles(page) {
  const body = h("div", {});
  page.append(
    h(
      "div",
      { class: "header" },
      h("h1", {}, "Bottles"),
      h("span", { class: "spacer" }),
      h("button", { class: "primary", onclick: () => newBottleDialog() }, "New bottle"),
    ),
    body,
  );

  async function load() {
    let bottles;
    try {
      bottles = await call("list_bottles");
    } catch (err) {
      fail(err);
      return;
    }
    body.replaceChildren();
    if (!bottles.length) {
      body.append(h("div", { class: "empty" }, h("p", {}, "No bottles yet."), h("button", { class: "primary", onclick: () => newBottleDialog() }, "Create a bottle")));
      return;
    }
    body.append(
      h(
        "div",
        { class: "panel" },
        h(
          "table",
          {},
          h("thead", {}, h("tr", {}, ["", "Name", "Id", "Engine", "Windows", "Steam", "Programs"].map((t) => h("th", {}, t)))),
          h(
            "tbody",
            {},
            bottles.map((b) =>
              h(
                "tr",
                { style: "cursor: pointer", onclick: () => go(bottlePath(b.id)) },
                h("td", {}, h("span", { class: `dot ${b.running ? "on" : ""}`, title: b.running ? "running" : "stopped" })),
                h("td", {}, b.name),
                h("td", { class: "muted" }, b.id),
                h("td", {}, b.engine || "—"),
                h("td", {}, windowsLabel(b.windows)),
                h("td", {}, b.steam_installed ? "installed" : "—"),
                h("td", {}, String(b.apps)),
              ),
            ),
          ),
        ),
      ),
    );
  }

  await load();
  return { refresh: load };
}
