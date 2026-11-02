const { invoke } = window.__TAURI__.core;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const root = document.documentElement;
const widget = document.querySelector("#widget");
const label = document.querySelector("#label");
const count = document.querySelector("#count");
const lights = [...document.querySelectorAll(".light")];
const lightsBox = document.querySelector(".lights");
const info = document.querySelector("#info");
const bubble = document.querySelector("#bubble");
const tail = document.querySelector("#tail");
const sessionList = document.querySelector("#sessions");

const GAP = 10; // must match TAIL_GAP in src-tauri/src/main.rs
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

let prefs = null;
let prefsKey = "";
let popupOpen = false;
let sessionsKey = "";

// ---- dragging ----------------------------------------------------------

// mousedown's `detail` is the click count, so the second press of a double click is
// seen before any drag starts.
widget.addEventListener("mousedown", (event) => {
  if (event.button !== 0 || event.target.closest(".info")) return;
  if (event.detail >= 2) {
    event.preventDefault();
    void invoke("toggle_orientation")
      .then((next) => {
        prefsKey = JSON.stringify(next);
        applyPrefs(next);
      })
      .catch(console.error);
    return;
  }
  // Resizing the window mid-drag cancels the drag, so a click on the widget only closes the popup.
  if (popupOpen) return;
  void appWindow.startDragging().catch(console.error);
});

// ---- theme + scale -----------------------------------------------------

function resolveTheme() {
  const osDark = darkQuery.matches;
  switch (prefs?.theme) {
    case "dark": return "dark";
    case "light": return "light";
    case "contrast_os": return osDark ? "light" : "dark";
    default: return osDark ? "dark" : "light";
  }
}

function applyPrefs(next) {
  prefs = next;
  root.dataset.orientation = next.orientation;
  root.style.setProperty("--s", String(next.size));
  root.style.setProperty("--t", String(next.text_size));
  root.style.setProperty("--p", String(next.popup_size / 100));
  root.dataset.theme = resolveTheme();
  requestLayout();
}

darkQuery.addEventListener("change", () => {
  if (prefs) root.dataset.theme = resolveTheme();
});

// ---- layout ------------------------------------------------------------
// The window is the only surface, so it is resized to fit the widget and, when open, the bubble.
// Rust decides placement (below/above, horizontal shift) and returns where the widget now sits.

let layoutRunning = false;
let layoutQueued = false;

function requestLayout() {
  if (layoutRunning) {
    layoutQueued = true;
    return;
  }
  layoutRunning = true;
  void layout()
    .catch(console.error)
    .finally(() => {
      layoutRunning = false;
      if (layoutQueued) {
        layoutQueued = false;
        requestLayout();
      }
    });
}

async function layout() {
  const widgetBox = widget.getBoundingClientRect();
  const widgetW = Math.ceil(widgetBox.width);
  const widgetH = Math.ceil(widgetBox.height);

  let popupW = 0;
  let popupH = 0;
  if (popupOpen) {
    bubble.style.visibility = "hidden";
    bubble.hidden = false;
    popupW = Math.ceil(bubble.offsetWidth);
    popupH = Math.ceil(bubble.offsetHeight);
  }

  const placement = await invoke("set_layout", { widgetW, widgetH, popupW, popupH });

  widget.style.left = `${placement.offset_x}px`;
  widget.style.top = `${placement.offset_y}px`;

  if (!popupOpen) {
    bubble.hidden = true;
    bubble.style.visibility = "";
    return;
  }

  bubble.classList.toggle("above", placement.popup_above);
  bubble.style.top = placement.popup_above ? "0px" : `${widgetH + GAP}px`;

  // Point the tail at the centre of the ⓘ dot.
  const dot = info.getBoundingClientRect();
  const dotCenter = dot.left + dot.width / 2;
  const inset = 16;
  tail.style.left = `${Math.min(Math.max(dotCenter, inset), popupW - inset)}px`;
  bubble.style.visibility = "visible";
}

new ResizeObserver(requestLayout).observe(widget);

// ---- popup -------------------------------------------------------------

function basename(path) {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : path || "(unknown)";
}

function ago(seconds) {
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.floor(minutes / 60)}h ago`;
}

const statusText = { idle: "Idle", working: "Working", waiting_permission: "Needs permission" };

function renderSessions(rows) {
  sessionList.replaceChildren();
  if (rows.length === 0) {
    const item = document.createElement("li");
    item.className = "empty";
    item.textContent = "No sessions";
    sessionList.append(item);
    return;
  }
  for (const row of rows) {
    const item = document.createElement("li");
    const dot = document.createElement("span");
    dot.className = `dot ${row.status}`;
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = basename(row.cwd);
    name.title = row.cwd;
    const detail = document.createElement("span");
    detail.className = "detail";
    const bits = [statusText[row.status] ?? row.status];
    if (row.active_tool) bits.push(row.active_tool);
    if (row.model) bits.push(row.effort ? `${row.model} · ${row.effort}` : row.model);
    if (row.subagent_count) bits.push(`${row.subagent_count} sub-agent${row.subagent_count > 1 ? "s" : ""}`);
    bits.push(ago(row.age_secs));
    detail.textContent = bits.join(" · ");
    item.append(dot, name, detail);
    sessionList.append(item);
  }
}

async function refreshSessions() {
  const rows = await invoke("get_sessions");
  const key = JSON.stringify(rows);
  if (key === sessionsKey) return;
  sessionsKey = key;
  renderSessions(rows);
  requestLayout();
}

function setPopup(open) {
  if (popupOpen === open) return;
  popupOpen = open;
  info.setAttribute("aria-expanded", String(open));
  if (open) {
    sessionsKey = "";
    void refreshSessions().catch(console.error);
  }
  requestLayout();
}

info.addEventListener("click", () => setPopup(!popupOpen));
document.addEventListener("pointerdown", (event) => {
  if (popupOpen && !bubble.contains(event.target) && !info.contains(event.target)) setPopup(false);
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") setPopup(false);
});

// ---- light state -------------------------------------------------------

const labels = {
  dark: "No sessions",
  green: "Idle",
  yellow: "Working",
  red: "Permission",
};

function render(state) {
  for (const light of lights) {
    light.classList.toggle("active", light.classList.contains(state.light));
  }
  label.textContent = labels[state.light] ?? "Unavailable";
  count.textContent = `${state.active_session_count} active`;
  lightsBox.setAttribute("aria-label", labels[state.light] ?? "Unavailable");
}

async function refresh() {
  try {
    const [state, nextPrefs] = await Promise.all([invoke("get_light_state"), invoke("get_prefs")]);
    const key = JSON.stringify(nextPrefs);
    if (key !== prefsKey) {
      prefsKey = key;
      applyPrefs(nextPrefs);
    }
    render(state);
    if (popupOpen) await refreshSessions();
  } catch (error) {
    label.textContent = "Unavailable";
    count.textContent = "state read failed";
    console.error(error);
  }
}

void refresh();
window.setInterval(refresh, 400);
