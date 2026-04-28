// DevOpster Desktop — multi-screen renderer.
// Talks to the Rust backend via window.__TAURI__.core.invoke and listens for streamed output.

const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke ?? (() => Promise.reject(new Error("Tauri bridge unavailable")));
const listen = tauri?.event?.listen ?? (async () => () => {});

const view = document.getElementById("view");
const navContainer = document.getElementById("nav");
const routeTitle = document.getElementById("route-title");
const routeSub = document.getElementById("route-sub");
const topbarActions = document.getElementById("topbar-actions");
const toastEl = document.getElementById("toast");
const cliDrawer = document.getElementById("cli-drawer");
const cliInput = document.getElementById("cli-input");
const cliRun = document.getElementById("cli-run");
const cliClear = document.getElementById("cli-clear");
const cliClose = document.getElementById("cli-close");
const cliOutput = document.getElementById("cli-output");

let env = null;
let currentRoute = "dashboard";
let consoleBuffer = [];

// ───── helpers ─────────────────────────────────────────────────────
function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "class") node.className = v;
    else if (k === "html") node.innerHTML = v;
    else if (k === "text") node.textContent = v;
    else if (k.startsWith("on") && typeof v === "function") node.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null) node.setAttribute(k, v);
  }
  for (const child of [].concat(children)) {
    if (child == null) continue;
    node.appendChild(typeof child === "string" ? document.createTextNode(child) : child);
  }
  return node;
}

function clear(node) { while (node.firstChild) node.removeChild(node.firstChild); }

function toast(msg, kind = "") {
  toastEl.textContent = msg;
  toastEl.className = `toast show ${kind}`;
  clearTimeout(toast._t);
  toast._t = setTimeout(() => { toastEl.className = "toast"; }, 3200);
}

function setActions(buttons) {
  clear(topbarActions);
  if (!setActions._cliBtn) {
    setActions._cliBtn = actionBtn("CLI Panel", () => toggleCliDrawer());
  }
  topbarActions.appendChild(setActions._cliBtn);
  for (const b of buttons) topbarActions.appendChild(b);
}

function setHeader(title, sub) {
  routeTitle.textContent = title;
  routeSub.textContent = sub;
}

function loading(text = "Loading…") {
  return el("div", { class: "card" }, [
    el("span", { class: "spinner" }),
    text,
  ]);
}

function emptyState(text) {
  return el("div", { class: "empty" }, text);
}

function renderCliOutput() {
  if (!cliOutput) return;
  cliOutput.textContent = consoleBuffer.join("") || "Ready.\n";
  cliOutput.scrollTop = cliOutput.scrollHeight;
}

function setCliOpen(open) {
  document.body.classList.toggle("cli-open", open);
  if (cliDrawer) cliDrawer.setAttribute("aria-hidden", open ? "false" : "true");
  if (open && cliInput) cliInput.focus();
  if (open) renderCliOutput();
}

function toggleCliDrawer(force) {
  const isOpen = document.body.classList.contains("cli-open");
  setCliOpen(typeof force === "boolean" ? force : !isOpen);
}

// ───── routes ─────────────────────────────────────────────────────
const routes = {
  dashboard: renderDashboard,
  diagnostics: renderDiagnostics,
  inventory: renderInventory,
  audit: renderAudit,
  stats: renderStats,
  catalog: renderCatalog,
  topics: renderTopics,
  setup: renderSetup,
  config: renderConfig,
  console: renderConsole,
};

function navigate(route) {
  if (!routes[route]) route = "dashboard";
  currentRoute = route;
  for (const b of navContainer.querySelectorAll(".nav")) {
    b.classList.toggle("active", b.dataset.route === route);
  }
  setActions([]);
  routes[route]();
}

navContainer.addEventListener("click", (e) => {
  const btn = e.target.closest(".nav");
  if (btn) navigate(btn.dataset.route);
});

