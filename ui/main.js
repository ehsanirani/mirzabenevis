// Popup behaviour. Talks to the Rust side through the global Tauri API
// (withGlobalTauri = true in tauri.conf.json), so no bundler is needed.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const win = window.__TAURI__.window.getCurrentWindow();
const { LogicalSize } = window.__TAURI__.dpi;

const $ = (id) => document.getElementById(id);
const actionsEl = $("actions");
const original = $("original");
const resultEl = $("result");
const resultName = $("resultName");
const statusEl = $("status");
const backendEl = $("backend");
const picker = $("picker");
const pickerList = $("pickerList");
const pinEl = $("pin");
const promptEl = $("prompt");
const recentsEl = $("recents");
const modsEl = $("modifiers");
const contextEl = $("context");
const CUSTOM = "_custom";
const EXPLAIN = "The text holds an original and a rewritten version of it. List the changes "
  + "made in the rewrite and why, max 5 bullets. Answer in {explain_in}.";

const store = (k, v) => (v === undefined ? JSON.parse(localStorage.getItem(k) || "null") : localStorage.setItem(k, JSON.stringify(v)));
const el = (tag, cls, text) => Object.assign(document.createElement(tag), cls ? { className: cls } : {}, text ? { textContent: text } : {});

// Window size (not position: Wayland does not tell apps where they are) and zoom are remembered.
// Size in logical pixels, so it survives a change of display scale.
async function restoreSize() {
  const s = store("size");
  // Clamp to the screen: a size saved on another display (or by an older build
  // in physical pixels) must not produce a window Mutter cannot show.
  if (s && s.w >= 520 && s.h >= 360) {
    const w = Math.min(s.w, screen.availWidth - 40);
    const h = Math.min(s.h, screen.availHeight - 40);
    await win.setSize(new LogicalSize(w, h)).catch(() => {});
  }
  let t;
  await win.onResized(({ payload }) => {
    clearTimeout(t);
    t = setTimeout(async () => {
      const l = payload.toLogical(await win.scaleFactor());
      store("size", { w: l.width, h: l.height });
    }, 300);
  });
}
// Zoom scales vw/vh too; CSS divides those by --z so they stay window-relative.
const applyZoom = () => {
  document.documentElement.style.zoom = zoom;
  document.documentElement.style.setProperty("--z", zoom);
};
let zoom = store("zoom") || 1;
applyZoom();
function setZoom(z) {
  zoom = Math.min(1.4, Math.max(0.8, Math.round(z * 10) / 10));
  applyZoom();
  store("zoom", zoom);
  setStatus(`Zoom ${Math.round(zoom * 100)}%`);
}

// The original grows with its text up to 40% of the window (CSS max-height); the result gets the rest.
const grow = () => {
  original.style.height = "auto";
  original.style.height = `${original.scrollHeight}px`;
};
addEventListener("resize", grow); // the 40% cap moves with the window

let pinned = true; // default; matches the Rust side
function setPinned(v) {
  pinned = v;
  pinEl.textContent = v ? "pinned" : "unpinned";
  invoke("set_pinned", { pinned: v });
}

// Recent custom instructions (from the history file) feed the input's <datalist>.
async function loadRecents() {
  const recents = await invoke("recent_prompts").catch(() => []);
  recentsEl.replaceChildren(...recents.map((p) => Object.assign(document.createElement("option"), { value: p })));
}

// The context line is remembered for the current calendar day only.
const today = () => new Date().toDateString();
function loadContext() {
  const c = store("context");
  contextEl.value = c?.day === today() ? c.text : "";
  contextEl.hidden = !contextEl.value;
}
contextEl.addEventListener("input", () => store("context", { day: today(), text: contextEl.value }));
function openContext() {
  contextEl.hidden = false;
  contextEl.focus();
}

