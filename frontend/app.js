/* MyEyeCare 前端逻辑：无框架、无 npm 依赖，直接通过 Tauri IPC 调用 Rust 命令 */

// 检查更新与更新日志指向同一开源仓库
const UPDATE_REPO = "yaoahos/MyEyeCare";
const APP_VERSION = "1.0.1";
const UPDATE_NOTES_LIMIT = 120; // 弹窗内更新日志最大字符数，超出则截断并给出"查看更多"

const $ = (selector) => document.querySelector(selector);
const invoke = (cmd, args = {}) => {
  const internals = window.__TAURI_INTERNALS__;
  if (internals && typeof internals.invoke === "function") {
    return internals.invoke(cmd, args);
  }
  // 浏览器预览环境无 IPC，静默返回
  return Promise.resolve(undefined);
};

const DEFAULT_OVERLAY_COLOR = "#8FD6BC";
const MAX_CUSTOM = 3;

// 每套模板自带遮罩配置，切换时亮度 / 色温 / 遮罩一起生效。
// 设计原则：按使用场景由日间到深夜递进——色温递减、亮度递减、遮罩递增；
// 色卡取各自场景的代表色（蓝→绿→黄→紫→橙→红），一眼即可区分，不与遮罩色混为一谈。
const TEMPLATES = [
  { id: "standard",  name: "标准日间", temperature: 5800, brightness: 1.00, swatch: ["#d6ecfb", "#8fc6ea"], overlayEnabled: true,  overlayColor: "#8FD6BC", overlayStrength: 5  },
  { id: "focus",     name: "专注工作", temperature: 5300, brightness: 0.94, swatch: ["#d4f0df", "#7fd0a8"], overlayEnabled: true,  overlayColor: "#8FD6BC", overlayStrength: 12 },
  { id: "reading",   name: "阅读模式", temperature: 4800, brightness: 0.88, swatch: ["#fff0c9", "#eccf8c"], overlayEnabled: true,  overlayColor: "#FFD98A", overlayStrength: 18 },
  { id: "movie",     name: "影院模式", temperature: 4200, brightness: 0.70, swatch: ["#eadffa", "#b9a2e6"], overlayEnabled: true,  overlayColor: "#B6A6E8", overlayStrength: 22 },
  { id: "night",     name: "夜间护眼", temperature: 3600, brightness: 0.64, swatch: ["#ffdcc0", "#f2a874"], overlayEnabled: true,  overlayColor: "#FFB060", overlayStrength: 30 },
  { id: "deepnight", name: "深夜模式", temperature: 3000, brightness: 0.50, swatch: ["#f7cfc4", "#dd8b78"], overlayEnabled: true,  overlayColor: "#FF9E80", overlayStrength: 40 },
];

// 柔色遮罩预设（Rust 端按 #RRGGBB 解析为 COLORREF）
const OVERLAY_COLORS = [
  { name: "暖黄", hex: "#FFD98A" },
  { name: "琥珀", hex: "#FFB060" },
  { name: "珊瑚", hex: "#FF9E80" },
  { name: "玫瑰", hex: "#F2A0BE" },
  { name: "薰衣草", hex: "#B6A6E8" },
  { name: "薄荷", hex: "#8FD6BC" },
];

const state = {
  brightness: 1,
  temperature: 5800,
  enabled: true,
  autostartEnabled: true,
  startMinimized: true,
  checkUpdatesOnStartup: true,
  templateId: "standard",
  overlayEnabled: true,
  overlayStrength: 5,
  overlayColor: DEFAULT_OVERLAY_COLOR,
  customTemplates: [],
};

let saveTimer = null;
let toastTimer = null;
let latestRelease = null;

/* ---------- 通用 UI ---------- */
function setSwitch(el, on) { el.setAttribute("aria-checked", on ? "true" : "false"); }
function isSwitchOn(el) { return el.getAttribute("aria-checked") === "true"; }
function toast(message, error = false) {
  const el = $("#toast");
  el.textContent = message;
  el.classList.toggle("error", error);
  el.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove("show"), 2200);
}
function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c]);
}

