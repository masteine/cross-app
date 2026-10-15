// Navigation between pages (hash routes such as #/bottle/steam).

export function go(path) {
  const hash = `#/${path}`;
  if (location.hash === hash) {
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  } else {
    location.hash = hash;
  }
}

/** `#/bottle/<id>` with the id escaped. */
export function bottlePath(id) {
  return `bottle/${encodeURIComponent(id)}`;
}