let actions = [];
let active = null;      // action id the next Enter/R runs
let result = "";        // plain result text; Copy/Replace always use this
let error = "";         // last run's error, shown in the result pane
let lastInput = "";     // what the result was made from (diff base)
let resultAction = null;
let resultPrompt = "";  // the instruction behind the result (seeds a chat that follows up on it)
let resultLabel = "";
let resultOutput = "replace"; // a refine keeps the output mode of what it refines
let grabbed = "";       // the selection in the other app: what a Replace overwrites (for undo)
let maxChars = Infinity; // general.max_input_chars
let resultBackend = null; // id of the backend that answered (explain-change reuses it)
let explain = "";       // X: what changed and why; a second view, never pasted or copied
let showExplain = false;
let target = "";        // translate.target
let explainIn = "";     // general.explain_in
let defaultAction = "";
let noSelection = false; // the grab came back empty: `/` on an empty original chats
let chat = null;        // [{role, text}] while chatting, else null
let chatLive = "";      // the answer streaming in
let liveEl = null;      // ...and where it is shown
let modifiers = [];
const mods = new Set(); // modifier ids switched on for the next run
let dirty = false;      // original edited since the last run
let running = false;
let backendError = null;
let chosenLabel = "";   // backend that will run, from the last get_backends
let configPath = "";
let pasteHow = "pasted"; // what Replace does: "pasted" | "typed" | "copied"
let grabStatus = ["", false]; // [message, isError] from the last grab
// Every run gets a number. Cancelling, closing or reopening the popup moves `runId` on,
// so late output from an abandoned run is ignored instead of being pasted.
let runId = 0;
// Same idea for the backend picker: only the latest answer is rendered.
let backendSeq = 0;

const setStatus = (msg, isError = false) => {
  statusEl.textContent = msg;
  statusEl.style.color = isError ? "var(--error)" : "";
};
// Idle messages lose to a backend problem: nothing can run until it is fixed.
const idleStatus = (msg, isError = false) => setStatus(backendError || msg, !!backendError || isError);

const actionName = (id) => (id === CUSTOM ? "Custom" : actions.find((a) => a.id === id)?.name || id);
const outputOf = (id) => actions.find((a) => a.id === id)?.output || "replace";
const diffOn = (id) => store(`diff:${id}`) ?? id === "fix";

// Chat iff nothing was selected and the original is still empty (or a chat is going on).
const chatting = () => !!chat || (noSelection && !original.value.trim());

// Footer hints, placeholder and labels follow the state, so they never lie.
function sync() {
  const out = result && !dirty ? resultOutput : null;
  $("enterKey").hidden = out === "show" || !!chat; // show-only result: C copies, nothing replaces
  $("enterHint").textContent = !out ? "Run" : out === "clipboard" || pasteHow === "copied" ? "Copy" : "Replace";
  $("escHint").textContent = running ? "Cancel" : "Close";
  promptEl.placeholder = chatting() ? "Message… (Enter sends, N new chat)"
    : out === "show" ? "Ask a follow-up… (continues as a chat)"
    : out ? "Refine result… (acts on the result)" : "/ Type an instruction…";
}

const turnEl = (t) => {
  const d = el("div", `turn ${t.role === "user" ? "you" : "ai md"}`);
  if (t.role === "user") d.textContent = t.text;
  else d.replaceChildren(...md(t.text));
  return d;
};

