// Real-time logs: pick a bottle (or the global logs) and a file, then follow it.

import { call } from "../api.js";
import { go } from "../nav.js";
import { bytes, fail, h } from "../ui.js";

const POLL_MS = 1000;
/** Keep the view bounded: drop the oldest text beyond this many characters. */
const MAX_CHARS = 2_000_000;

export async function renderLogs(page, bottle = "", file = "") {
  const bottleSelect = h("select", {});
  const fileSelect = h("select", {});
  const follow = h("input", { type: "checkbox", checked: true });
  const view = h("pre", { class: "log-view selectable" });
  const info = h("span", { class: "muted small" });

  page.append(
    h(
      "div",
      { class: "header" },
      h("h1", {}, "Logs"),
      h("span", { class: "spacer" }),
      bottleSelect,
      fileSelect,
      h("label", { class: "inline small" }, follow, "Follow"),
      h("button", { onclick: () => view.replaceChildren() }, "Clear view"),
    ),
    view,
    h("div", { class: "row", style: "margin-top: 6px" }, info),
  );

  let bottles = [];
  try {
    bottles = await call("list_bottles");
  } catch (err) {
    fail(err);
  }
  bottleSelect.append(
    h("option", { value: "" }, "cross-app (global)"),
    ...bottles.map((b) => h("option", { value: b.id, selected: b.id === bottle }, b.name)),
  );
  bottleSelect.addEventListener("change", () => go(bottleSelect.value ? `logs/${encodeURIComponent(bottleSelect.value)}` : "logs"));
  fileSelect.addEventListener("change", () =>
    go(`logs/${encodeURIComponent(bottleSelect.value)}/${encodeURIComponent(fileSelect.value)}`),
  );

  const target = bottleSelect.value || null;
  let files = [];
  try {
    files = await call("list_logs", { bottle: target });
  } catch (err) {
    fail(err);
  }
  if (!files.length) {
    fileSelect.append(h("option", { value: "" }, "no log files"));
    fileSelect.disabled = true;
    view.textContent = "No logs yet. Programs started from cross-app write their output here.";
    return null;
  }
  const chosen = files.some((f) => f.name === file) ? file : files[0].name;
  fileSelect.append(...files.map((f) => h("option", { value: f.name, selected: f.name === chosen }, `${f.name} (${bytes(f.size)})`)));

  let offset = null;
  let timer = null;
  let stopped = false;

  async function poll() {
    if (stopped) return;
    try {
      const chunk = await call("read_log", { bottle: target, name: chosen, offset });
      if (chunk.reset) view.textContent = "";
      if (chunk.text) {
        const atBottom = view.scrollTop + view.clientHeight >= view.scrollHeight - 20;
        view.append(chunk.text);
        if (view.textContent.length > MAX_CHARS) view.textContent = view.textContent.slice(-MAX_CHARS);
        if (follow.checked && atBottom) view.scrollTop = view.scrollHeight;
      }
      offset = chunk.offset;
      info.textContent = `${chosen} · ${bytes(chunk.size)}`;
    } catch (err) {
      info.textContent = String(err);
    }
    if (!stopped) timer = setTimeout(poll, POLL_MS);
  }
  await poll();
  view.scrollTop = view.scrollHeight;

  return {
    cleanup: () => {
      stopped = true;
      clearTimeout(timer);
    },
    refresh: () => {},
  };
}