// ───── DASHBOARD ──────────────────────────────────────────────────
async function renderDashboard() {
  setHeader("Dashboard", "Overview of your DevOpster environment.");
  clear(view);

  const wrap = el("div", { class: "grid grid-3" });
  wrap.appendChild(card("Sidecar", env?.sidecar_exists ? "ready" : "missing",
    env?.sidecar_exists ? "ok" : "warn",
    env?.sidecar_path || ""));
  wrap.appendChild(card("Config", env?.config_exists ? "found" : "missing",
    env?.config_exists ? "ok" : "warn",
    env?.config_path || ""));
  wrap.appendChild(card("Platform", `${env?.os || ""} · ${env?.arch || ""}`, "info", "Native build"));
  view.appendChild(wrap);

  view.appendChild(el("div", { class: "card", style: "margin-top:18px" }, [
    el("h3", { text: "Quick actions" }),
    el("div", { style: "display:flex;gap:8px;flex-wrap:wrap;margin-top:10px" }, [
      actionBtn("Run diagnostics", () => navigate("diagnostics")),
      actionBtn("Open inventory", () => navigate("inventory")),
      actionBtn("Audit repos", () => navigate("audit")),
      actionBtn("Edit config", () => navigate("config")),
      actionBtn("Open CLI mode", () => navigate("console")),
    ]),
  ]));

  view.appendChild(el("div", { class: "card", style: "margin-top:18px" }, [
    el("h3", { text: "About" }),
    el("p", { text: "DevOpster is an open-source DevOps control plane. The desktop app wraps the same Rust CLI you can use from a terminal — every screen here calls a real devopster command." }),
  ]));
}

function card(title, value, pillKind, sub) {
  return el("div", { class: "card" }, [
    el("div", { class: "kicker", text: title }),
    el("div", { class: "num", text: value }),
    el("div", { style: "margin-top:8px" }, [
      el("span", { class: `pill pill-${pillKind}`, text: value }),
    ]),
    sub ? el("p", { style: "margin-top:8px;font-size:0.78rem", text: sub }) : null,
  ]);
}
function actionBtn(label, fn) {
  const b = el("button", { class: "btn", text: label });
  b.addEventListener("click", fn);
  return b;
}

// ───── DIAGNOSTICS ────────────────────────────────────────────────
async function renderDiagnostics() {
  setHeader("Diagnostics", "Check Docker and provider CLI tooling readiness.");
  clear(view);
  view.appendChild(loading("Running devopster diagnostics…"));

  try {
    const res = await invoke("run_devopster", { args: ["diagnostics"] });
    clear(view);

    const ok = res.status === 0;
    view.appendChild(el("div", { class: "card" }, [
      el("h3", {}, [
        ok ? el("span", { class: "pill pill-ok", text: "PASS" }) : el("span", { class: "pill pill-err", text: "FAIL" }),
        " ",
        document.createTextNode(ok ? "Environment is ready." : "Issues detected — see output below."),
      ]),
      el("p", { text: `exit ${res.status}` }),
    ]));
    view.appendChild(el("div", { class: "console", style: "margin-top:14px;height:auto;min-height:180px", text: (res.stdout || "") + (res.stderr ? "\n[stderr]\n" + res.stderr : "") }));

    setActions([
      actionBtn("Re-run", renderDiagnostics),
      actionBtn("Open CLI mode", () => setCliOpen(true)),
    ]);
  } catch (err) {
    clear(view);
    view.appendChild(errCard("Diagnostics failed", String(err)));
  }
}

function errCard(title, msg) {
  return el("div", { class: "card" }, [
    el("h3", {}, [el("span", { class: "pill pill-err", text: "Error" })]),
    el("p", { style: "margin-top:6px", text: title }),
    el("pre", { class: "console", style: "margin-top:10px;height:auto;min-height:120px", text: msg }),
  ]);
}

// ───── INVENTORY ──────────────────────────────────────────────────
let inventoryCache = null;
let inventoryFilter = "";
let inventoryProvider = "all";

