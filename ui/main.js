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
  if (about) {
    resultName.textContent = "· About";
    resultEl.classList.add("md");
    resultEl.replaceChildren(...md(about));
    return;
  }
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

// Settings (☰ → Settings…, Ctrl+,). A field saves to config.toml when it changes; Rust
// checks that the file still loads before writing it. A refused value stays red with the reason.
const settings = $("settings");
const setPane = $("setPane");
const NEW_PROMPT = "Rewrite this text so that …";
let sv = null; // the last get_settings answer
let setTab = "General";
let settingsOpener = null;

function setMsg(msg, isError = false) {
  const s = $("setStatus");
  s.textContent = msg;
  s.style.color = isError ? "var(--error)" : "";
}
function bad(input, msg) {
  input.classList.add("bad");
  input.closest(".field").querySelector(".err").textContent = msg;
  setMsg(`Not saved: ${msg}`, true);
}

// Run a settings command; on success the page (`rerender`) and the popup show the new config.
async function settingsCall(cmd, args, input, rerender = false) {
  try {
    const v = await invoke(cmd, args);
    sv = Array.isArray(v) ? await invoke("get_settings") : v; // add/remove_action answer with chips
  } catch (e) {
    if (input) bad(input, String(e));
    else setMsg(String(e), true);
    if (!settings.open) setStatus(`Settings: ${e}`, true); // saved on the way out (Esc)
    return false;
  }
  if (input) {
    input.classList.remove("bad");
    input.closest(".field").querySelector(".err").textContent = "";
  }
  setMsg("Saved");
  await refreshPopup().catch(() => {});
  if (rerender) renderSettings();
  return true;
}

const textIn = (value, attrs) => Object.assign(el("input"), { type: "text", spellcheck: false, value: value ?? "" }, attrs);
const numIn = (value, attrs) => Object.assign(el("input"), { type: "number", value }, attrs);
const checkIn = (checked) => Object.assign(el("input"), { type: "checkbox", checked });
function selectIn(value, options) {
  const s = el("select");
  s.append(...options.map(([v, label]) => Object.assign(el("option", "", label), { value: v })));
  s.value = value ?? "";
  return s;
}
function smallBtn(label, title, fn, disabled = false) {
  const b = el("button", "lbtn", label);
  b.type = "button";
  b.title = title;
  b.setAttribute("aria-label", title);
  b.disabled = disabled;
  b.addEventListener("click", fn);
  return b;
}

// A labelled control; `save(value, input)` runs when it changes.
function field(label, input, save, note) {
  const w = el("label", input.type === "checkbox" ? "field check" : "field");
  input.dataset.f ||= label; // focus survives a re-render
  w.append(el("span", "flabel", label), input);
  if (note) w.append(el("small", "note", note));
  w.append(el("small", "err"));
  if (save) {
    input.addEventListener("change", () => save(input.type === "checkbox" ? input.checked
      : input.type === "number" ? input.valueAsNumber : input.value, input));
  }
  return w;
}
const setting = (path) => (value, input) => (typeof value === "number" && !Number.isFinite(value)
  ? bad(input, "Enter a number")
  : settingsCall("set_setting", { path: path.split("."), value }, input));

// Client-side check of a key; Rust checks again (reserved and duplicate keys).
function keyProblem(key, kind, id) {
  const k = key.trim().toLowerCase();
  if (!k) return "";
  if ([...k].length > 1) return "A key is a single character";
  if (sv.reserved.includes(k)) return `"${k}" is used by the popup itself`;
  const a = sv.actions.find((x) => x.key === k && !(kind === "action" && x.id === id));
  if (a) return `"${k}" is taken by the action ${a.name}`;
  const m = sv.modifiers.find((x) => x.key === k && !(kind === "modifier" && x.id === id));
  return m ? `"${k}" is taken by the modifier ${m.name}` : "";
}

// The backends found on this machine (from the picker's last answer), blank = auto.
function backendOptions(current) {
  const opts = [["", "Auto"], ...(view?.found || []).map((b) => [b.id, b.label])];
  if (current && !opts.some(([id]) => id === current)) opts.push([current, `${current} (not found)`]);
  return opts;
}