function renderResult() {
  sync();
  $("resultTitle").textContent = chat ? "Chat" : "Result";
  $("modeTag").textContent = chat ? "· chat" : "";
  resultName.textContent = chat ? "· N new chat" : showExplain ? "· What changed (X back)" : resultLabel;
  resultEl.className = "result-text";
  resultEl.setAttribute("aria-busy", String(running)); // the live region speaks once, when done
  // In a chat only the newest answer is live (not every turn again).
  resultEl.setAttribute("aria-live", chat ? "off" : "polite");
  if (chat) {
    const last = chat.at(-1);
    if (!running && liveEl?.isConnected && last?.role === "assistant" && !error) {
      // The answer just finished: fill its live turn in place, so it is read out once.
      liveEl.className = "turn ai md";
      liveEl.replaceChildren(...md(last.text));
      liveEl.setAttribute("aria-busy", "false");
      liveEl = null;
      resultEl.scrollTop = resultEl.scrollHeight;
      return;
    }
    resultEl.replaceChildren(...chat.map(turnEl));
    liveEl = null;
    if (running) {
      liveEl = resultEl.appendChild(el("div", "turn ai streaming", chatLive));
      liveEl.setAttribute("aria-live", "polite");
      liveEl.setAttribute("aria-busy", "true");
    }
    if (error) resultEl.append(el("div", "turn error", error));
    if (!chat.length && !running && !error) resultEl.append(el("div", "turn empty", "Type a message below; Enter sends."));
    resultEl.scrollTop = resultEl.scrollHeight;
    return;
  }
  if (running) return; // streaming text is written by the chunk listener
  if (showExplain) {
    resultEl.classList.add("md");
    resultEl.replaceChildren(...md(explain));
  } else if (error) {
    resultEl.classList.add("error");
    resultEl.textContent = error;
  } else if (result && resultOutput === "show") {
    resultEl.classList.add("md"); // D (diff) has no effect here
    resultEl.replaceChildren(...md(result));
  } else if (result && diffOn(resultAction)) {
    resultEl.replaceChildren(...wordDiff(lastInput, result).map(([op, t]) => {
      if (op === "=") return document.createTextNode(t);
      const el = document.createElement(op === "-" ? "del" : "ins");
      el.textContent = t;
      return el;
    }));
  } else if (result) {
    resultEl.textContent = result;
  } else {
    resultEl.classList.add("empty");
    const keys = actions.map((a) => a.key).filter(Boolean);
    resultEl.textContent = backendError
      ? `${backendError}\n\nSet an API key, install an agent CLI (claude, codex) or start a local server, `
        + `then check with: mirzabenevis-cli providers --check\nConfig: ${configPath}`
      : chatting() ? "Nothing selected: / to chat, or E to type or paste text to work on."
      : `Enter: ${actionName(active)}` + (keys.length ? ` · ${keys[0]}–${keys.at(-1)} other actions` : "") + " · / instruction";
  }
}

async function init() {
  restoreSize().catch(() => {}); // size is a nicety; never block the popup on it
  actions = await invoke("get_actions");
  configPath = await invoke("config_path").catch(() => "");
  await listen("chunk", (e) => {
    if (!running || e.payload.run !== runId) return;
    if (chat) {
      chatLive += e.payload.text;
      if (liveEl) liveEl.textContent = chatLive;
    } else if (showExplain) {
      explain += e.payload.text;
      resultEl.textContent = explain;
    } else {
      result += e.payload.text;
      resultEl.textContent = result;
    }
    resultEl.scrollTop = resultEl.scrollHeight;
  });
}

// Backend picker: a footer button opening a dialog with "Auto" (first found) and every
// backend detected on this machine, each with its model (Enter in the field saves it).
const describe = (b) => `${b.label} · ${b.model}`;
const short = (b) => (b.model === "default model" ? b.label : b.model);
const GROUPS = [["api", "APIs"], ["agent", "Agent CLIs"], ["local", "Local"], ["custom", "Custom"]];
let view = null; // the last get_backends answer

function renderBackends(v) {
  view = v;
  const byId = Object.fromEntries(v.found.map((b) => [b.id, b]));
  const autoPick = v.wanted === "auto" && v.chosen ? byId[v.chosen] : v.found[0];
  const label = v.wanted === "auto" ? (autoPick ? `Auto: ${short(autoPick)}` : "Auto: none")
    : byId[v.wanted] ? short(byId[v.wanted]) : `${v.wanted} (n/a)`;
  backendEl.textContent = `${label} ▾`;
  backendError = v.error;
  chosenLabel = v.chosen && byId[v.chosen] ? describe(byId[v.chosen]) : "";
  backendEl.classList.toggle("error", !!v.error);
  backendEl.title = v.error || `${chosenLabel || "AI backend"} — M to change (remembered in config.toml)`;
}

