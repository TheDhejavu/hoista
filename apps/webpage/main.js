// hoista.dev: the theme switch, the copy buttons, the two menus, the
// terminal recording and the downloads for this computer. No dependencies.
// Each part is set up at the bottom.

const THEME_KEY = 'hoista-theme';
const COPIED_FOR_MS = 3500;

/* ---------------------------------------------------------------------------
 * Light and dark. The script in <head> has already set data-theme; this
 * keeps the screenshots and the button in step with it.
 * ------------------------------------------------------------------------ */

function setUpThemeSwitch() {
  const toggle = document.querySelector('.theme-toggle');
  applyTheme(document.documentElement.dataset.theme === 'dark');

  toggle.addEventListener('click', () => {
    const dark = document.documentElement.dataset.theme !== 'dark';
    applyTheme(dark);
    try {
      localStorage.setItem(THEME_KEY, dark ? 'dark' : 'light');
    } catch {
      // Private windows can refuse storage; the switch still works for this visit.
    }
  });

  function applyTheme(dark) {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    toggle.setAttribute('aria-label', `Switch to ${dark ? 'light' : 'dark'} mode`);

    // Every screenshot has a light copy (name.webp) and a dark one (name-dark.webp).
    for (const img of document.querySelectorAll('img[src*="assets/shots/"]')) {
      const light = img.getAttribute('src').replace('-dark.webp', '.webp');
      img.setAttribute('src', dark ? light.replace('.webp', '-dark.webp') : light);
    }
  }
}

/* ---------------------------------------------------------------------------
 * Copy buttons: copy the command beside them and say so for a few seconds.
 * ------------------------------------------------------------------------ */

function setUpCopyButtons() {
  for (const button of document.querySelectorAll('.copy')) {
    let reset;
    button.setAttribute('aria-live', 'polite');

    button.addEventListener('click', async () => {
      const text = button.parentElement.querySelector('.command').textContent.trim();
      const copied = await copyText(text, button);

      clearTimeout(reset);
      button.textContent = copied ? 'Copied!' : 'Try again';
      button.classList.toggle('done', copied);
      button.setAttribute('aria-label', copied ? 'Copied to clipboard' : 'Copy to clipboard');

      reset = setTimeout(() => {
        button.textContent = 'Copy';
        button.classList.remove('done');
        button.setAttribute('aria-label', 'Copy to clipboard');
      }, COPIED_FOR_MS);
    });
  }
}

async function copyText(text, button) {
  if (navigator.clipboard && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through to the older way below.
    }
  }

  // Opened from a file or over plain http: the clipboard API isn't there.
  const field = document.createElement('textarea');
  field.value = text;
  field.readOnly = true;
  field.style.cssText = 'position: fixed; top: 0; left: -9999px';
  document.body.append(field);
  field.select();
  let copied = false;
  try {
    copied = document.execCommand('copy');
  } catch {
    copied = false;
  }
  field.remove();
  button.focus({ preventScroll: true });
  return copied;
}

/* ---------------------------------------------------------------------------
 * The phone menu and the other-downloads menu: Escape closes them, and
 * choosing a link or clicking elsewhere puts them away.
 * ------------------------------------------------------------------------ */

function setUpMenus() {
  const menus = document.querySelectorAll('.mobile-nav, .other-downloads');

  for (const link of document.querySelectorAll('.mobile-nav a')) {
    link.addEventListener('click', () => {
      link.closest('details').open = false;
    });
  }

  document.addEventListener('click', (event) => {
    for (const menu of menus) {
      if (!menu.contains(event.target)) menu.open = false;
    }
  });

  document.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape') return;
    for (const menu of menus) {
      if (!menu.open) continue;
      menu.open = false;
      menu.querySelector('summary').focus();
    }
  });
}

/* ---------------------------------------------------------------------------
 * "Prefer the terminal?": one terminal whose numbered tabs play each step:
 * install, connect Railway, add Hoista to Claude Code, then Claude Code
 * deploying a repository through it. It tours the steps while in view;
 * choosing a tab stops on that one. With reduced motion each step shows at
 * once.
 * ------------------------------------------------------------------------ */