// One action or modifier: fields save on change; buttons move, reset or remove it.
function entryCard(kind, e, i, list) {
  const isAction = kind === "action";
  const card = el("fieldset", "entry");
  card.append(el("legend", "", e.builtin ? `${e.id} · built in` : e.id));
  const input = (control, k, label) => {
    control.dataset.f = `${kind}:${e.id}:${k}`;
    control.setAttribute("aria-label", `${label} of ${kind} ${e.id}`);
    return control;
  };
  const patch = (k) => (v, control) => {
    const problem = k === "key" && keyProblem(v, kind, e.id);
    if (problem) return bad(control, problem);
    settingsCall(isAction ? "set_action" : "set_modifier", { id: e.id, patch: { [k]: v } }, control);
  };
  const grid = el("div", "entry-grid");
  grid.append(
    field("Name", input(textIn(e.name), "name", "Name"), patch("name")),
    field("Key", input(textIn(e.key, { maxLength: 1 }), "key", "Key"), patch("key")),
  );
  if (isAction) {
    grid.append(
      field("Output", input(selectIn(e.output, [["replace", "Replace"], ["clipboard", "Copy"], ["show", "Show only"]]), "output", "Output"), patch("output")),
      field("Backend", input(selectIn(e.backend || "", backendOptions(e.backend)), "backend", "Backend"), patch("backend")),
      field("Model", input(textIn(e.model, { placeholder: "default" }), "model", "Model"), patch("model")),
    );
  }
  const long = Object.assign(el("textarea"), { value: isAction ? e.prompt : e.suffix, rows: isAction ? 3 : 1, spellcheck: false });
  const btns = el("div", "entry-btns");
  const cmd = (verb) => `${verb}_${kind}`;
  if (isAction) {
    btns.append(
      smallBtn("↑", `Move ${e.name} up`, () => settingsCall("move_action", { id: e.id, delta: -1 }, null, true), i === 0),
      smallBtn("↓", `Move ${e.name} down`, () => settingsCall("move_action", { id: e.id, delta: 1 }, null, true), i === list.length - 1),
    );
  }
  btns.append(e.builtin
    ? smallBtn("Reset to default", `Reset ${e.name} to its default name, key, ${isAction ? "prompt and output" : "and instruction"}`, () => settingsCall(cmd("reset"), { id: e.id }, null, true))
    : smallBtn("Remove", `Remove ${e.name}`, () => settingsCall(cmd("remove"), { id: e.id }, null, true)));
  btns.querySelectorAll("button").forEach((b, n) => (b.dataset.f = `${kind}:${e.id}:btn${n}`));
  card.append(grid, field(isAction ? "Prompt ({text} = the selection)" : "Instruction added to the run",
    input(long, isAction ? "prompt" : "suffix", isAction ? "Prompt" : "Instruction"), patch(isAction ? "prompt" : "suffix")), btns);
  return card;
}

// Add one, then put the cursor in its prompt (action) or name (modifier).
async function addEntry(cmd, args, selector) {
  if (!(await settingsCall(cmd, args, null, true))) return;
  const f = [...setPane.querySelectorAll(selector)].at(-1);
  f?.focus();
  f?.select();
}