async function renderInventory() {
  setHeader("Inventory", "Repositories from every provider in your config.");
  clear(view);

  const refreshBtn = actionBtn("Refresh", async () => { inventoryCache = null; await renderInventory(); });
  setActions([refreshBtn]);

  if (!inventoryCache) {
    view.appendChild(loading("Loading inventory from devopster…"));
    try {
      inventoryCache = await invoke("run_devopster_json", { args: ["inventory", "--json"] });
    } catch (err) {
      clear(view);
      view.appendChild(errCard("Could not load inventory", String(err)));
      return;
    }
  }

  clear(view);
  const entries = Array.isArray(inventoryCache) ? inventoryCache : [];
  if (entries.length === 0) {
    view.appendChild(emptyState("No repositories returned. Check your devopster-config.yaml in Settings."));
    return;
  }

  const providers = ["all", ...Array.from(new Set(entries.map((e) => e.provider))).sort()];
  const toolbar = el("div", { class: "toolbar" }, [
    el("input", {
      class: "input", placeholder: "Filter by name…", value: inventoryFilter,
      oninput: (e) => { inventoryFilter = e.target.value.toLowerCase(); renderTable(); },
    }),
    (() => {
      const s = el("select", { class: "input" });
      providers.forEach((p) => s.appendChild(el("option", { value: p, text: p })));
      s.value = inventoryProvider;
      s.addEventListener("change", () => { inventoryProvider = s.value; renderTable(); });
      return s;
    })(),
    el("div", { class: "spacer" }),
    el("div", { class: "muted", id: "inv-count" }),
  ]);
  view.appendChild(toolbar);

  const tableWrap = el("div");
  view.appendChild(tableWrap);

  function renderTable() {
    const filtered = entries.filter((e) => {
      if (inventoryProvider !== "all" && e.provider !== inventoryProvider) return false;
      if (!inventoryFilter) return true;
      return (e.repository.name || "").toLowerCase().includes(inventoryFilter)
          || (e.repository.description || "").toLowerCase().includes(inventoryFilter);
    });
    document.getElementById("inv-count").textContent = `${filtered.length} of ${entries.length}`;
    clear(tableWrap);
    if (filtered.length === 0) { tableWrap.appendChild(emptyState("No matches.")); return; }

    const table = el("table", { class: "data" });
    table.appendChild(el("thead", {}, [el("tr", {}, [
      el("th", { text: "Provider" }),
      el("th", { text: "Org" }),
      el("th", { text: "Repository" }),
      el("th", { text: "Visibility" }),
      el("th", { text: "Branch" }),
      el("th", { text: "Topics" }),
      el("th", { text: "Description" }),
    ])]));
    const tb = el("tbody");
    for (const e of filtered) {
      const r = e.repository;
      tb.appendChild(el("tr", {}, [
        el("td", { text: e.provider }),
        el("td", { text: e.organization }),
        el("td", { text: r.name }),
        el("td", {}, [el("span", { class: `pill ${r.is_private ? "pill-warn" : "pill-info"}`, text: r.is_private ? "private" : "public" })]),
        el("td", { text: r.default_branch || "—" }),
        el("td", { text: String((r.topics || []).length) }),
        el("td", { text: r.description?.trim() || "—" }),
      ]));
    }
    table.appendChild(tb);
    tableWrap.appendChild(table);
  }
  renderTable();
}

// ───── REPO AUDIT ─────────────────────────────────────────────────
async function renderAudit() {
  setHeader("Repository audit", "Run policy checks across every targeted repository.");
  clear(view);

  view.appendChild(el("div", { class: "card" }, [
    el("h3", { text: "Run an audit" }),
    el("p", { text: "Audits enforce description, topics, license, and default-branch policy. Issues are listed below; fixes can be scheduled from the CLI." }),
    el("div", { style: "margin-top:12px;display:flex;gap:8px;flex-wrap:wrap" }, [
      btn("Run audit", "btn-primary", () => doAudit(false)),
      btn("Run audit + report only", "btn", () => doAudit(true)),
    ]),
  ]));
  const out = el("div", { id: "audit-out", style: "margin-top:14px" });
  view.appendChild(out);

  async function doAudit(reportOnly) {
    clear(out);
    out.appendChild(loading("Running devopster repo audit…"));
    try {
      const args = ["repo", "audit"];
      if (reportOnly) args.push("--report-only");
      const res = await invoke("run_devopster", { args });
      clear(out);
      out.appendChild(el("div", { class: "card" }, [
        el("h3", {}, [
          res.status === 0 ? el("span", { class: "pill pill-ok", text: "PASS" }) : el("span", { class: "pill pill-warn", text: "ISSUES" }),
          " ", document.createTextNode(`exit ${res.status}`),
        ]),
        el("div", { class: "console", style: "margin-top:10px;height:auto;min-height:240px", text: (res.stdout || "") + (res.stderr ? "\n[stderr]\n" + res.stderr : "") }),
      ]));
    } catch (err) {
      clear(out);
      out.appendChild(errCard("Audit failed", String(err)));
    }
  }
}

