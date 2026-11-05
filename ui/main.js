// Popup behaviour. Talks to the Rust side through the global Tauri API
// (withGlobalTauri = true in tauri.conf.json), so no bundler is needed.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const actionsEl = $("actions");
const original = $("original");
const resultPane = $("resultPane");
const resultEl = $("result");
const statusEl = $("status");

let actions = [];
let active = null;      // action id
let result = "";
let running = false;
let unlistenChunk = null;

const setStatus = (msg, isError = false) => {
  statusEl.textContent = msg;
  statusEl.style.color = isError ? "var(--error)" : "";
};

async function init() {
  actions = await invoke("get_actions");
  const info = await invoke("get_info");
  $("model").textContent = info.model;
  active = info.default_action;
  renderActions();
  if (unlistenChunk) unlistenChunk();
  unlistenChunk = await listen("chunk", (e) => {
    result += e.payload;
    resultEl.textContent = result;
    resultPane.scrollTop = resultPane.scrollHeight;
  });
}

function renderActions() {
  actionsEl.innerHTML = "";
  for (const a of actions) {
    const b = document.createElement("button");
    b.className = "action" + (a.id === active ? " active" : "");
    b.type = "button";
    b.dataset.id = a.id;
    b.innerHTML = `<span class="k">${a.key || "·"}</span>${a.name}`;
    b.title = `Run "${a.name}"`;
    b.addEventListener("click", () => run(a.id));
    actionsEl.appendChild(b);
  }
}

async function onShown() {
  result = "";
  resultEl.textContent = "";
  resultEl.classList.remove("error", "streaming");
  resultPane.hidden = true;
  setStatus("");
  try {
    original.value = await invoke("grab");
    setStatus("Press a number to run an action");
  } catch (e) {
    original.value = "";
    setStatus(String(e), true);
  }
  original.focus();
  original.select();
}

async function run(id) {
  if (running) return;
  const input = original.value.trim();
  if (!input) {
    setStatus("Nothing to work on", true);
    return;
  }
  active = id;
  renderActions();
  running = true;
  result = "";
  resultEl.textContent = "";
  resultEl.classList.remove("error");
  resultEl.classList.add("streaming");
  resultPane.hidden = false;
  setStatus("Thinking…");
  try {
    result = await invoke("run_action", { actionId: id, input });
    resultEl.textContent = result;
    setStatus("Enter to paste, Ctrl+C to copy");
    resultEl.focus();
  } catch (e) {
    resultEl.textContent = String(e);
    resultEl.classList.add("error");
    setStatus("Failed", true);
  } finally {
    running = false;
    resultEl.classList.remove("streaming");
  }
}

async function accept() {
  if (!result || running) return;
  try {
    const how = await invoke("accept", { result });
    setStatus(how === "pasted" ? "Pasted" : "Copied to clipboard");
  } catch (e) {
    setStatus(String(e), true);
  }
}

async function copyResult() {
  if (!result) return;
  await invoke("copy", { result });
  setStatus("Copied");
}

document.addEventListener("keydown", (e) => {
  const inTextarea = document.activeElement === original;
  if (e.key === "Escape") {
    e.preventDefault();
    invoke("close");
    return;
  }
  if (e.key === "Enter" && !e.shiftKey) {
    // In the textarea, plain Enter runs the active action; after a result, it accepts.
    e.preventDefault();
    if (result && !running) accept();
    else if (active) run(active);
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c" && result && !inTextarea) {
    e.preventDefault();
    copyResult();
    return;
  }
  if (!inTextarea && !e.ctrlKey && !e.altKey && !e.metaKey) {
    if (e.key.toLowerCase() === "r" && active) { e.preventDefault(); run(active); return; }
    const a = actions.find((x) => x.key && x.key === e.key);
    if (a) { e.preventDefault(); run(a.id); }
  }
});

// Number keys work from inside the textarea too, if the text has not been edited.
original.addEventListener("keydown", (e) => {
  if (e.ctrlKey || e.altKey || e.metaKey) return;
  const a = actions.find((x) => x.key && x.key === e.key);
  if (a && original.selectionStart === 0 && original.selectionEnd === original.value.length) {
    e.preventDefault();
    run(a.id);
  }
});

listen("popup-shown", onShown);
init().then(onShown);