/* ---------- 状态同步 ---------- */
function allTemplates() { return TEMPLATES.concat(state.customTemplates); }

function matchTemplate() {
  return allTemplates().find((t) =>
    t.temperature === state.temperature
    && Math.abs(t.brightness - state.brightness) < 0.0001
    && Boolean(t.overlayEnabled) === state.overlayEnabled
    && Number(t.overlayStrength) === state.overlayStrength
    && String(t.overlayColor).toUpperCase() === String(state.overlayColor).toUpperCase()
  );
}

// 自定义模板的色卡直接取其遮罩颜色，让色卡真正反映模板效果
function lighten(hex, ratio = 0.62) {
  const m = /^#?([0-9a-f]{6})$/i.exec(String(hex || ""));
  if (!m) return "#eef4ef";
  const n = parseInt(m[1], 16);
  const rgb = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  return `#${rgb.map((c) => Math.round(c + (255 - c) * ratio).toString(16).padStart(2, "0")).join("")}`;
}
function swatchForTemplate(t) {
  // 内置模板用各自独立的场景色，颜色区分明显；自定义模板没有独立色卡时再按遮罩色生成
  if (Array.isArray(t.swatch) && t.swatch.length === 2) return t.swatch;
  if (t.overlayEnabled && t.overlayColor) return [lighten(t.overlayColor), t.overlayColor];
  if (t.temperature <= 3500) return ["#ffe0c2", "#f0b684"];
  if (t.temperature <= 4500) return ["#fff1d6", "#e7c79b"];
  if (t.temperature <= 5500) return ["#f2f6e6", "#cfdcb6"];
  return ["#e6f2fb", "#b8d6ea"];
}

function renderColorSwatches() {
  $("#colorSwatches").innerHTML = OVERLAY_COLORS.map((c) => `
    <button class="color-swatch" data-hex="${c.hex}" style="background:${c.hex}" title="${c.name}"></button>
  `).join("");
}

function syncUI() {
  const brightnessPct = Math.round(state.brightness * 100);
  const bRange = $("#brightnessRange");
  bRange.value = brightnessPct;
  bRange.style.setProperty("--pct", `${((brightnessPct - 30) / 70) * 100}%`);
  $("#brightnessValue").textContent = `${brightnessPct}%`;

  const tRange = $("#temperatureRange");
  tRange.value = state.temperature;
  tRange.style.setProperty("--pct", `${((state.temperature - 2700) / 3800) * 100}%`);
  $("#temperatureValue").textContent = `${state.temperature} K`;

  const matched = matchTemplate();
  state.templateId = matched ? matched.id : "custom";
  setSwitch($("#masterSwitch"), state.enabled);
  $("#masterState").textContent = state.enabled ? "屏幕色彩已优化" : "已恢复原始屏幕";

  // 当前配置只要已存在（内置或自定义模板）就不允许保存，避免重复项
  const saveBtn = $("#btnSaveTemplate");
  saveBtn.disabled = Boolean(matched);
  saveBtn.title = matched ? "当前配置已是模板" : "保存当前设置为模板";

  setSwitch($("#updateCheckSwitch"), state.checkUpdatesOnStartup);
  setSwitch($("#autostartSwitch"), state.autostartEnabled === true);
  setSwitch($("#minimizedSwitch"), state.startMinimized);

  // 柔色遮罩
  setSwitch($("#overlaySwitch"), state.overlayEnabled);
  $("#overlayBody").classList.toggle("disabled", !state.overlayEnabled);
  $("#strengthValue").textContent = `${state.overlayStrength}%`;
  const sRange = $("#strengthRange");
  sRange.value = state.overlayStrength;
  sRange.style.setProperty("--pct", `${state.overlayStrength}%`);
  document.querySelectorAll(".color-swatch").forEach((sw) => {
    sw.classList.toggle("active", sw.dataset.hex === String(state.overlayColor).toUpperCase());
  });

  document.querySelectorAll(".template").forEach((card) => {
    card.classList.toggle("active", card.dataset.id === state.templateId);
  });
}

