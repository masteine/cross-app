// Files dropped on the window (or picked with File > Add Program): add them as
// shortcuts (Bottle.apps in bottle.toml) to a bottle of the user's choice.

import { call } from "../api.js";
import { bottlePath, go } from "../nav.js";
import { closeModal, fail, h, modal, toast } from "../ui.js";
import { newBottleDialog } from "./bottles.js";

const PROGRAM = /\.(exe|msi|lnk|bat|cmd)$/i;

function baseName(path) {
  return path.split("/").pop().replace(/\.[^.]+$/, "");
}

export async function addProgramsDialog(paths, preferredBottle) {
  const programs = paths.filter((p) => PROGRAM.test(p));
  const ignored = paths.length - programs.length;
  if (!programs.length) {
    toast("Only Windows programs (.exe, .msi, .lnk, .bat, .cmd) can be added.", "error");
    return;
  }

  let bottles;
  try {
    bottles = await call("list_bottles");
  } catch (err) {
    fail(err);
    return;
  }
  if (!bottles.length) {
    await modal(
      h(
        "div",
        { class: "form" },
        h("h2", {}, "Add to a bottle"),
        h("p", {}, "There is no bottle yet. Create one, and the program will be added to it."),
        h(
          "div",
          { class: "actions" },
          h("button", { onclick: () => closeModal() }, "Cancel"),
          h(
            "button",
            {
              class: "primary",
              onclick: () => {
                closeModal();
                newBottleDialog({
                  suggestedName: "Apps",
                  afterCreate: (bottle) => addAll(bottle.id, programs.map((p) => [p, null]), false),
                });
              },
            },
            "New bottle…",
          ),
        ),
      ),
    );
    return;
  }

  const select = h(
    "select",
    {},
    bottles.map((b) => h("option", { value: b.id, selected: b.id === preferredBottle }, b.name)),
  );
  const names = programs.map((p) => [p, h("input", { type: "text", value: baseName(p) })]);
  const submit = (run) => () => {
    closeModal();
    addAll(select.value, names.map(([p, input]) => [p, input.value]), run);
  };
  await modal(
    h(
      "div",
      { class: "form" },
      h("h2", {}, programs.length === 1 ? "Add program" : `Add ${programs.length} programs`),
      h("label", {}, "Bottle", select),
      names.map(([p, input]) => h("label", {}, h("span", { class: "muted small selectable" }, p), input)),
      ignored ? h("p", { class: "muted small" }, `${ignored} other file(s) ignored.`) : null,
      h(
        "div",
        { class: "actions" },
        h("button", { onclick: () => closeModal() }, "Cancel"),
        programs.length === 1 ? h("button", { onclick: submit(true) }, "Add and run") : null,
        h("button", { class: "primary", onclick: submit(false) }, "Add"),
      ),
    ),
  );
}

async function addAll(bottle, entries, run) {
  let added = 0;
  for (const [path, name] of entries) {
    try {
      const app = await call("add_app", { bottle, path, name: name || null });
      added += 1;
      if (run) await call("run_program", { bottle, target: app.path });
    } catch (err) {
      fail(err);
    }
  }
  if (added) {
    toast(`Added ${added} program(s) to ${bottle}.`);
    go(bottlePath(bottle));
  }
}
