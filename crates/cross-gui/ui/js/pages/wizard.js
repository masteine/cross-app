// First-run wizard: system check (Rosetta), engine, optional GPTK, first bottle.
// Runs inside the modal <dialog>, which sits above the task tray, so it shows
// the progress of its own tasks itself.

import { call, newTaskId, on } from "../api.js";
import { bottlePath, go } from "../nav.js";
import { idFromName } from "./bottles.js";
import { WINDOWS_VERSIONS, bytes, closeModal, fail, h, modal, windowsLabel } from "../ui.js";

const STEPS = ["System", "Engine", "D3DMetal", "Bottle", "Done"];

/** Run a task command and mirror its progress into `bar`/`text`. */
async function withProgress(command, args, bar, text) {
  const task = newTaskId(command);
  const unlisten = await on("task://progress", (p) => {
    if (p.task !== task) return;
    if (p.percent !== null && p.percent !== undefined) {
      bar.max = 100;
      bar.value = p.percent;
    } else {
      bar.removeAttribute("value");
    }
    text.textContent = `${p.label}: ${p.stage.replace("-", " ")}${p.percent != null ? ` ${p.percent}%` : ""}`;
  });
  bar.hidden = false;
  bar.removeAttribute("value");
  try {
    return await call(command, { ...args, task });
  } finally {
    unlisten();
  }
}