const PROJECT_FOLDER = '/Users/sam/moodboard';
const LIVE_URL = 'https://moodboard-production.up.railway.app';

// Each line is [kind, text]. command: typed at the prompt. output, ok, dim:
// printed.
const STEPS = [
  {
    title: '~/moodboard — zsh',
    lines: [
      ['command', 'curl -fsSL https://hoista.dev/install.sh | sh -s -- v0.1.0-beta.1'],
      ['output', 'Verified OK'],
      ['output', 'Installed Hoista to /Users/sam/.local/bin. Add this directory to PATH.'],
      ['output', 'Hoista will start at login and restart after exit.'],
      ['dim', 'Next: hoist accounts add, then hoist mcp config to connect your coding tool.'],
    ],
  },
  {
    title: '~/moodboard — zsh',
    lines: [
      ['command', 'hoist accounts add railway'],
      [
        'output',
        'Make a token at https://railway.com/account/tokens (Hoista needs read and deploy access).',
      ],
      ['output', 'Railway token (hidden):'],
      ['ok', '✓ Connected Sam Rivera’s Projects. Found 3 resources.'],
      ['output', 'Bring its projects in with hoist import.'],
    ],
  },
  {
    title: '~/moodboard — zsh',
    lines: [
      ['command', 'claude mcp add --scope user hoista -- hoist mcp'],
      ['output', 'Added stdio MCP server hoista with command: hoist mcp  to user config'],
      ['dim', 'File modified: /Users/sam/.claude.json'],
    ],
  },
  { title: '~/moodboard — claude', claude: true },
];

// The Hoista tools Claude Code calls in the last step, and what each returns.
const CLAUDE_CALLS = [
  [
    'create_project_at_provider',
    'name: "moodboard", repo: "samrivera/moodboard"',
    '{"project":{"id":"5f0c…","name":"moodboard"},"resources":[{"id":"r7c1…","kind":"web","provider":"railway"}]}',
  ],
  [
    'get_provider_details',
    'id: "r7c1…"',
    '{"deployments":[{"id":"d41a…","status":"building","commit":"a3f9e21"}]}',
  ],
  ['wait_for_deploy', 'id: "d41a…"', `{"finished":true,"status":"live","url":"${LIVE_URL}"}`],
];

// How long each part of the recording takes, in milliseconds.
const PAUSE = {
  beforeTyping: 350,
  afterTyping: 250,
  perKeyShort: 34,
  perKeyLong: 16,
  afterLine: 450,
  afterBanner: 500,
  toolCall: 900,
  slowToolCall: 1800,
  afterResult: 350,
  betweenSteps: 2200,
  beforeRestart: 5500,
};