/* ---------- 模板网格 ---------- */
function templateCardHtml(t, custom) {
  const swatch = swatchForTemplate(t);
  const overlayTip = t.overlayEnabled ? `遮罩 ${t.overlayStrength}%` : "无遮罩";
  const del = custom
    ? `<span class="template-del" data-action="delete" data-id="${escapeHtml(t.id)}" title="删除模板">
         <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M6 6l12 12M18 6L6 18" /></svg>
       </span>`
    : "";
  return `
    <button class="template" data-id="${escapeHtml(t.id)}" title="${escapeHtml(t.name)} · ${overlayTip}">
      ${del}
      <div class="template-swatch" style="background:linear-gradient(145deg, ${swatch[0]}, ${swatch[1]})"></div>
      <div class="template-name">${escapeHtml(t.name)}</div>
      <div class="template-desc">${t.temperature} K · ${Math.round(t.brightness * 100)}%</div>
    </button>`;
}

function renderTemplates() {
  const html = TEMPLATES.map((t) => templateCardHtml(t, false));
  state.customTemplates.forEach((t) => html.push(templateCardHtml(t, true)));
  for (let i = state.customTemplates.length; i < MAX_CUSTOM; i += 1) {
    html.push('<div class="template-empty" aria-hidden="true"></div>');
  }
  $("#templateGrid").innerHTML = html.join("");
}

function applyTemplate(id) {
  const tpl = allTemplates().find((t) => t.id === id);
  if (!tpl) return;
  state.temperature = tpl.temperature;
  state.brightness = tpl.brightness;
  state.overlayEnabled = Boolean(tpl.overlayEnabled);
  state.overlayColor = tpl.overlayColor || DEFAULT_OVERLAY_COLOR;
  state.overlayStrength = typeof tpl.overlayStrength === "number" ? tpl.overlayStrength : 18;
  state.enabled = true;
  syncUI();
  applyNow();
  scheduleSave();
  toast(`已应用「${tpl.name}」`);
}

function openSaveModal() {
  if (matchTemplate()) {
    toast("当前配置已是模板，请先调节出新配置", true);
    return;
  }
  if (state.customTemplates.length >= MAX_CUSTOM) {
    toast(`最多保存 ${MAX_CUSTOM} 个，请先删除一个`, true);
    return;
  }
  const mask = state.overlayEnabled ? `遮罩 ${state.overlayStrength}%` : "无遮罩";
  $("#savePreview").textContent = `当前：${state.temperature} K · ${Math.round(state.brightness * 100)}% · ${mask}`;
  $("#templateNameInput").value = "";
  $("#saveModal").classList.add("show");
  setTimeout(() => $("#templateNameInput").focus(), 50);
}

function confirmSaveTemplate() {
  const name = $("#templateNameInput").value.trim();
  if (!name) { toast("请输入模板名称", true); return; }
  if (state.customTemplates.length >= MAX_CUSTOM) {
    toast(`最多保存 ${MAX_CUSTOM} 个，请先删除一个`, true);
    $("#saveModal").classList.remove("show");
    return;
  }
  state.customTemplates.push({
    id: `custom-${Date.now()}`,
    name,
    temperature: state.temperature,
    brightness: state.brightness,
    overlayEnabled: state.overlayEnabled,
    overlayColor: state.overlayColor,
    overlayStrength: state.overlayStrength,
  });
  $("#saveModal").classList.remove("show");
  renderTemplates();
  syncUI();
  scheduleSave();
  toast("模板已保存");
}

let pendingDeleteId = null;