const TABS = {
  General: () => {
    const g = sv.general;
    return [
      field("Default action (Enter)", selectIn(g.default_action, sv.actions.map((a) => [a.id, a.name])), setting("general.default_action")),
      field("Replace the selection automatically", checkIn(g.auto_paste), setting("general.auto_paste"), "Off: Enter copies the result instead"),
      field("Paste by", selectIn(sv.paste, [["clipboard", "Clipboard + Ctrl+V"], ["type", "Typing the text"]]), setting("linux.paste")),
      field("Keep a history (undo, recent instructions)", checkIn(g.history), setting("general.history")),
      field("Desktop notifications (CLI)", checkIn(g.notify), setting("general.notify")),
      field("Explanations in", textIn(g.explain_in, { placeholder: "the language of the text" }), setting("general.explain_in"), "Language of Explain and X, e.g. fa or German"),
    ];
  },
  Actions: () => [
    el("p", "note", "The order here is the order of the chips. Built-in actions can be reset, your own removed."),
    smallBtn("+ Add action", "Add an action", () => addEntry("add_action", { prompt: NEW_PROMPT }, ".entry textarea")),
    ...sv.actions.map((a, i, list) => entryCard("action", a, i, list)),
  ],
  Modifiers: () => [
    el("p", "note", "Toggles under the chips: the instruction is added to the next run."),
    smallBtn("+ Add modifier", "Add a modifier", () => addEntry("add_modifier", {}, ".entry input[data-f$=':name']")),
    ...sv.modifiers.map((m, i, list) => entryCard("modifier", m, i, list)),
  ],
  Translate: () => {
    const t = sv.translate;
    const targets = t.targets.includes(t.target) ? t.targets : [...t.targets, t.target];
    return [
      field("Translate into", selectIn(t.target, targets.map((x) => [x, x])), setting("translate.target"), "T in the popup switches to the next one"),
      field("Languages", textIn(t.targets.join(", ")), (v, input) => {
        const list = v.split(",").map((x) => x.trim()).filter(Boolean);
        if (!list.length) return bad(input, "List at least one language");
        settingsCall("set_setting", { path: ["translate", "targets"], value: list }, input, true);
      }, "ISO 639-1 codes, comma-separated, e.g. en, de, fa"),
    ];
  },
  Advanced: () => {
    const row = el("div", "set-row");
    row.append(
      smallBtn("Open config.toml", "Open config.toml in your editor", async () => {
        try { setMsg(`Opened ${await invoke("open_config")} — Reload after editing`); }
        catch (e) { setMsg(String(e), true); }
      }),
      smallBtn("Reload", "Read config.toml again", async () => {
        try { await reloadConfig(); sv = await invoke("get_settings"); renderSettings(); setMsg("Reloaded"); }
        catch (e) { setMsg(String(e), true); }
      }),
    );
    return [
      field("Max tokens per answer", numIn(sv.max_tokens, { min: 1, step: 1 }), setting("provider.max_tokens")),
      field("Temperature", numIn(sv.temperature, { min: 0, max: 2, step: 0.1 }), setting("provider.temperature"), "Only sent to backends that accept it"),
      field("Timeout (seconds)", numIn(sv.timeout_secs, { min: 1, step: 1 }), setting("provider.timeout_secs")),
      field("Longest input (characters)", numIn(sv.general.max_input_chars, { min: 1, step: 1 }), setting("general.max_input_chars")),
      field("Global hotkey", textIn(sv.general.hotkey, { placeholder: "e.g. Ctrl+Space" }), setting("general.hotkey"),
        "X11 only, used after a restart. On Wayland bind mirzabenevis --popup in your desktop's keyboard settings"),
      field("Config file", textIn(sv.config_path, { readOnly: true })),
      row,
    ];
  },
};

function renderSettings() {
  const f = document.activeElement?.dataset?.f;
  $("setTabs").replaceChildren(...Object.keys(TABS).map((name) => {
    const b = el("button", "set-tab", name);
    b.type = "button";
    b.dataset.f = `tab:${name}`;
    b.setAttribute("role", "tab");
    b.setAttribute("aria-selected", String(name === setTab));
    b.tabIndex = name === setTab ? 0 : -1;
    b.addEventListener("click", () => {
      setTab = name;
      renderSettings();
    });
    return b;
  }));
  setPane.setAttribute("aria-label", setTab);
  setPane.replaceChildren(...TABS[setTab]());
  setPane.scrollTop = f?.startsWith("tab:") ? 0 : setPane.scrollTop;
  // A button that became disabled (↑ at the top) hands focus to its neighbour.
  const back = f && settings.querySelector(`[data-f="${CSS.escape(f)}"]`);
  (back?.disabled ? back.parentElement.querySelector("button:not(:disabled)") : back)?.focus();
}
$("setTabs").addEventListener("keydown", (e) => {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  e.preventDefault();
  const names = Object.keys(TABS);
  setTab = names[(names.indexOf(setTab) + (e.key === "ArrowDown" ? 1 : -1) + names.length) % names.length];
  renderSettings();
  settings.querySelector(`[data-f="tab:${setTab}"]`).focus();
});