function setUpTerminal() {
  const section = document.getElementById('terminal');
  const screen = document.getElementById('terminal-screen');
  const title = document.getElementById('terminal-where');
  const tabs = [...document.querySelectorAll('.terminal-tab')];
  const still = matchMedia('(prefers-reduced-motion: reduce)').matches;

  let run = 0; // Bumped to stop whatever is playing.
  let current = 0;
  let touring = true;
  let playing = false;

  const wait = (ms) => new Promise((done) => setTimeout(done, still ? 0 : ms));

  // Like a real terminal, the newest line stays in view once the window is full.
  new MutationObserver(() => {
    screen.scrollTop = screen.scrollHeight;
  }).observe(screen, { childList: true, subtree: true, characterData: true });

  tabs.forEach((tab, index) => {
    tab.id = `terminal-tab-${index}`;
    tab.setAttribute('aria-controls', 'terminal-screen');
    tab.addEventListener('click', () => {
      touring = false;
      play(index);
    });
    tab.addEventListener('keydown', (event) => {
      const by = { ArrowRight: 1, ArrowLeft: -1 }[event.key];
      if (!by) return;
      event.preventDefault();
      const next = tabs[(index + by + tabs.length) % tabs.length];
      next.focus();
      next.click();
    });
  });

  // Plays while in view, stops when scrolled away and picks up again on return.
  new IntersectionObserver(
    (entries) => {
      const shown = entries.some((entry) => entry.isIntersecting);
      if (shown && !playing) {
        play(current);
      } else if (!shown && touring) {
        run++;
        playing = false;
      }
    },
    { threshold: 0.3 },
  ).observe(section);

  async function play(index) {
    const mine = ++run;
    const alive = () => mine === run;
    current = index;
    playing = true;

    tabs.forEach((tab, i) => {
      tab.setAttribute('aria-selected', String(i === index));
      tab.tabIndex = i === index ? 0 : -1;
      tab.style.setProperty('--done', '0');
    });
    screen.setAttribute('aria-labelledby', tabs[index].id);
    title.textContent = STEPS[index].title;

    const lines = element('div', 'terminal-lines');
    screen.replaceChildren(lines);
    if (STEPS[index].claude) {
      await playClaude(lines, alive);
    } else {
      await playShell(STEPS[index].lines, lines, alive);
    }

    if (!alive() || still || !touring) return;
    await wait(index === STEPS.length - 1 ? PAUSE.beforeRestart : PAUSE.betweenSteps);
    if (alive() && touring) play((index + 1) % STEPS.length);
  }

  function showProgress(done, total) {
    tabs[current].style.setProperty('--done', String(done / total));
  }

  async function type(target, text, alive) {
    const caret = element('span', 'caret');
    const typed = document.createTextNode('');
    target.append(typed, caret);
    await wait(PAUSE.beforeTyping);

    const perKey = text.length > 50 ? PAUSE.perKeyLong : PAUSE.perKeyShort;
    for (let n = 1; n <= text.length && !still; n++) {
      if (!alive()) return false;
      typed.textContent = text.slice(0, n);
      await wait(perKey);
    }
    typed.textContent = text;
    caret.remove();
    await wait(PAUSE.afterTyping);
    return true;
  }

  function promptLine() {
    const line = element('div', 'line');
    line.append(element('span', 'prompt', '$ '));
    return line;
  }

  async function playShell(steps, lines, alive) {
    for (const [index, [kind, text]] of steps.entries()) {
      if (!alive()) return;

      if (kind === 'command') {
        const line = promptLine();
        lines.append(line);
        if (!(await type(line, text, alive))) return;
      } else {
        lines.append(element('div', `line appear ${kind}`, text));
        await wait(PAUSE.afterLine);
      }
      showProgress(index + 1, steps.length);
    }

    if (!alive()) return;
    const end = promptLine();
    end.append(element('span', 'caret'));
    lines.append(end);
  }

  // Claude Code in the same terminal, the way it shows things: the prompt,
  // each MCP tool call (⏺ server - tool (MCP)(arguments)) with its result
  // (⎿), then its answer.
  async function playClaude(lines, alive) {
    const total = CLAUDE_CALLS.length + 2;

    const start = promptLine();
    lines.append(start);
    if (!(await type(start, 'claude', alive))) return;

    const banner = element('div', 'claude-banner appear');
    banner.append(
      element('span', 'star', '✻ '),
      'Welcome to Claude Code!',
      document.createElement('br'),
      element('span', 'cwd', `cwd: ${PROJECT_FOLDER}`),
    );
    lines.append(banner);
    await wait(PAUSE.afterBanner);

    const ask = element('div', 'line claude-input', '> ');
    lines.append(ask);
    if (!(await type(ask, 'Deploy samrivera/moodboard to Railway and send me the link.', alive)))
      return;
    showProgress(1, total);

    for (const [index, [tool, args, result]] of CLAUDE_CALLS.entries()) {
      if (!alive()) return;
      const call = element('div', 'line appear');
      call.append(
        element('span', 'claude-dot', '⏺ '),
        element('span', 'claude-tool', `hoista - ${tool} (MCP)`),
        `(${args})`,
      );
      lines.append(call);
      await wait(tool === 'wait_for_deploy' ? PAUSE.slowToolCall : PAUSE.toolCall);
      lines.append(element('div', 'line appear claude-result', `  ⎿  ${result}`));
      showProgress(index + 2, total);
      await wait(PAUSE.afterResult);
    }

    if (!alive()) return;
    const reply = element('div', 'line appear');
    reply.append(
      element('span', 'claude-dot reply', '⏺ '),
      `moodboard is live at ${LIVE_URL}. It built and passed its health check on Railway.`,
    );
    lines.append(reply);
    showProgress(total, total);

    const next = element('div', 'line claude-input', '> ');
    next.append(element('span', 'caret'));
    lines.append(next);
  }
}

