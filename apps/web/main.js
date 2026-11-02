/**
 * Interactive parts of the hoista.ai landing page.
 *
 * Each feature finds its own elements through data attributes and does
 * nothing when they are absent, so sections can be removed independently.
 * The page is fully readable without this script.
 */

const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * Wires the light/dark toggle in the header.
 *
 * theme.js has already applied the initial theme. The visitor's explicit
 * choice is saved; a storage failure keeps the choice for this page view only.
 */
function setUpThemeToggle() {
  const button = document.querySelector(".theme-toggle");
  if (!button) return;

  const label = () => {
    const next = document.documentElement.dataset.theme === "light" ? "dark" : "light";
    button.setAttribute("aria-label", `Switch to ${next} mode`);
  };

  label();
  button.addEventListener("click", () => {
    const theme = document.documentElement.dataset.theme === "light" ? "dark" : "light";
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("hoista-theme", theme);
    } catch {
      // Storage blocked: the theme still applies until the page is reloaded.
    }
    label();
  });
}

/**
 * Splits a tab's `data-command` into its dim and highlighted parts.
 *
 * The attribute uses `|` around the part to emphasise, for example
 * `brew install |hoista|`.
 *
 * @param {string} command Command template from the tab.
 * @returns {{ text: string, before: string, key: string, after: string }}
 */
function parseCommand(command) {
  const [before = "", key = "", after = ""] = command.split("|");
  return { text: before + key + after, before, key, after };
}

/**
 * Wires the install box: tab switching (click and arrow keys) and copying the
 * selected command to the clipboard.
 */
function setUpInstall() {
  const box = document.querySelector("[data-install]");
  if (!box) return;

  const tabs = [...box.querySelectorAll('[role="tab"]')];
  const code = box.querySelector("[data-install-code]");
  const copy = box.querySelector("[data-copy]");
  let current = parseCommand(tabs[0].dataset.command);

  const select = (tab) => {
    for (const other of tabs) {
      other.setAttribute("aria-selected", String(other === tab));
      other.tabIndex = other === tab ? 0 : -1;
    }
    current = parseCommand(tab.dataset.command);
    const key = document.createElement("b");
    key.textContent = current.key;
    code.replaceChildren(current.before, key, current.after);
  };

  tabs.forEach((tab, index) => {
    tab.tabIndex = index === 0 ? 0 : -1;
    tab.addEventListener("click", () => select(tab));
    tab.addEventListener("keydown", (event) => {
      const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
      if (!step) return;
      const next = tabs[(index + step + tabs.length) % tabs.length];
      next.focus();
      select(next);
    });
  });

  copy.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(current.text);
      copy.classList.add("is-copied");
      copy.setAttribute("aria-label", "Copied");
      setTimeout(() => {
        copy.classList.remove("is-copied");
        copy.setAttribute("aria-label", "Copy install command");
      }, 1600);
    } catch {
      // Clipboard denied: the command stays visible and selectable.
    }
  });
}

/**
 * Points the Download button at the build for the chosen operating system,
 * preselecting the visitor's platform when it can be detected.
 */
function setUpDownload() {
  const link = document.querySelector("[data-download]");
  const os = document.querySelector("[data-download-os]");
  if (!link || !os) return;

  const platform = navigator.userAgentData?.platform ?? navigator.platform ?? "";
  if (/win/i.test(platform)) os.value = "windows";
  else if (/linux/i.test(platform)) os.value = "linux";

  const update = () => {
    link.href = `#download-${os.value}`;
  };
  update();
  os.addEventListener("change", update);
}

/**
 * Renders one frame of the Hoista logo as shimmering ASCII: a ring with a gap
 * and a dot, matching the logo mark, placed low on the right of the window.
 *
 * @param {number} tick Animation step; advances the shimmer.
 * @returns {string} Thirty rows of eighty-two characters.
 */
function ringFrame(tick) {
  const cols = 82;
  const rows = 30;
  const cellWidth = 7.2;
  const cellHeight = 14;
  const centreX = 455;
  const centreY = 262;
  const radius = 54;
  const band = 16;
  const ramp = " .:+*#";
  const dotAngle = -Math.PI / 4;
  const dotX = Math.cos(dotAngle) * radius * 1.55;
  const dotY = Math.sin(dotAngle) * radius * 1.55;
  const lines = [];

  for (let r = 0; r < rows; r++) {
    let line = "";
    for (let c = 0; c < cols; c++) {
      const x = c * cellWidth - centreX;
      const y = r * cellHeight - centreY;
      const angle = Math.atan2(y, x);
      const inGap = angle > -Math.PI / 4 - 0.12 && angle < 0.1;
      const ring = inGap ? Infinity : Math.abs(Math.hypot(x, y) - radius) - band / 2;
      const dot = Math.hypot(x - dotX, y - dotY) - 13;
      const edge = Math.min(ring, dot);
      const wave = Math.sin(c * 0.55 + r * 0.9 + tick * 0.35) * 0.5 + 0.5;

      if (edge < 0) {
        line += ramp[Math.min(5, 2 + Math.floor(wave * 4))];
      } else if (edge < 16) {
        line += wave > 0.55 ? ramp[1 + Math.floor((1 - edge / 16) * 2)] : " ";
      } else {
        const noise = Math.sin(c * 12.9898 + r * 78.233 + Math.floor(tick / 8)) * 43758.5453;
        line += noise - Math.floor(noise) > 0.985 ? "." : " ";
      }
    }
    lines.push(line);
  }
  return lines.join("\n");
}

/**
 * Plays the hero activity window: steps appear one by one, the live bar
 * follows, then the cycle repeats. Pauses while the window is off screen or
 * the tab is hidden; shows the finished state when motion is reduced.
 */
function setUpActivity() {
  const ring = document.querySelector("[data-ring]");
  const steps = [...document.querySelectorAll("[data-steps] li")];
  const live = document.querySelector("[data-live]");
  if (!ring || !steps.length || !live) return;

  const cycle = 110;
  let tick = 30;
  let visible = true;

  const render = () => {
    const shown = Math.min(steps.length + 1, 1 + Math.floor((tick % cycle) / 3));
    steps.forEach((step, index) => step.classList.toggle("is-pending", index >= shown));
    live.classList.toggle("is-pending", shown <= steps.length);
    ring.textContent = ringFrame(tick);
  };

  render();
  if (reducedMotion) return;

  new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting;
  }).observe(ring.parentElement);

  setInterval(() => {
    if (!visible || document.hidden) return;
    tick += 1;
    render();
  }, 110);
}

/**
 * Holds the newsletter form until a mailing list provider is connected.
 * Nothing is sent or stored; the button says so instead of pretending.
 */
function setUpSubscribe() {
  const form = document.querySelector("[data-subscribe]");
  if (!form) return;

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    form.querySelector("button").textContent = "Opens at launch";
  });
}

setUpThemeToggle();
setUpInstall();
setUpDownload();
setUpActivity();
setUpSubscribe();