function openDeleteModal(id) {
  const tpl = state.customTemplates.find((t) => t.id === id);
  if (!tpl) return;
  pendingDeleteId = id;
  $("#deleteTemplateName").textContent = tpl.name;
  $("#deleteModal").classList.add("show");
}

function closeDeleteModal() {
  pendingDeleteId = null;
  $("#deleteModal").classList.remove("show");
}

function confirmDeleteTemplate() {
  if (!pendingDeleteId) return;
  state.customTemplates = state.customTemplates.filter((t) => t.id !== pendingDeleteId);
  closeDeleteModal();
  renderTemplates();
  syncUI();
  scheduleSave();
  toast("模板已删除");
}

/* ---------- 效果与持久化 ---------- */
async function applyNow() {
  try {
    await invoke("apply_effect", { settings: state });
  } catch (e) {
    toast(`应用失败：${e}`, true);
  }
}
function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    invoke("save_settings", { settings: state }).catch((e) => toast(`保存失败：${e}`, true));
  }, 400);
}

/* ---------- 事件绑定 ---------- */
function bindEvents() {
  // 原生应用体验：禁用右键菜单
  document.addEventListener("contextmenu", (event) => event.preventDefault());

  document.querySelectorAll(".nav-item").forEach((item) => {
    item.addEventListener("click", () => {
      document.querySelectorAll(".nav-item").forEach((n) => n.classList.remove("active"));
      item.classList.add("active");
      const view = item.dataset.view;
      document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
      $(`#view-${view}`).classList.add("active");
    });
  });

  $("#brightnessRange").addEventListener("input", (e) => {
    state.brightness = Number(e.target.value) / 100;
    syncUI();
    applyNow();
    scheduleSave();
  });
  $("#temperatureRange").addEventListener("input", (e) => {
    state.temperature = Number(e.target.value);
    syncUI();
    applyNow();
    scheduleSave();
  });

  $("#masterSwitch").addEventListener("click", () => {
    state.enabled = !state.enabled;
    syncUI();
    applyNow();
    scheduleSave();
    toast(state.enabled ? "护眼效果已开启" : "护眼效果已关闭");
  });

  // 模板区委托：删除 / 应用模板（空槽仅为占位展示，不是新增入口）
  $("#templateGrid").addEventListener("click", (event) => {
    const del = event.target.closest("[data-action='delete']");
    if (del) {
      event.stopPropagation();
      openDeleteModal(del.dataset.id);
      return;
    }
    const card = event.target.closest(".template");
    if (card) applyTemplate(card.dataset.id);
  });

  $("#btnSaveTemplate").addEventListener("click", openSaveModal);
  $("#btnCancelSave").addEventListener("click", () => $("#saveModal").classList.remove("show"));
  $("#btnConfirmSave").addEventListener("click", confirmSaveTemplate);
  $("#btnCancelDelete").addEventListener("click", closeDeleteModal);
  $("#btnConfirmDelete").addEventListener("click", confirmDeleteTemplate);
  $("#templateNameInput").addEventListener("keydown", (e) => {
    if (e.key === "Enter") confirmSaveTemplate();
    if (e.key === "Escape") $("#saveModal").classList.remove("show");
  });

  $("#autostartSwitch").addEventListener("click", async () => {
    const next = !isSwitchOn($("#autostartSwitch"));
    try {
      await invoke("set_autostart", { enabled: next });
      state.autostartEnabled = next;
      setSwitch($("#autostartSwitch"), next);
      scheduleSave();
      toast(next ? "开机自启已开启" : "开机自启已关闭");
    } catch (e) {
      toast(`设置失败：${e}`, true);
    }
  });

  $("#minimizedSwitch").addEventListener("click", () => {
    state.startMinimized = !state.startMinimized;
    setSwitch($("#minimizedSwitch"), state.startMinimized);
    scheduleSave();
    toast(state.startMinimized ? "启动时将直接进入托盘" : "启动时将显示主窗口");
  });

  $("#updateCheckSwitch").addEventListener("click", () => {
    state.checkUpdatesOnStartup = !state.checkUpdatesOnStartup;
    setSwitch($("#updateCheckSwitch"), state.checkUpdatesOnStartup);
    scheduleSave();
    toast(state.checkUpdatesOnStartup ? "启动时将自动检查更新" : "已关闭启动检查更新");
  });

  $("#overlaySwitch").addEventListener("click", () => {
    state.overlayEnabled = !state.overlayEnabled;
    syncUI();
    applyNow();
    scheduleSave();
    toast(state.overlayEnabled ? "柔色遮罩已开启" : "柔色遮罩已关闭");
  });
  $("#colorSwatches").addEventListener("click", (e) => {
    const sw = e.target.closest(".color-swatch");
    if (!sw) return;
    state.overlayColor = sw.dataset.hex;
    state.overlayEnabled = true;
    syncUI();
    applyNow();
    scheduleSave();
    const color = OVERLAY_COLORS.find((c) => c.hex === sw.dataset.hex);
    toast(`已切换为${color ? color.name : "柔色"}遮罩`);
  });
  $("#strengthRange").addEventListener("input", (e) => {
    state.overlayStrength = Number(e.target.value);
    syncUI();
    applyNow();
    scheduleSave();
  });

  $("#btnClose").addEventListener("click", () => {
    invoke("close_main_window").catch((e) => toast(e, true));
  });

  $("#btnUpdate").addEventListener("click", () => checkUpdate(false));
  $("#btnChangelog").addEventListener("click", () => {
    invoke("open_external", { url: `https://github.com/${UPDATE_REPO}/releases` })
      .catch((e) => toast(e, true));
  });

  $("#modalMore").addEventListener("click", () => {
    if (latestRelease && latestRelease.html_url) {
      invoke("open_external", { url: latestRelease.html_url }).catch((e) => toast(e, true));
    }
  });
  $("#btnLater").addEventListener("click", () => $("#updateModal").classList.remove("show"));
  $("#btnDownload").addEventListener("click", () => {
    if (latestRelease && latestRelease.html_url) {
      invoke("open_external", { url: latestRelease.html_url }).catch((e) => toast(e, true));
    }
    $("#updateModal").classList.remove("show");
  });
}