function renderPicker() {
  const v = view;
  const byId = Object.fromEntries(v.found.map((b) => [b.id, b]));
  const autoPick = v.wanted === "auto" && v.chosen ? byId[v.chosen] : v.found[0];
  const row = (id, name, detail, field) => {
    const b = el("button", "opt");
    b.type = "button";
    if (id === v.wanted) b.setAttribute("aria-current", "true");
    b.append(el("span", "check", id === v.wanted ? "✓" : ""), el("span", "name", name), el("span", "muted", detail));
    b.addEventListener("click", () => chooseBackend(id));
    const r = el("div", "row");
    r.append(b, ...(field ? [field] : []));
    return r;
  };
  const modelField = (b) => {
    const f = el("input", "model");
    f.type = "text";
    f.spellcheck = false;
    f.value = b.model === "default model" ? "" : b.model;
    f.placeholder = "default model";
    f.title = "Model (Enter saves; empty = the default)";
    f.setAttribute("aria-label", `Model for ${b.label} (Enter saves)`);
    f.addEventListener("keydown", (e) => {
      if (e.key !== "Enter") return;
      e.preventDefault();
      saveModel(b.id, f.value);
    });
    return f;
  };
  const kids = [row("auto", "Auto", autoPick ? describe(autoPick) : "nothing found")];
  for (const [cat, title] of GROUPS) {
    const bs = v.found.filter((b) => b.category === cat);
    if (!bs.length) continue;
    const g = el("div", "group");
    g.setAttribute("role", "group");
    g.setAttribute("aria-label", title);
    g.append(el("div", "group-title", title), ...bs.map((b) => row(b.id, b.label, b.source, modelField(b))));
    kids.push(g);
  }
  // A pinned backend that is not available right now still shows, marked.
  if (v.wanted !== "auto" && !byId[v.wanted]) kids.push(row(v.wanted, `${v.wanted} (not available)`, v.error || ""));
  pickerList.replaceChildren(...kids);
}

let opener = null; // focus goes back there when the picker closes
function openPicker() {
  if (!view) return setStatus("Still looking for backends…");
  opener = document.activeElement;
  renderPicker();
  picker.showModal();
  (picker.querySelector(".opt[aria-current=true]") || picker.querySelector(".opt")).focus();
}
picker.addEventListener("close", () => (opener?.isConnected && opener !== document.body ? opener.focus() : home()));
picker.addEventListener("click", (e) => e.target === picker && picker.close()); // the backdrop
picker.addEventListener("keydown", (e) => {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  e.preventDefault();
  const opts = [...picker.querySelectorAll(".opt")];
  const i = opts.indexOf(document.activeElement.closest(".row")?.querySelector(".opt"));
  opts[(i + (e.key === "ArrowDown" ? 1 : -1) + opts.length) % opts.length].focus();
});
backendEl.addEventListener("click", openPicker);

async function loadBackends(refresh) {
  const seq = ++backendSeq;
  try {
    const v = await invoke("get_backends", { refresh });
    if (seq === backendSeq) renderBackends(v);
  } catch (e) {
    backendError = String(e);
  }
}

// Both save to config.toml and answer with a fresh picker view.
async function pickerCall(cmd, args, done) {
  const seq = ++backendSeq;
  try {
    const v = await invoke(cmd, args);
    if (seq === backendSeq) renderBackends(v);
    setStatus(backendError || done, !!backendError);
  } catch (e) {
    setStatus(String(e), true);
  }
  picker.close();
  if (!running) renderResult();
}
const chooseBackend = (id) => pickerCall("set_backend", { id }, "Backend saved");
const saveModel = (id, model) => pickerCall("set_model", { id, model }, model.trim() ? `${id} now uses ${model.trim()}` : `${id} uses its default model`);

function chip(key, label, title, cls, onClick) {
  const b = document.createElement("button");
  b.className = "action" + cls;
  b.type = "button";
  const k = document.createElement("span");
  k.className = "k";
  k.textContent = key.toUpperCase() || "·";
  b.append(k, label);
  b.title = title;
  b.addEventListener("click", onClick);
  return b;
}

// Per-chip menu (right-click). Middle-click = "Edit prompt in the field" directly.
const chipMenu = $("chipMenu");
function promptToField(a) {
  promptEl.value = promptOf(a.id);
  promptEl.focus();
  promptEl.setSelectionRange(promptEl.value.length, promptEl.value.length);
  setStatus(`Prompt of "${a.name}" — edit, Enter runs it, ★ saves it as a new button`);
}
function closeChipMenu() {
  chipMenu.hidden = true;
  chipMenu.replaceChildren();
}
function openChipMenu(a, x, y) {
  const item = (label, fn, cls) => {
    const b = el("button", cls || "", label);
    b.type = "button";
    b.setAttribute("role", "menuitem");
    b.addEventListener("click", () => { closeChipMenu(); fn(); });
    return b;
  };
  const items = [
    item("Edit prompt in the field", () => promptToField(a)),
    item("Copy prompt to clipboard", async () => { await invoke("copy", { result: promptOf(a.id) }); setStatus(`Copied the prompt of "${a.name}"`); }),
    item(`Run "${a.name}"`, () => run(a.id)),
  ];
  if (!a.builtin) items.push(el("div", "sep"), item(`Remove "${a.name}"`, () => removeAction(a), "danger"));
  chipMenu.replaceChildren(...items);
  chipMenu.hidden = false;
  // Keep it on screen.
  const r = chipMenu.getBoundingClientRect();
  chipMenu.style.left = Math.min(x, innerWidth - r.width - 8) + "px";
  chipMenu.style.top = Math.min(y, innerHeight - r.height - 8) + "px";
  items[0].focus();
}
document.addEventListener("pointerdown", (e) => { if (!chipMenu.hidden && !chipMenu.contains(e.target)) closeChipMenu(); });
chipMenu.addEventListener("keydown", (e) => {
  const bs = [...chipMenu.querySelectorAll("button")];
  const i = bs.indexOf(document.activeElement);
  if (e.key === "ArrowDown") { e.preventDefault(); bs[(i + 1) % bs.length].focus(); }
  else if (e.key === "ArrowUp") { e.preventDefault(); bs[(i - 1 + bs.length) % bs.length].focus(); }
});

