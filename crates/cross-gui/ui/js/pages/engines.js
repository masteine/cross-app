// Engine settings: install/remove Wine engines, DXVK, default engine, GPTK import.

import { call, runTask } from "../api.js";
import { busy, bytes, fail, h, toast } from "../ui.js";

/** Import GPTK from a picked .dmg (or mounted folder). Shared with the wizard. */
export async function importGptk(fromFolder = false) {
  const path = await call("pick_file", { kind: fromFolder ? "gptk-folder" : "gptk" });
  if (!path) return null;
  const info = await runTask("import_gptk", "gptk", { path, version: null });
  toast(`Imported Game Porting Toolkit ${info.version || "(unknown version)"}.`);
  return info;
}

/** Install an engine as a task (progress in the tray and the Dock). */
export function installEngine(id, dxvk) {
  return runTask("install_engine", "engine", { id, dxvk });
}

export async function renderEngines(page) {
  const body = h("div", {});
  page.append(h("div", { class: "header" }, h("h1", {}, "Engines"), h("span", { class: "spacer" }), h("button", { onclick: () => load() }, "Refresh")), body);

  async function load() {
    let view;
    try {
      view = await call("list_engines");
    } catch (err) {
      fail(err);
      return;
    }
    body.replaceChildren(engines(view), gptk(view));
  }

  function engines(view) {
    const action = (e) => {
      const buttons = [];
      if (!e.installed) {
        const dxvk = h("input", { type: "checkbox", checked: e.supports_dxvk, disabled: !e.supports_dxvk });
        buttons.push(
          h("label", { class: "inline small" }, dxvk, "DXVK"),
          h(
            "button",
            {
              class: "primary",
              onclick: (ev) =>
                busy(ev.target, async () => {
                  try {
                    await installEngine(e.id, dxvk.checked);
                    toast(`Installed ${e.name}.`);
                  } catch (err) {
                    fail(err);
                  }
                  load();
                }),
            },
            `Install${e.size ? ` (${bytes(e.size)})` : ""}`,
          ),
        );
      } else {
        if (e.supports_dxvk && !e.dxvk_installed) {
          buttons.push(
            h(
              "button",
              {
                onclick: (ev) =>
                  busy(ev.target, async () => {
                    try {
                      await installEngine(e.id, true);
                    } catch (err) {
                      fail(err);
                    }
                    load();
                  }),
              },
              "Add DXVK",
            ),
          );
        }
        if (!e.is_default) {
          buttons.push(
            h(
              "button",
              {
                onclick: async () => {
                  try {
                    await call("set_default_engine", { id: e.id });
                  } catch (err) {
                    fail(err);
                  }
                  load();
                },
              },
              "Make default",
            ),
          );
        }
        buttons.push(
          h(
            "button",
            {
              class: "danger",
              onclick: async () => {
                const ok = await call("confirm", {
                  title: `Remove ${e.name}?`,
                  message: "Bottles that use this engine stay on disk but cannot run until you pick another engine for them.",
                  ok: "Remove",
                  danger: true,
                }).catch(fail);
                if (!ok) return;
                try {
                  await call("remove_engine", { id: e.id });
                } catch (err) {
                  fail(err);
                }
                load();
              },
            },
            "Remove",
          ),
        );
      }
      return buttons;
    };

    const rosetta = view.rosetta_installed
      ? null
      : h("p", { class: "small", style: "color: var(--danger)" }, "Rosetta 2 is not installed; current engines need it. Use First-run setup to install it.");
    return h(
      "div",
      { class: "panel" },
      h("h2", { style: "margin-top: 0" }, "Wine engines"),
      rosetta,
      h(
        "table",
        {},
        h("thead", {}, h("tr", {}, ["Engine", "Wine", "Translation", "State", ""].map((t) => h("th", {}, t)))),
        h(
          "tbody",
          {},
          view.engines.map((e) =>
            h(
              "tr",
              {},
              h("td", {}, e.name, e.is_default ? h("span", { class: "muted small" }, "  default") : null),
              h("td", {}, e.wine_version),
              h("td", {}, e.translation),
              h(
                "td",
                {},
                e.installed ? (e.dxvk_installed ? "installed + DXVK" : "installed") : "available",
                e.in_catalog ? null : h("span", { class: "muted small" }, " (not in catalog)"),
              ),
              h("td", { style: "text-align: right" }, h("div", { class: "row", style: "justify-content: flex-end" }, action(e))),
            ),
          ),
        ),
      ),
    );
  }

  function gptk(view) {
    const run = (folder) => (ev) =>
      busy(ev.target, async () => {
        try {
          await importGptk(folder);
        } catch (err) {
          fail(err);
        }
        load();
      });
    return h(
      "div",
      { class: "panel" },
      h("h2", { style: "margin-top: 0" }, "Game Porting Toolkit (D3DMetal)"),
      view.gptk
        ? h("p", { class: "selectable" }, `Imported: ${view.gptk.version || "unknown version"} — ${view.gptk.dir}`)
        : h("p", { class: "muted" }, "Not imported. Download Game_Porting_Toolkit_*.dmg yourself from developer.apple.com (Apple ID required); cross-app cannot ship it."),
      h(
        "div",
        { class: "row" },
        h("button", { onclick: run(false) }, view.gptk ? "Re-import from .dmg…" : "Import from .dmg…"),
        h("button", { onclick: run(true) }, "Import from mounted volume…"),
      ),
      h("p", { class: "muted small" }, "If the image shows a license, mount it once in Finder, accept it, then import from the mounted volume."),
    );
  }

  await load();
  return { refresh: load };
}