function btn(label, cls, fn) {
  const b = el("button", { class: `btn ${cls}`, text: label });
  b.addEventListener("click", fn);
  return b;
}

// ───── STATS ──────────────────────────────────────────────────────
async function renderStats() {
  setHeader("Stats", "Org-wide metadata coverage and compliance.");
  clear(view);
  view.appendChild(loading("Computing stats…"));
  try {
    const res = await invoke("run_devopster", { args: ["stats"] });
    clear(view);
    view.appendChild(el("div", { class: "card" }, [
      el("h3", { text: "Latest stats" }),
      el("p", { text: `exit ${res.status}` }),
      el("div", { class: "console", style: "margin-top:10px;height:auto;min-height:260px", text: (res.stdout || "") + (res.stderr ? "\n[stderr]\n" + res.stderr : "") }),
    ]));
    setActions([actionBtn("Re-run", renderStats)]);
  } catch (err) {
    clear(view);
    view.appendChild(errCard("Stats failed", String(err)));
  }
}

// ───── CATALOG ────────────────────────────────────────────────────
async function renderCatalog() {
  setHeader("Catalog", "Generate a machine-readable catalog.json for your org.");
  clear(view);
  view.appendChild(el("div", { class: "card" }, [
    el("h3", { text: "Generate catalog" }),
    el("p", { text: "Outputs catalog.json in the working directory. Open the file from your shell after generation." }),
    el("div", { style: "margin-top:12px" }, [
      btn("Generate", "btn-primary", async () => {
        const out = document.getElementById("catalog-out");
        clear(out); out.appendChild(loading("Running devopster catalog…"));
        try {
          const res = await invoke("run_devopster", { args: ["catalog"] });
          clear(out);
          out.appendChild(el("div", { class: "console", style: "height:auto;min-height:200px", text: (res.stdout || "") + (res.stderr ? "\n[stderr]\n" + res.stderr : "") }));
          toast(res.status === 0 ? "Catalog generated." : "Catalog finished with warnings.", res.status === 0 ? "ok" : "err");
        } catch (err) {
          clear(out); out.appendChild(errCard("Catalog failed", String(err)));
        }
      }),
    ]),
  ]));
  view.appendChild(el("div", { id: "catalog-out", style: "margin-top:14px" }));
}

// ───── TOPICS ─────────────────────────────────────────────────────
async function renderTopics() {
  setHeader("Topics", "Apply missing template topics across repositories.");
  clear(view);
  view.appendChild(el("div", { class: "card" }, [
    el("h3", { text: "Align topics" }),
    el("p", { text: "Adds any required topics from your config to repositories that don't already have them." }),
    el("div", { style: "margin-top:12px" }, [
      btn("Run alignment", "btn-primary", async () => {
        const out = document.getElementById("topics-out");
        clear(out); out.appendChild(loading("Running devopster topics…"));
        try {
          const res = await invoke("run_devopster", { args: ["topics"] });
          clear(out);
          out.appendChild(el("div", { class: "console", style: "height:auto;min-height:200px", text: (res.stdout || "") + (res.stderr ? "\n[stderr]\n" + res.stderr : "") }));
        } catch (err) {
          clear(out); out.appendChild(errCard("Topic alignment failed", String(err)));
        }
      }),
    ]),
  ]));
  view.appendChild(el("div", { id: "topics-out", style: "margin-top:14px" }));
}

// ───── SETUP ──────────────────────────────────────────────────────
async function renderSetup() {
  setHeader("Setup", "One-command developer setup (login + guided config).");
  clear(view);
  view.appendChild(el("div", { class: "card" }, [
    el("h3", { text: "Run guided setup" }),
    el("p", { text: "Prompts will appear in the streamed output. Setup configures providers and writes devopster-config.yaml." }),
    el("div", { style: "margin-top:12px" }, [
      btn("Start setup", "btn-primary", () => navigate("console") || streamCmd(["setup"], "Setup")),
    ]),
  ]));
}