function renderActions() {
  actionsEl.replaceChildren(...actions.map((a) => {
    const name = a.target ? `${a.name} → ${target}` : a.name;
    const c = chip(a.key, name, `Run "${name}"` + (a.target ? " (T: other language)" : "") + `\nRight-click: edit its prompt\n\n${promptOf(a.id)}`, a.id === active ? " active" : "", () => run(a.id));
    // Right-click opens a small menu; middle-click puts the prompt straight into the instruction field.
    c.addEventListener("contextmenu", (e) => { e.preventDefault(); openChipMenu(a, e.clientX, e.clientY); });
    c.addEventListener("auxclick", (e) => { if (e.button === 1) { e.preventDefault(); promptToField(a); } });
    if (a.builtin) return c;
    // Your own chips (★) get a remove button on hover.
    const x = el("button", "del", "×");
    x.type = "button";
    x.title = `Remove "${a.name}" from config.toml`;
    x.setAttribute("aria-label", `Remove ${a.name}`);
    x.addEventListener("click", () => removeAction(a));
    const w = el("span", "user-chip");
    w.append(c, x);
    return w;
  }));
}

// ★: save the instruction as a chip (a new [[actions]] entry in config.toml).
async function saveInstruction() {
  const prompt = promptEl.value.trim();
  if (!prompt) {
    setStatus("Type an instruction first; ★ saves it as a button");
    return promptEl.focus();
  }
  try {
    actions = await invoke("add_action", { prompt });
    renderActions();
    setStatus(`Saved as a button: ${actions.at(-1).name} (× on hover removes it)`);
  } catch (e) {
    setStatus(String(e), true);
  }
}
$("star").addEventListener("click", saveInstruction);

async function removeAction(a) {
  try {
    actions = await invoke("remove_action", { id: a.id });
    if (active === a.id) active = defaultAction;
    renderActions();
    setStatus(`Removed "${a.name}"`);
    home();
  } catch (e) {
    setStatus(String(e), true);
  }
}

function renderModifiers() {
  modsEl.replaceChildren(...modifiers.map((m) => {
    // Focus goes back home, so Enter still runs/replaces after a click.
    const b = chip(m.key, m.name, `For the next run: ${m.suffix}`, mods.has(m.id) ? " on" : "", () => { toggleMod(m.id); home(); });
    b.setAttribute("aria-pressed", String(mods.has(m.id)));
    return b;
  }));
}
function toggleMod(id) {
  if (!mods.delete(id)) mods.add(id);
  renderModifiers();
}

async function cycleTarget() {
  try {
    target = await invoke("cycle_target");
    renderActions();
    setStatus(`Translate → ${target} (saved)`);
  } catch (e) {
    setStatus(String(e), true);
  }
}

// Where focus rests when not editing: the active chip (so keys and Enter work).
const home = () => (actionsEl.querySelector(".active") || actionsEl.querySelector(".action") || resultEl).focus();

original.addEventListener("input", () => {
  dirty = true;
  grow();
  if (chat && original.value.trim()) leaveChat(); // text to work on: back to actions
  sync();
});

