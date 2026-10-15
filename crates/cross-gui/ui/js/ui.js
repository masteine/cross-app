// DOM helpers. Text always goes through textContent, never innerHTML, because
// names and paths come from bottles, Steam manifests and dropped files.

/**
 * h("button", { class: "primary", onclick: fn }, "Label", child, ...)
 * Attributes starting with "on" become listeners; `null`/`false` children are skipped.
 */
export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs || {})) {
    if (value === null || value === undefined || value === false) continue;
    if (key.startsWith("on") && typeof value === "function") {
      el.addEventListener(key.slice(2), value);
    } else if (key === "class") {
      el.className = value;
    } else if (key in el && typeof value !== "string") {
      el[key] = value;
    } else {
      el.setAttribute(key, value === true ? "" : value);
    }
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return el;
}

export function clear(el) {
  while (el.firstChild) el.removeChild(el.firstChild);
  return el;
}

export function toast(message, kind = "info") {
  const box = document.getElementById("toasts");
  const el = h("div", { class: `toast ${kind}` }, String(message));
  box.append(el);
  setTimeout(() => el.remove(), kind === "error" ? 8000 : 4000);
}

/** Show an error from a failed `call`. */
export function fail(err) {
  console.error(err);
  toast(typeof err === "string" ? err : (err && err.message) || String(err), "error");
}

export function bytes(n) {
  if (n === null || n === undefined) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

export function windowsLabel(v) {
  return { win7: "Windows 7", win8: "Windows 8", win10: "Windows 10", win11: "Windows 11" }[v] || v;
}

export const WINDOWS_VERSIONS = ["win11", "win10", "win8", "win7"];

const RATING_TEXT = { green: "Works", yellow: "Issues", red: "Broken", unknown: "Unknown" };

/** Compatibility badge (cross-compat ratings; "unknown" until Sprint 7). */
export function compatBadge(compat) {
  const rating = (compat && compat.rating) || "unknown";
  const title = (compat && compat.note) || (rating === "unknown" ? "No compatibility data yet" : RATING_TEXT[rating]);
  return h("span", { class: `badge ${rating}`, title }, RATING_TEXT[rating] || rating);
}

/** Disable a button while `work()` runs. */
export async function busy(button, work) {
  button.disabled = true;
  try {
    return await work();
  } finally {
    button.disabled = false;
  }
}

/** Open the shared <dialog> with `content`; resolves when it closes. */
export function modal(content, { onClose } = {}) {
  const dialog = document.getElementById("modal");
  clear(dialog).append(content);
  const closed = new Promise((resolve) => {
    dialog.addEventListener(
      "close",
      () => {
        if (onClose) onClose();
        resolve(dialog.returnValue);
      },
      { once: true },
    );
  });
  if (!dialog.open) dialog.showModal();
  return closed;
}

export function closeModal(value = "") {
  const dialog = document.getElementById("modal");
  if (dialog.open) dialog.close(value);
}