async function openSettings() {
  if (settings.open) return;
  try {
    sv = await invoke("get_settings");
  } catch (e) {
    return setStatus(String(e), true);
  }
  settingsOpener = document.activeElement;
  setMsg("");
  renderSettings();
  settings.showModal();
  settings.querySelector(`[data-f="tab:${setTab}"]`).focus();
}
// Esc: save what is being typed (change fires on blur) before the dialog goes.
settings.addEventListener("cancel", () => document.activeElement?.blur());
settings.addEventListener("close", () => (settingsOpener?.isConnected && settingsOpener !== document.body ? settingsOpener.focus() : home()));
settings.addEventListener("click", (e) => e.target === settings && settings.close()); // the backdrop

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
const menuItem = (label, fn, cls) => {
  const b = el("button", cls || "", label);
  b.type = "button";
  b.setAttribute("role", "menuitem");
  b.addEventListener("click", () => { closeChipMenu(); fn(); });
  return b;
};
function openChipMenu(a, x, y) {
  const items = [
    menuItem("Edit prompt in the field", () => promptToField(a)),
    menuItem("Copy prompt to clipboard", async () => { await invoke("copy", { result: promptOf(a.id) }); setStatus(`Copied the prompt of "${a.name}"`); }),
    menuItem(`Run "${a.name}"`, () => run(a.id)),
  ];
  if (!a.builtin) items.push(el("div", "sep"), menuItem(`Remove "${a.name}"`, () => removeAction(a), "danger"));
  openMenu(items, x, y);
}
// App menu (☰): about, settings, config, quit.
let about = ""; // Markdown shown in the result pane until the next run / open
async function showAbout() {
  const i = await invoke("app_info");
  about = [
    `## mirzabenevis ${i.version} · میرزا بنویس`,
    "Select text anywhere, press a key, get it fixed, rewritten or translated.",
    "",
    `- Config: \`${i.config_path}\``,
    "- Help: `mirzabenevis-cli doctor` checks your desktop setup",
    "",
    "Keys: numbers run actions · / instruction · D diff · X what changed · U undo · M backend · P pin · Ctrl+, settings · Esc close",
  ].join("\n");
  renderResult();
  // Clickable links (the Markdown renderer deliberately has none).
  const links = el("p", "links");
  for (const [label, url] of [["GitHub: ehsanirani/mirzabenevis", "https://github.com/ehsanirani/mirzabenevis"], ["Report a problem", "https://github.com/ehsanirani/mirzabenevis/issues"]]) {
    const b = el("button", "linkish", label);
    b.type = "button";
    b.title = url;
    b.addEventListener("click", () => invoke("open_url", { url }).catch((e) => setStatus(String(e), true)));
    links.append(b);
  }
  resultEl.append(links);
}
function openAppMenu(x, y) {
  openMenu([
    menuItem("About", showAbout),
    menuItem("Settings…", openSettings),
    menuItem("Open config.toml", async () => {
      try { setStatus(`Opened ${await invoke("open_config")} — Reload config after editing`); }
      catch (e) { setStatus(String(e), true); }
    }),
    menuItem("Reload config", async () => {
      try { await reloadConfig(); setStatus("Config reloaded"); }
      catch (e) { setStatus(String(e), true); }
    }),
    el("div", "sep"),
    menuItem("Quit mirzabenevis", () => invoke("quit"), "danger"),
  ], x, y);
}
$("menuBtn").addEventListener("click", (e) => {
  const r = e.currentTarget.getBoundingClientRect();
  openAppMenu(r.right - 200, r.bottom + 4);
});
// Paste / Clear for the original text.
$("clearBtn").addEventListener("click", () => { leaveChatIfNeeded(); original.value = ""; dirty = true; grow(); original.focus(); setStatus("Cleared"); });
$("pasteBtn").addEventListener("click", async () => {
  try {
    const t = await invoke("read_clipboard");
    if (!t) return setStatus("Clipboard is empty", true);
    leaveChatIfNeeded();
    original.value = t; dirty = true; grow(); original.focus();
    setStatus("Pasted from clipboard — press a number to run");
  } catch (e) { setStatus(String(e), true); }
});
const leaveChatIfNeeded = () => { if (chat) leaveChat(); };
function openMenu(items, x, y) {
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

// What get_info says about the config.
function applyInfo(info) {
  defaultAction = info.default_action;
  explainIn = info.explain_in;
  pasteHow = info.paste;
  target = info.target;
  maxChars = info.max_input_chars;
  modifiers = info.modifiers;
}
// Chips, modifiers and the translate label follow a changed config.
async function refreshPopup() {
  actions = await invoke("get_actions");
  applyInfo(await invoke("get_info"));
  if (!actions.some((a) => a.id === active)) active = defaultAction;
  renderActions();
  renderModifiers();
  if (!running) renderResult();
}
async function reloadConfig() {
  await invoke("reload_config");
  await refreshPopup();
  loadBackends(true);
}

async function onShown() {
  if (picker.open) picker.close();
  if (settings.open) settings.close();
  const shown = ++runId; // forget whatever was running when the popup was last open
  running = false;
  about = "";
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
  applyInfo(await invoke("get_info"));
  active = defaultAction;
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
  about = "";
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
  about = "";
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
  if (t.closest("dialog") || picker.open || settings.open) return; // the backend picker and settings handle their own keys (Esc closes them)
  if (!chipMenu.hidden) { if (e.key === "Escape") { e.preventDefault(); closeChipMenu(); home(); } return; }
  if (e.ctrlKey) {
    if (e.key === "," || e.code === "Comma") {
      e.preventDefault();
      return openSettings();
    }
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
