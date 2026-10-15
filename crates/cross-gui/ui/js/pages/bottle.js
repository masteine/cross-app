// One bottle: engine, Windows version, programs, Steam, dependencies,
// processes, start/stop and Wine tools.

import { call, runTask } from "../api.js";
import { go } from "../nav.js";
import { WINDOWS_VERSIONS, busy, fail, h, toast, windowsLabel } from "../ui.js";

const TOOLS = [
  ["winecfg", "Wine settings"],
  ["regedit", "Registry editor"],
  ["taskmgr", "Task manager"],
  ["cmd", "Command prompt"],
];

export async function renderBottle(page, id) {
  const root = h("div", {});
  page.append(root);

  async function load() {
    let b;
    try {
      b = await call("bottle_detail", { id });
    } catch (err) {
      fail(err);
      root.replaceChildren(h("div", { class: "empty" }, "This bottle could not be opened."));
      return;
    }
    root.replaceChildren(header(b), overview(b), programs(b), steam(b), deps(b), processes(b), tools(b), danger(b));
  }

  // --- header: name, state, start/stop ---------------------------------------

  function header(b) {
    const stop = (force) => async (e) => {
      await busy(e.target, async () => {
        try {
          const r = await call("stop_bottle", { id, force });
          if (!r.was_running) toast(`${b.name} was not running.`);
          else if (r.remaining === 0) toast(`Stopped ${b.name}.`);
          else toast(`${b.name} is still running; try Force stop.`, "error");
        } catch (err) {
          fail(err);
        }
      });
      load();
    };
    const runOnce = async () => {
      try {
        const path = await call("pick_file", { kind: "program" });
        if (!path) return;
        const r = await call("run_program", { bottle: id, target: path });
        toast(`Started; output goes to ${r.log}`);
        setTimeout(load, 1500);
      } catch (err) {
        fail(err);
      }
    };
    return h(
      "div",
      { class: "header" },
      h("span", { class: `dot ${b.running ? "on" : ""}` }),
      h("h1", {}, b.name),
      h("span", { class: "muted" }, b.running ? "running" : "stopped"),
      h("span", { class: "spacer" }),
      h("button", { onclick: runOnce }, "Run program…"),
      h("button", { onclick: stop(false), disabled: !b.running }, "Stop"),
      h("button", { class: "danger", onclick: stop(true), disabled: !b.running }, "Force stop"),
      h("button", { onclick: () => go(`logs/${encodeURIComponent(id)}`) }, "Logs"),
    );
  }

  // --- engine and Windows version -------------------------------------------

  function overview(b) {
    const engine = h(
      "select",
      {
        onchange: async (e) => {
          try {
            await call("set_bottle_engine", { id, engine: e.target.value });
            toast(`Engine set to ${e.target.value}.`);
          } catch (err) {
            fail(err);
          }
          load();
        },
      },
      (b.engines.includes(b.engine) ? b.engines : [b.engine, ...b.engines].filter(Boolean)).map((e) =>
        h("option", { value: e, selected: e === b.engine }, e),
      ),
    );
    const windows = h(
      "select",
      {
        onchange: async (e) => {
          const select = e.target;
          select.disabled = true;
          try {
            await call("set_windows_version", { id, windows: select.value });
            toast(`Windows version set to ${windowsLabel(select.value)}.`);
          } catch (err) {
            fail(err);
          }
          load();
        },
      },
      WINDOWS_VERSIONS.map((v) => h("option", { value: v, selected: v === b.windows }, windowsLabel(v))),
    );
    return h(
      "div",
      { class: "panel" },
      h(
        "div",
        { class: "row" },
        h("label", {}, "Engine", engine),
        h("label", {}, "Windows version", windows),
      ),
      h("p", { class: "muted small selectable" }, `Id: ${b.id} · Prefix: ${b.prefix}`),
    );
  }

  // --- program shortcuts -----------------------------------------------------

  function programs(b) {
    const list = b.apps.filter((a) => a.kind !== "steam-profile");
    const add = async () => {
      try {
        const path = await call("pick_file", { kind: "program" });
        if (!path) return;
        await call("add_app", { bottle: id, path, name: null });
        load();
      } catch (err) {
        fail(err);
      }
    };
    const run = (app) => async (e) => {
      await busy(e.target, async () => {
        try {
          if (app.kind === "steam-client") {
            await call("start_steam", { bottle: id });
            toast("Starting Steam…");
          } else {
            await call("run_program", { bottle: id, target: app.path });
            toast(`Starting ${app.name}…`);
          }
        } catch (err) {
          fail(err);
        }
      });
      setTimeout(load, 1500);
    };
    const remove = (app) => async () => {
      try {
        await call("remove_app", { bottle: id, path: app.path });
        load();
      } catch (err) {
        fail(err);
      }
    };
    return h(
      "div",
      { class: "panel" },
      h("div", { class: "row" }, h("h2", { style: "margin: 0" }, "Programs"), h("span", { class: "spacer" }), h("button", { onclick: add }, "Add program…")),
      list.length
        ? h(
            "table",
            {},
            h(
              "tbody",
              {},
              list.map((a) =>
                h(
                  "tr",
                  {},
                  h("td", {}, a.name),
                  h("td", { class: "muted small selectable" }, a.path),
                  h(
                    "td",
                    { style: "text-align: right; white-space: nowrap" },
                    h("button", { class: "primary", onclick: run(a) }, "Run"),
                    " ",
                    a.kind === "program" ? h("button", { onclick: remove(a) }, "Remove") : null,
                  ),
                ),
              ),
            ),
          )
        : h("p", { class: "muted" }, "No programs yet. Drop an .exe onto the window or use Add program."),
    );
  }

  // --- Steam -----------------------------------------------------------------

  function steam(b) {
    const install = async (e) => {
      await busy(e.target, async () => {
        try {
          await runTask("install_steam", "steam", { bottle: id });
        } catch (err) {
          fail(err);
        }
      });
      load();
    };
    const start = async () => {
      try {
        await call("start_steam", { bottle: id });
        toast("Starting Steam… Sign in inside the Steam window.");
        setTimeout(load, 3000);
      } catch (err) {
        fail(err);
      }
    };
    return h(
      "div",
      { class: "panel" },
      h("h2", { style: "margin-top: 0" }, "Steam"),
      b.steam_installed
        ? h("div", { class: "row" }, h("button", { class: "primary", onclick: start }, "Start Steam"), h("button", { onclick: () => go("library") }, "Show games"))
        : h(
            "div",
            {},
            h("p", { class: "muted" }, "Installs the Windows Steam client (pinned SteamSetup.exe, silent install, compatibility patches)."),
            h("button", { onclick: install }, "Install Steam"),
          ),
    );
  }

  // --- dependencies ------------------------------------------------------------

  function deps(b) {
    const boxes = b.deps.map((d) => ({ d, box: h("input", { type: "checkbox", disabled: d.installed }) }));
    const force = h("input", { type: "checkbox" });
    const install = async (e) => {
      const verbs = boxes.filter((x) => x.box.checked).map((x) => x.d.verb);
      if (!verbs.length) {
        toast("Select at least one dependency.");
        return;
      }
      await busy(e.target, async () => {
        try {
          const outcomes = await runTask("install_deps", "deps", { id, verbs, force: force.checked, winetricks: false });
          toast(outcomes.map((o) => `${o.verb}: ${o.status.replace(/-/g, " ")}`).join(", "));
        } catch (err) {
          fail(err);
        }
      });
      load();
    };
    return h(
      "div",
      { class: "panel" },
      h("h2", { style: "margin-top: 0" }, "Dependencies"),
      h(
        "table",
        {},
        h(
          "tbody",
          {},
          boxes.map(({ d, box }) =>
            h(
              "tr",
              {},
              h("td", { style: "width: 28px" }, box),
              h("td", {}, h("code", {}, d.verb)),
              h("td", {}, d.title || (d.source === "winetricks" ? "installed with winetricks" : "")),
              h("td", { class: "muted" }, d.installed ? "installed" : ""),
            ),
          ),
        ),
      ),
      h(
        "div",
        { class: "row", style: "margin-top: 10px" },
        h("button", { onclick: install }, "Install selected"),
        h("label", { class: "inline small" }, force, "Reinstall"),
        h("span", { class: "spacer" }),
        h("span", { class: "muted small" }, b.winetricks ? `Other verbs: winetricks at ${b.winetricks} (CLI only for now)` : ""),
      ),
    );
  }

  // --- processes -----------------------------------------------------------------

  function processes(b) {
    const body = h("div", {}, h("p", { class: "muted" }, b.running ? "Loading…" : "Not running."));
    const refresh = async () => {
      if (!b.running) return;
      try {
        const list = await call("bottle_processes", { id });
        body.replaceChildren(
          list.length
            ? h(
                "table",
                {},
                h("thead", {}, h("tr", {}, h("th", {}, "PID"), h("th", {}, "Name"))),
                h("tbody", {}, list.map((p) => h("tr", {}, h("td", {}, String(p.pid)), h("td", {}, p.name)))),
              )
            : h("p", { class: "muted" }, "No Windows processes."),
        );
      } catch (err) {
        body.replaceChildren(h("p", { class: "muted" }, String(err)));
      }
    };
    refresh();
    return h(
      "div",
      { class: "panel" },
      h("div", { class: "row" }, h("h2", { style: "margin: 0" }, "Processes"), h("span", { class: "spacer" }), h("button", { onclick: refresh, disabled: !b.running }, "Refresh")),
      body,
    );
  }

  // --- Wine tools ------------------------------------------------------------------

  function tools() {
    return h(
      "div",
      { class: "panel" },
      h("h2", { style: "margin-top: 0" }, "Tools"),
      h(
        "div",
        { class: "row" },
        TOOLS.map(([tool, label]) =>
          h(
            "button",
            {
              onclick: async () => {
                try {
                  await call("run_tool", { bottle: id, tool });
                  setTimeout(load, 1500);
                } catch (err) {
                  fail(err);
                }
              },
            },
            label,
          ),
        ),
      ),
    );
  }

  function danger(b) {
    const remove = async () => {
      const ok = await call("confirm", {
        title: `Delete ${b.name}?`,
        message: `This deletes the bottle "${b.id}" and everything installed in it. This cannot be undone.`,
        ok: "Delete",
        danger: true,
      }).catch(fail);
      if (!ok) return;
      try {
        await call("delete_bottle", { id });
        toast(`Deleted ${b.name}.`);
        go("bottles");
      } catch (err) {
        fail(err);
      }
    };
    return h(
      "div",
      { class: "panel" },
      h("div", { class: "row" }, h("span", { class: "muted" }, "Delete this bottle and everything in it"), h("span", { class: "spacer" }), h("button", { class: "danger", onclick: remove }, "Delete bottle…")),
    );
  }

  // Re-rendered after tasks finish and programs exit (see app.js), not on a
  // timer, so a half-filled form is not wiped.
  await load();
  return { refresh: load };
}