/* ---------------------------------------------------------------------------
 * Downloads: say what the page detected under the buttons and point at the
 * right installer. Only Chromium browsers can tell an Intel Mac from an
 * Apple Silicon one; elsewhere a Mac gets Apple Silicon, with Intel offered.
 * ------------------------------------------------------------------------ */

const WINDOWS_INSTALL = 'https://docs.hoista.dev/getting-started/install#windows';

async function setUpDownloads() {
  const note = document.querySelector('.detected');
  const intel = document.querySelector('[data-mac-other]');
  const { system, chip } = await detectComputer();

  if (system === 'macOS' && chip === 'x86') {
    useIntelBuild();
    say('Detected a Mac with an Intel chip.');
  } else if (system === 'macOS' && chip === 'arm') {
    say('Detected a Mac with Apple Silicon.');
  } else if (system === 'macOS') {
    say('Detected a Mac. On an Intel Mac? ', link(intel.href, 'Get the Intel version'), '.');
  } else if (system === 'Linux') {
    const appImage = document.querySelector('[data-linux="appimage"]').href;
    const deb = document.querySelector('[data-linux="deb"]').href;
    say('Detected Linux. Get the ', link(appImage, 'AppImage'), ' or the ', link(deb, '.deb'), '.');
  } else if (system === 'Windows') {
    say(
      'Detected Windows. Desktop for Windows is coming soon; until then, ',
      link(WINDOWS_INSTALL, 'install the CLI'),
      '.',
    );
  } else if (system === 'iOS' || system === 'Android') {
    say('Hoista runs on a computer. Open this page on a Mac or Linux PC to download it.');
  }

  function say(...parts) {
    note.replaceChildren(...parts);
    note.hidden = false;
  }

  function link(href, text) {
    const anchor = element('a', null, text);
    anchor.href = href;
    return anchor;
  }

  // The buttons take the Intel build, and the menu offers Apple Silicon instead.
  function useIntelBuild() {
    const appleSilicon = document.querySelector('[data-mac-download]').href;
    for (const button of document.querySelectorAll('[data-mac-download]')) {
      button.href = intel.href;
      button.querySelector('.mac-chip').textContent = 'Intel';
    }
    intel.href = appleSilicon;
    intel.querySelector('strong').textContent = 'macOS Apple Silicon';
    intel.querySelector('strong + span').textContent = 'M1 and later · .dmg';
    document.querySelector('[data-mac-other-chip]').textContent = 'Apple Silicon';
  }
}

async function detectComputer() {
  const agent = navigator.userAgent;
  let system = navigator.userAgentData?.platform;
  if (!system) {
    if (/iPhone|iPad|iPod/.test(agent)) system = 'iOS';
    else if (/Android/.test(agent)) system = 'Android';
    // iPads ask for the desktop site and say Macintosh; only they have touch.
    else if (/Macintosh/.test(agent)) system = navigator.maxTouchPoints > 1 ? 'iOS' : 'macOS';
    else if (/Windows/.test(agent)) system = 'Windows';
    else if (/Linux/.test(agent)) system = 'Linux';
  }

  let chip;
  if (system === 'macOS' && navigator.userAgentData) {
    try {
      const values = await navigator.userAgentData.getHighEntropyValues(['architecture']);
      chip = values.architecture;
    } catch {
      // The browser can refuse; the Mac then gets the default build.
    }
  }
  return { system, chip };
}

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

setUpThemeSwitch();
setUpCopyButtons();
setUpMenus();
setUpTerminal();
setUpDownloads();