export async function openWizard(initialStatus) {
  const body = h("div", {});
  const stepBar = h("div", { class: "steps" }, STEPS.map(() => h("span", {})));
  let status = initialStatus;
  let step = 0;

  const markSteps = () => [...stepBar.children].forEach((s, i) => s.classList.toggle("done", i <= step));
  const show = (...content) => {
    markSteps();
    body.replaceChildren(h("h2", { style: "margin-top: 0" }, `Setup · ${STEPS[step]}`), ...content.filter(Boolean));
  };
  const next = () => {
    step += 1;
    render();
  };
  const skipAll = () => {
    try {
      localStorage.setItem("wizard-skipped", "1");
    } catch (_) {
      // Private storage unavailable: the wizard simply shows again next time.
    }
    closeModal();
  };

  async function refreshStatus() {
    status = await call("system_status");
    return status;
  }

  // 1. System check, with the Rosetta install behind a confirmation.
  async function systemStep() {
    await refreshStatus();
    const rosetta = status.system.rosetta_installed;
    const checks = status.report.checks.map((c) =>
      h("div", { class: "check" }, h("span", { class: `status ${c.status}` }, c.status), h("div", {}, h("strong", {}, c.name), h("div", { class: "muted small" }, c.detail))),
    );
    const install = h("button", { class: "primary" }, "Install Rosetta 2");
    const note = h("p", { class: "muted small" });
    install.addEventListener("click", async () => {
      const ok = await call("confirm", {
        title: "Install Rosetta 2?",
        message: `cross-app will run:\n\n${status.rosetta_command}\n\nThis accepts Apple's Rosetta license. macOS may ask for an administrator password.`,
        ok: "Install",
      }).catch(fail);
      if (!ok) return;
      install.disabled = true;
      note.textContent = "Installing Rosetta 2…";
      try {
        await call("install_rosetta", { task: newTaskId("rosetta") });
        systemStep();
      } catch (err) {
        install.disabled = false;
        note.textContent = `${err} — you can also run it in Terminal: sudo ${status.rosetta_command}`;
      }
    });
    show(
      h("div", { class: "panel" }, checks),
      rosetta ? null : h("div", { class: "row", style: "margin-top: 12px" }, install, note),
      h(
        "div",
        { class: "actions" },
        h("button", { onclick: skipAll }, "Skip setup"),
        h("button", { class: "primary", onclick: next }, rosetta ? "Next" : "Continue without Rosetta"),
      ),
    );
  }

  // 2. Engine: install one from the catalog (or keep the installed one).
  async function engineStep() {
    const view = await call("list_engines");
    const installed = view.engines.filter((e) => e.installed);
    const available = view.engines.filter((e) => !e.installed && e.in_catalog);
    const bar = h("progress", { hidden: true });
    const text = h("div", { class: "muted small" });

    if (installed.length) {
      show(
        h("p", {}, `Installed: ${installed.map((e) => e.name).join(", ")}.`),
        h("p", { class: "muted small" }, "More engines can be added later on the Engines page."),
        h("div", { class: "actions" }, h("button", { class: "primary", onclick: next }, "Next")),
      );
      return;
    }
    const choice = h(
      "select",
      {},
      available.map((e) => h("option", { value: e.id }, `${e.name} (Wine ${e.wine_version}${e.size ? `, ${bytes(e.size)}` : ""})`)),
    );
    const dxvk = h("input", { type: "checkbox", checked: true });
    const install = h("button", { class: "primary" }, "Install");
    install.addEventListener("click", async () => {
      install.disabled = true;
      choice.disabled = true;
      try {
        const engine = view.engines.find((e) => e.id === choice.value);
        await withProgress("install_engine", { id: choice.value, dxvk: dxvk.checked && engine.supports_dxvk }, bar, text);
        next();
      } catch (err) {
        text.textContent = String(err);
        install.disabled = false;
        choice.disabled = false;
      }
    });
    show(
      h("p", {}, "Wine runs Windows programs. Pick an engine to download (verified by SHA-256 and cached)."),
      h("div", { class: "form" }, h("label", {}, "Engine", choice), h("label", { class: "inline" }, dxvk, "Also install DXVK + MoltenVK (DirectX 9-11 through Vulkan)")),
      bar,
      text,
      h("div", { class: "actions" }, h("button", { onclick: next }, "Skip"), install),
    );
  }

  // 3. GPTK (optional): D3DMetal for DirectX 12 games.
  async function gptkStep() {
    const view = await call("list_engines");
    const bar = h("progress", { hidden: true });
    const text = h("div", { class: "muted small" });
    const pick = (folder) => async () => {
      try {
        const path = await call("pick_file", { kind: folder ? "gptk-folder" : "gptk" });
        if (!path) return;
        await withProgress("import_gptk", { path, version: null }, bar, text);
        next();
      } catch (err) {
        text.textContent = String(err);
      }
    };
    show(
      view.gptk
        ? h("p", {}, `Already imported: Game Porting Toolkit ${view.gptk.version || ""}.`)
        : h("p", {}, "DirectX 12 games need Apple's D3DMetal from the Game Porting Toolkit. Apple does not allow redistributing it: download Game_Porting_Toolkit_*.dmg from developer.apple.com, then import it here. This step is optional."),
      h("div", { class: "row" }, h("button", { onclick: pick(false) }, "Import .dmg…"), h("button", { onclick: pick(true) }, "Import mounted volume…")),
      bar,
      text,
      h("div", { class: "actions" }, h("button", { class: "primary", onclick: next }, view.gptk ? "Next" : "Skip")),
    );
  }

  // 4. First bottle.
  async function bottleStep() {
    await refreshStatus();
    if (!status.engines_installed.length) {
      show(
        h("p", {}, "A bottle needs an engine, and none is installed yet."),
        h("div", { class: "actions" }, h("button", { onclick: () => ((step = 1), render()) }, "Back to engines"), h("button", { onclick: next }, "Skip")),
      );
      return;
    }
    const name = h("input", { type: "text", value: "Games" });
    const windows = h("select", {}, WINDOWS_VERSIONS.map((v) => h("option", { value: v }, windowsLabel(v))));
    const bar = h("progress", { hidden: true });
    const text = h("div", { class: "muted small" });
    const create = h("button", { class: "primary" }, "Create bottle");
    create.addEventListener("click", async () => {
      create.disabled = true;
      text.textContent = "Creating the Wine prefix (about half a minute)…";
      try {
        const bottle = await withProgress("create_bottle", { options: { id: idFromName(name.value), name: name.value, engine: null, windows: windows.value } }, bar, text);
        createdId = bottle.id;
        next();
      } catch (err) {
        text.textContent = String(err);
        create.disabled = false;
      }
    });
    show(
      h("p", {}, "A bottle is a separate Windows environment. Steam and games usually share one bottle."),
      h("div", { class: "form" }, h("label", {}, "Name", name), h("label", {}, "Windows version", windows)),
      bar,
      text,
      h("div", { class: "actions" }, h("button", { onclick: next }, "Skip"), create),
    );
  }

  let createdId = null;

  function doneStep() {
    show(
      h("p", {}, "Setup is done."),
      createdId
        ? h("p", {}, "Next: open the bottle to install Steam or add a program. You can also drop an .exe onto the window at any time.")
        : h("p", {}, "You can create a bottle any time on the Bottles page."),
      h(
        "div",
        { class: "actions" },
        h("button", { onclick: () => closeModal() }, "Close"),
        createdId ? h("button", { class: "primary", onclick: () => (closeModal(), go(bottlePath(createdId))) }, "Open bottle") : null,
      ),
    );
  }

  async function render() {
    try {
      await [systemStep, engineStep, gptkStep, bottleStep, doneStep][step]();
    } catch (err) {
      fail(err);
    }
  }

  const done = modal(h("div", {}, stepBar, body));
  await render();
  await done;
}