async function onShown() {
  if (picker.open) picker.close();
  const shown = ++runId; // forget whatever was running when the popup was last open
  running = false;
  result = error = explain = "";
  showExplain = false;
  dirty = false;
  chat = null;
  noSelection = false;
  original.value = grabbed = "";
  grow();
  mods.clear();
  loadContext();
  loadRecents();
  grabStatus = ["", false];
  setStatus("");
  const info = await invoke("get_info");
  active = defaultAction = info.default_action;
  explainIn = info.explain_in;
  pasteHow = info.paste;
  target = info.target;
  maxChars = info.max_input_chars;
  modifiers = info.modifiers;
  renderActions();
  renderModifiers();
  renderResult();
  // Detection runs in the background; keys work meanwhile.
  loadBackends(true).then(() => {
    if (running || result) return;
    idleStatus(...grabStatus); // also clears a backend error that went away
    renderResult();
  });
  try {
    const text = await invoke("grab");
    grabStatus = [text ? "" : "Nothing selected — / to chat", false];
    original.value = grabbed = text;
  } catch (e) {
    grabStatus = [String(e), true];
  }
  noSelection = !grabbed;
  grow();
  // A run started before the grab landed worked on stale text.
  if (runId !== shown) cancelRun();
  dirty = false;
  idleStatus(...grabStatus);
  renderResult();
  home();
}

// Chips and R act on the original; the instruction field refines the result when there is one.
async function run(id, refine = false) {
  const input = refine ? result : original.value;
  if (!input.trim()) {
    setStatus("Nothing to work on", true);
    return;
  }
  const prompt = id === CUSTOM ? promptEl.value.trim() : null;
  if (id === CUSTOM && !prompt) {
    promptEl.focus();
    return;
  }
  // A run already in flight is aborted by the Rust side when this one registers.
  const myRun = ++runId;
  chat = null; // chips and R act on the original
  if (!refine) {
    active = id; // a refine is a one-off: Enter/R keep their action
    renderActions();
  }
  dirty = false;
  running = true;
  result = error = explain = "";
  showExplain = false;
  lastInput = input;
  resultAction = id;
  resultPrompt = prompt || promptOf(id);
  if (!refine) resultOutput = outputOf(id);
  const used = [...mods]; // toggles made while this runs count for the next run
  const modNames = modifiers.filter((m) => mods.has(m.id)).map((m) => m.name);
  resultLabel = `· ${prompt || actionName(id)}` + (modNames.length ? ` + ${modNames.join(", ")}` : "");
  resultEl.textContent = "";
  renderResult();
  resultEl.classList.add("streaming");
  const { secs, timer } = thinking(myRun);
  try {
    const res = await invoke("run_action", { run: myRun, actionId: id, input, prompt, modifiers: used, context: contextEl.value });
    if (myRun !== runId) return; // abandoned meanwhile
    result = res.text;
    resultBackend = res.backend_id;
    setStatus(`✓ ${res.backend} · ${secs()}s`);
    used.forEach((m) => mods.delete(m)); // modifiers apply to one run
    renderModifiers();
    if (prompt) loadRecents();
  } catch (e) {
    if (myRun !== runId) return;
    result = "";
    error = String(e);
    setStatus("R retry · M picks another backend", true);
  } finally {
    clearInterval(timer);
    if (myRun === runId) {
      running = false;
      renderResult();
      if (result) resultEl.focus();
    }
  }
}

// "Thinking…" with the backend, and the seconds after 3 s; `secs()` for the final status.
function thinking(myRun) {
  const t0 = Date.now();
  setStatus(`Thinking…${chosenLabel ? ` ${chosenLabel}` : ""}`);
  // Seconds tick in a hidden span so the status region does not chatter.
  const secsEl = statusEl.appendChild(document.createElement("span"));
  secsEl.setAttribute("aria-hidden", "true");
  const timer = setInterval(() => {
    if (myRun !== runId) return clearInterval(timer);
    const s = Math.floor((Date.now() - t0) / 1000);
    if (s >= 3) secsEl.textContent = ` · ${s}s`;
  }, 1000);
  return { secs: () => ((Date.now() - t0) / 1000).toFixed(1), timer };
}

// An action's prompt minus the text (a chat's first turn adds that); Rust fills {target}/{explain_in}.
const promptOf = (id) => (actions.find((a) => a.id === id)?.prompt || actionName(id)).replace("{text}", "").trim();