/* ---------- 检查更新 ---------- */
// GitHub release body 是 Markdown，这里转成弹窗里可读的纯文本
function releaseNotesToText(body) {
  if (!body || typeof body !== "string") return "";
  return body
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")        // 图片
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")     // 链接保留显示文字
    .replace(/^\s{0,3}#{1,6}\s*/gm, "")          // 标题符号
    .replace(/\*\*|__|`/g, "")                   // 加粗 / 行内代码
    .replace(/^\s*[-*+]\s+/gm, "· ")             // 列表项
    .split("\n")
    .map((line) => line.replace(/\s+$/g, ""))
    .filter((line, i, arr) => line || (i > 0 && arr[i - 1])) // 合并连续空行
    .join("\n")
    .trim();
}

// 弹窗显示真实更新日志；过长则截断并显示"查看更多"
function renderUpdateNotes(release) {
  const notes = $("#modalNotes");
  const more = $("#modalMore");
  const text = releaseNotesToText(release && release.body);
  if (!text) {
    notes.textContent = "新版本已发布，包含体验优化与问题修复。建议更新以获得更好的护眼体验。";
    more.style.display = "none";
    return;
  }
  if (text.length > UPDATE_NOTES_LIMIT) {
    let cut = text.slice(0, UPDATE_NOTES_LIMIT);
    // 优先在行末或句读处收尾，避免截断在半个句子中间
    const lastBreak = Math.max(cut.lastIndexOf("\n"), cut.lastIndexOf("。"), cut.lastIndexOf("；"));
    if (lastBreak > UPDATE_NOTES_LIMIT * 0.6) cut = cut.slice(0, lastBreak + 1);
    notes.textContent = `${cut.trimEnd()}…`;
    more.style.display = "block";
  } else {
    notes.textContent = text;
    more.style.display = "none";
  }
}

function compareVersion(a, b) {
  const pa = String(a).replace(/^v/i, "").split(".").map(Number);
  const pb = String(b).split(".").map(Number);
  for (let i = 0; i < 3; i += 1) {
    const diff = (pa[i] || 0) - (pb[i] || 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

async function checkUpdate(isStartup) {
  const btn = $("#btnUpdate");
  const result = $("#updateResult");
  if (!isStartup) {
    btn.disabled = true;
    result.textContent = "正在检查…";
  }
  try {
    const response = await fetch(`https://api.github.com/repos/${UPDATE_REPO}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const release = await response.json();
    latestRelease = release;
    const latest = release.tag_name || "";
    if (!isStartup) {
      if (compareVersion(latest, APP_VERSION) > 0) {
        result.className = "update-result warn";
        result.textContent = `发现新版本 ${latest}`;
      } else {
        result.className = "update-result ok";
        result.textContent = `当前已是最新版本（${APP_VERSION}）`;
      }
    }
    if (isStartup && compareVersion(latest, APP_VERSION) > 0) {
      // 启动最小化时窗口隐藏在托盘，先唤起主窗口，否则弹窗用户看不到
      invoke("show_main_window").catch(() => {});
      $("#modalVersion").textContent = latest;
      renderUpdateNotes(release);
      $("#updateModal").classList.add("show");
    }
  } catch (error) {
    console.warn("update check failed:", error);
    if (!isStartup) {
      result.className = "update-result";
      result.textContent = "暂时无法获取更新信息，请检查网络或更新源配置";
    }
  } finally {
    btn.disabled = false;
  }
}

/* ---------- 启动 ---------- */
async function init() {
  try {
    const saved = await invoke("load_settings");
    if (saved && typeof saved === "object") Object.assign(state, saved);
  } catch (e) {
    console.warn("load settings failed:", e);
  }
  // 兼容旧配置 / 异常数据
  if (!Array.isArray(state.customTemplates)) state.customTemplates = [];
  if (state.customTemplates.length > MAX_CUSTOM) state.customTemplates.length = MAX_CUSTOM;
  state.customTemplates.forEach((t) => {
    if (typeof t.overlayEnabled !== "boolean") t.overlayEnabled = false;
    if (typeof t.overlayStrength !== "number") t.overlayStrength = 18;
    if (typeof t.overlayColor !== "string") t.overlayColor = DEFAULT_OVERLAY_COLOR;
  });
  if (typeof state.overlayEnabled !== "boolean") state.overlayEnabled = false;
  if (typeof state.overlayStrength !== "number") state.overlayStrength = 18;
  if (typeof state.overlayColor !== "string") state.overlayColor = DEFAULT_OVERLAY_COLOR;

  // 版本号单点注入：以后只改 APP_VERSION 即可，HTML 中不再硬编码
  document.querySelectorAll("[data-app-version]").forEach((el) => {
    el.textContent = `v${APP_VERSION}`;
  });

  renderTemplates();
  renderColorSwatches();
  bindEvents();
  try {
    const portable = await invoke("is_portable");
    if (portable) {
      const row = $("#autostartRow");
      if (row) row.style.display = "none";
      state.autostartEnabled = false;
    } else {
      const autostart = await invoke("is_autostart_enabled");
      state.autostartEnabled = Boolean(autostart);
      setSwitch($("#autostartSwitch"), state.autostartEnabled);
    }
  } catch (e) {
    console.warn("portable/autostart query failed:", e);
  }
  syncUI();
  $("#updateResult").textContent = `当前版本 v${APP_VERSION}`;
  applyNow();
  if (state.checkUpdatesOnStartup) setTimeout(() => checkUpdate(true), 1200);
}

init();
