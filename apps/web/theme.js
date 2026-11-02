/**
 * Applies the colour theme before the page paints.
 *
 * Loaded synchronously in <head> so a visitor who chose light mode never sees
 * a dark flash. The saved choice wins; otherwise the system preference does.
 * Storage can be unavailable (private windows, blocked site data), in which
 * case the system preference is used and nothing is remembered.
 */
(function applyTheme() {
  let saved = null;
  try {
    saved = localStorage.getItem("hoista-theme");
  } catch {
    // Storage blocked: fall back to the system preference below.
  }
  const prefersLight = window.matchMedia("(prefers-color-scheme: light)").matches;
  const theme = saved === "light" || saved === "dark" ? saved : prefersLight ? "light" : "dark";
  document.documentElement.dataset.theme = theme;
})();