// Chat: when nothing was selected, or as a follow-up to a show result (summary, explanation…).
async function sendChat() {
  const msg = promptEl.value.trim();
  if (!msg || running) return;
  if (!chat) {
    const followUp = result && !dirty && resultOutput === "show";
    chat = followUp ? [{ role: "user", text: `${resultPrompt}\n\n${lastInput}` }, { role: "assistant", text: result }] : [];
  }
  chat.push({ role: "user", text: msg });
  promptEl.value = "";
  const myRun = ++runId;
  running = true;
  error = chatLive = explain = "";
  showExplain = false;
  resultOutput = "show"; // C copies the last answer; nothing replaces
  renderResult();
  const { secs, timer } = thinking(myRun);
  try {
    const res = await invoke("chat", { run: myRun, turns: chat });
    if (myRun !== runId) return;
    chat.push({ role: "assistant", text: res.text });
    result = res.text;
    resultBackend = res.backend_id;
    setStatus(`✓ ${res.backend} · ${secs()}s`);
  } catch (e) {
    if (myRun !== runId) return;
    unsend();
    error = String(e);
    setStatus("Enter sends it again · M picks another backend", true);
  } finally {
    clearInterval(timer);
    if (myRun === runId) {
      running = false;
      renderResult();
      promptEl.focus();
    }
  }
}
// The message that got no answer goes back into the field, unless something new was typed there
// (then it stays in the chat and is sent along with the next message).
function unsend() {
  if (chat?.at(-1)?.role === "user" && !promptEl.value.trim()) promptEl.value = chat.pop().text;
}
function newChat() {
  if (running) cancelRun();
  chat = [];
  result = error = chatLive = "";
  renderResult();
  setStatus("New chat");
  promptEl.focus();
}
function leaveChat() {
  chat = null;
  result = error = "";
  renderResult();
}

// X: ask the same backend what it changed and why; X again shows the result.
async function explainChange() {
  if (running || !result || chat || resultOutput === "show") return;
  showExplain = !showExplain;
  if (!showExplain || explain) return renderResult();
  const input = `Original:\n${lastInput}\n\nRewritten:\n${result}`;
  if ([...input].length > maxChars) {
    showExplain = false;
    return setStatus(`Too long to explain: original + result exceed general.max_input_chars (${maxChars})`, true);
  }
  const myRun = ++runId;
  running = true;
  resultEl.textContent = "";
  renderResult();
  resultEl.classList.add("streaming");
  setStatus("Asking what changed…");
  try {
    const res = await invoke("run_action", { run: myRun, actionId: "_explain", input, prompt: EXPLAIN, backend: resultBackend });
    if (myRun !== runId) return;
    explain = res.text;
    setStatus("X back to the result" + (explainIn ? ` · ${explainIn}` : ""));
  } catch (e) {
    if (myRun !== runId) return;
    explain = "";
    showExplain = false;
    setStatus(String(e), true);
  } finally {
    if (myRun === runId) {
      running = false;
      renderResult();
    }
  }
}

// U: put back the original of the last replaced text (select that text first).
async function undo() {
  if (running) return;
  try {
    const how = await invoke("undo");
    if (how === "copied") setStatus("Original copied — Ctrl+V to insert");
  } catch (e) {
    setStatus(String(e), true);
  }
}

function cancelRun() {
  runId++;
  running = false;
  invoke("cancel");
  if (chat) {
    unsend();
    error = "";
  } else if (showExplain) {
    showExplain = false; // only the explanation is dropped; the result stays
    explain = "";
  } else {
    result = error = "";
  }
  renderResult();
  setStatus("Cancelled");
}

async function accept() {
  if (!result || running) return;
  try {
    // Only a replaced selection can be undone; with nothing selected the result was inserted.
    const applied = grabbed ? { original: grabbed, action: resultAction, backend: resultBackend || "" } : null;
    const how = await invoke("accept", { result, applied });
    // "pasted"/"typed": the popup is already hidden.
    if (how === "copied") setStatus("Copied — Ctrl+V to insert");
  } catch (e) {
    setStatus(String(e), true); // Rust says whether it reached the clipboard
  }
}

async function copyResult() {
  if (!result || running) return;
  try {
    await invoke("copy", { result });
    setStatus("Copied");
  } catch (e) {
    setStatus(String(e), true);
  }
}