// ───── CONFIG editor ──────────────────────────────────────────────
async function renderConfig() {
  setHeader("Config", `Edit ${env?.config_path || "devopster-config.yaml"}.`);
  clear(view);
  view.appendChild(loading("Reading config…"));

  let text = "";
  try { text = await invoke("read_config", { path: env?.config_path || "devopster-config.yaml" }); }
  catch (err) {
    text = `# Config not yet present.\n# Generate one from the Setup screen, or paste a starter template here and Save.\n`;
  }

  clear(view);
  const ta = el("textarea", { class: "textarea", spellcheck: "false" });
  ta.value = text;
  view.appendChild(el("div", { class: "card" }, [
    el("h3", { text: "devopster-config.yaml" }),
    el("p", { text: env?.config_path || "" }),
    el("div", { style: "margin-top:12px" }, [ta]),
    el("div", { style: "margin-top:10px;display:flex;gap:8px;flex-wrap:wrap" }, [
      btn("Save", "btn-primary", async () => {
        try {
          await invoke("write_config", { path: env?.config_path, contents: ta.value });
          toast("Config saved.", "ok");
        } catch (err) { toast(String(err), "err"); }
      }),
      btn("Reload", "btn", renderConfig),
    ]),
  ]));
}

// ───── CONSOLE (live streaming arbitrary commands) ────────────────
async function renderConsole() {
  setHeader("CLI Mode", "Run devopster commands without leaving the app.");
  clear(view);
  view.appendChild(el("div", { class: "card" }, [
    el("h3", { text: "CLI Mode" }),
    el("p", { text: "The CLI panel is always available at the bottom of the app. Use it to run any allow-listed devopster command." }),
    el("div", { style: "margin-top:12px;display:flex;gap:8px;flex-wrap:wrap" }, [
      btn("Open CLI panel", "btn-primary", () => setCliOpen(true)),
      btn("Clear output", "btn", () => { consoleBuffer = []; renderCliOutput(); }),
    ]),
  ]));
  setCliOpen(true);
}

function parseArgs(raw) {
  const out = [];
  const re = /"([^"]*)"|'([^']*)'|(\S+)/g;
  let m;
  while ((m = re.exec(raw)) !== null) out.push(m[1] ?? m[2] ?? m[3]);
  return out;
}

async function streamCmd(args, label) {
  // ensure subscriptions exist exactly once
  if (!streamCmd._subscribed) {
    streamCmd._subscribed = true;
    await listen("devopster:stdout", (ev) => {
      consoleBuffer.push(ev.payload + "\n");
      renderCliOutput();
    });
    await listen("devopster:stderr", (ev) => {
      consoleBuffer.push(`[err] ${ev.payload}\n`);
      renderCliOutput();
    });
  }
  return await invoke("stream_devopster", { args });
}

async function runCliFromDrawer() {
  const raw = (cliInput?.value || "").trim();
  if (!raw) return;
  const args = parseArgs(raw);
  consoleBuffer.push(`\n$ devopster ${args.join(" ")}\n`);
  renderCliOutput();
  if (cliRun) cliRun.disabled = true;
  try {
    const code = await streamCmd(args, raw);
    consoleBuffer.push(`\n[exit ${code}]\n`);
  } catch (err) {
    consoleBuffer.push(`\n[error] ${err}\n`);
  } finally {
    if (cliRun) cliRun.disabled = false;
    renderCliOutput();
  }
}

if (cliRun) cliRun.addEventListener("click", runCliFromDrawer);
if (cliInput) cliInput.addEventListener("keydown", (e) => { if (e.key === "Enter") runCliFromDrawer(); });
if (cliClear) cliClear.addEventListener("click", () => { consoleBuffer = []; renderCliOutput(); });
if (cliClose) cliClose.addEventListener("click", () => setCliOpen(false));

// ───── boot ───────────────────────────────────────────────────────
(async function boot() {
  try {
    env = await invoke("env_info");
  } catch (err) {
    env = { os: "unknown", arch: "unknown", sidecar_path: "", sidecar_exists: false, config_path: "", config_exists: false };
  }
  document.getElementById("env-os").textContent = `${env.os} · ${env.arch}`;
  document.getElementById("env-sidecar").textContent = env.sidecar_exists ? "sidecar: ready" : "sidecar: missing";
  navigate("dashboard");
})();