function toggleDiff() {
  const id = resultAction || active;
  store(`diff:${id}`, !diffOn(id));
  if (!running) renderResult();
  setStatus(diffOn(id) ? "Diff on" : "Diff off");
}

// Enter: run when there is nothing (fresh) to replace with, else Replace.
function enter() {
  if (running) return;
  if (result && !dirty) {
    const out = resultOutput;
    if (out === "show") setStatus("Show-only result: C copies it");
    else if (out === "clipboard") copyResult(); // the popup stays open
    else accept();
  } else if (active) run(active);
}

function escape() {
  if (running) return cancelRun();
  runId++;
  invoke("close");
}
$("closeBtn").addEventListener("click", () => { if (running) cancelRun(); runId++; invoke("close"); });

// The one key table, used when focus is not in the textarea or the instruction field.
const KEYS = {
  Enter: enter,
  Escape: escape,
  r: () => active && run(active),
  c: copyResult,
  d: toggleDiff,
  p: () => setPinned(!pinned),
  e: () => original.focus(),
  "/": () => promptEl.focus(),
  u: undo,
  t: cycleTarget,
  k: openContext,
  x: explainChange,
  m: openPicker,
  n: () => chatting() && newChat(),
};

$("promptForm").addEventListener("submit", (e) => {
  e.preventDefault(); // a form, so Enter on a highlighted <datalist> suggestion picks it first
  const fresh = !running && !!result && !dirty;
  if (chatting() || (fresh && resultOutput === "show" && !showExplain)) sendChat();
  else run(CUSTOM, fresh);
});

// Key lookup by character; non-Latin layouts (e.g. Persian) fall back to the physical key.
const NON_ASCII = /[^\x00-\x7f]/;
function lookup(e) {
  const hit = (k) => {
    if (KEYS[k]) return KEYS[k];
    const a = actions.find((x) => x.key && x.key === k);
    if (a) return () => run(a.id);
    const m = modifiers.find((x) => x.key && x.key === k);
    return m && (() => toggleMod(m.id));
  };
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  return hit(e.key) || hit(key) || (NON_ASCII.test(e.key) ? hit(e.code.replace(/^(Key|Digit)/, "").toLowerCase()) : null);
}

document.addEventListener("keydown", (e) => {
  if (e.altKey || e.metaKey || e.isComposing) return;
  const t = document.activeElement;
  if (t.closest("dialog")) return; // the backend picker handles its own keys (Esc closes it)
  if (!chipMenu.hidden) { if (e.key === "Escape") { e.preventDefault(); closeChipMenu(); home(); } return; }
  if (e.ctrlKey) {
    // Zoom: Ctrl+= (or +), Ctrl+-, Ctrl+0; by physical key too, for other layouts.
    const z = e.key === "=" || e.key === "+" || e.code === "Equal" || e.code === "NumpadAdd" ? 0.1
      : e.key === "-" || e.code === "Minus" || e.code === "NumpadSubtract" ? -0.1
      : e.key === "0" || e.code === "Digit0" || e.code === "Numpad0" ? 0 : null;
    if (z !== null) {
      e.preventDefault();
      return setZoom(z ? zoom + z : 1);
    }
    // Ctrl+C copies the whole result unless the user selected something to copy.
    if ((e.key.toLowerCase() === "c" || e.code === "KeyC") && t !== original && t !== promptEl && t !== contextEl && result && !getSelection().toString()) {
      e.preventDefault();
      copyResult();
    }
    return;
  }
  if (t === original || t === promptEl || t === contextEl) {
    if (e.key === "Escape" || (e.key === "Enter" && t === contextEl)) {
      e.preventDefault();
      if (running && e.key === "Escape") return cancelRun();
      if (t === contextEl && !contextEl.value.trim()) contextEl.hidden = true;
      home();
    } else if (e.key === "Enter" && !e.shiftKey && t === original) {
      e.preventDefault();
      enter();
    }
    return; // everything else is typing; Enter in the instruction field submits its form
  }
  // Native Enter: a Tab-focused button (another chip, ★, ×, the backend) clicks itself.
  if (e.key === "Enter" && t.tagName === "BUTTON" && !t.classList.contains("active")) return;
  const f = lookup(e);
  if (f) {
    e.preventDefault();
    f();
  }
});

listen("popup-shown", onShown);
init().then(onShown);
