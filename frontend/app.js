/* MyEyeCare 前端逻辑：无框架、无 npm 依赖，直接通过 Tauri IPC 调用 Rust 命令 */

// 检查更新与更新日志指向同一开源仓库
const UPDATE_REPO = "yaoahos/MyEyeCare";
const APP_VERSION = "1.0.3";

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
const DEFAULT_SOFTNESS = 45;
const MAX_CUSTOM = 3;

// 每套模板自带遮罩配置，切换时亮度 / 色温 / 柔和度 / 遮罩一起生效。
// 设计原则：按使用场景由日间到深夜递进——色温递减、亮度递减、柔和度与遮罩递增；
// 色卡取各自场景的代表色（蓝→绿→黄→紫→橙→红），一眼即可区分，不与遮罩色混为一谈。
const TEMPLATES = [
  { id: "standard",  name: "标准日间", temperature: 5000, brightness: 0.92, softness: 45, swatch: ["#d6ecfb", "#8fc6ea"], overlayEnabled: true,  overlayColor: "#8FD6BC", overlayStrength: 12 },
  { id: "focus",     name: "专注工作", temperature: 4800, brightness: 0.90, softness: 50, swatch: ["#d4f0df", "#7fd0a8"], overlayEnabled: true,  overlayColor: "#8FD6BC", overlayStrength: 16 },
  { id: "reading",   name: "阅读模式", temperature: 4500, brightness: 0.86, softness: 55, swatch: ["#fff0c9", "#eccf8c"], overlayEnabled: true,  overlayColor: "#FFD98A", overlayStrength: 20 },
  { id: "movie",     name: "影院模式", temperature: 4100, brightness: 0.70, softness: 55, swatch: ["#eadffa", "#b9a2e6"], overlayEnabled: true,  overlayColor: "#B6A6E8", overlayStrength: 24 },
  { id: "night",     name: "夜间护眼", temperature: 3600, brightness: 0.64, softness: 60, swatch: ["#ffdcc0", "#f2a874"], overlayEnabled: true,  overlayColor: "#FFB060", overlayStrength: 30 },
  { id: "deepnight", name: "深夜模式", temperature: 3000, brightness: 0.50, softness: 60, swatch: ["#f7cfc4", "#dd8b78"], overlayEnabled: true,  overlayColor: "#FF9E80", overlayStrength: 40 },
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
  brightness: 0.92,
  temperature: 5000,
  softness: DEFAULT_SOFTNESS,
  enabled: true,
  autostartEnabled: true,
  startMinimized: true,
  checkUpdatesOnStartup: true,
  templateId: "standard",
  overlayEnabled: true,
  overlayStrength: 12,
  overlayColor: DEFAULT_OVERLAY_COLOR,
  customTemplates: [],
  scheduleEnabled: false,
  // 昼夜两态日程（参考 LightBulb）：只设两组色温/亮度 + 日出日落 + 过渡时长
  dayTemperature: 6600,
  dayBrightness: 1.0,
  nightTemperature: 3900,
  nightBrightness: 0.85,
  sunriseMinute: 7 * 60 + 20,
  sunsetMinute: 18 * 60 + 30,
  transitionMinutes: 40,
  manualOverrideUntil: 0,
  // 息屏：空闲分钟(0=关闭，沿用系统设置)；快捷键固定 Ctrl+Enter
  idleSleepMinutes: 0,
  lockOnSleep: false,
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
    && Number(t.softness ?? 0) === Number(state.softness ?? 0)
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

  const softRange = $("#softnessRange");
  if (softRange) {
    softRange.value = state.softness;
    softRange.style.setProperty("--pct", `${state.softness}%`);
    $("#softnessValue").textContent = `${state.softness}%`;
  }

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
  setSwitch($("#lockOnSleepSwitch"), state.lockOnSleep);

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
  // 模板是固定配置，与按时间自动调节互斥：应用模板时关闭自动日程，
  // 避免日程 tick 把模板值抢回去（此前只打 30 分钟覆盖，过期就失效）。
  const wasScheduled = state.scheduleEnabled;
  if (wasScheduled) {
    state.scheduleEnabled = false;
    state.manualOverrideUntil = 0;
    setSwitch($("#scheduleSwitch"), false);
    // 只清覆盖标记，不调 clearManualOverride()——后者会异步拉回日程值覆盖即将设置的模板值
    invoke("set_manual_override", { until: 0 }).catch(() => {});
    invoke("set_schedule_enabled", { enabled: false }).catch(() => {});
    updateScheduleNow();
  }
  state.temperature = tpl.temperature;
  state.brightness = tpl.brightness;
  state.softness = typeof tpl.softness === "number" ? tpl.softness : DEFAULT_SOFTNESS;
  state.overlayEnabled = Boolean(tpl.overlayEnabled);
  state.overlayColor = tpl.overlayColor || DEFAULT_OVERLAY_COLOR;
  state.overlayStrength = typeof tpl.overlayStrength === "number" ? tpl.overlayStrength : 18;
  state.enabled = true;
  syncUI();
  applyNow();
  scheduleSave();
  toast(wasScheduled ? `已应用「${tpl.name}」，自动日程已关闭` : `已应用「${tpl.name}」`);
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
  $("#savePreview").textContent = `当前：${state.temperature} K · ${Math.round(state.brightness * 100)}% · 柔和 ${state.softness}% · ${mask}`;
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
    softness: state.softness,
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

/* ---------- 自动日程：昼夜两态模型（参考 LightBulb） ----------
   用户只设「白天」「夜晚」两组色温+亮度，以及日出/日落时刻和过渡时长。
   曲线是按与后端相同的余弦公式算出来的只读展示，不提供拖拽——
   参数即数值，从根上消除“点、线、数值对不上”。                    */
const SCHED_MIN_TEMP = 2700;
const SCHED_MAX_TEMP = 6500;
const SCHED_MIN_BRIGHT = 0.1, SCHED_MAX_BRIGHT = 1.0;

// viewBox 600×120：上轨色温 y∈[6,54]，下轨亮度 y∈[66,114]
const SCHED_TEMP_TOP = 6, SCHED_TEMP_BOT = 54;
const SCHED_BRIGHT_TOP = 66, SCHED_BRIGHT_BOT = 114;

function schedX(minute) { return (minute / 1440) * 600; }
function schedTempY(temp) {
  const t = (temp - SCHED_MIN_TEMP) / (SCHED_MAX_TEMP - SCHED_MIN_TEMP);
  return SCHED_TEMP_BOT - t * (SCHED_TEMP_BOT - SCHED_TEMP_TOP);
}
function schedBrightY(bright) {
  const t = (bright - SCHED_MIN_BRIGHT) / (SCHED_MAX_BRIGHT - SCHED_MIN_BRIGHT);
  return SCHED_BRIGHT_BOT - t * (SCHED_BRIGHT_BOT - SCHED_BRIGHT_TOP);
}
function padTime(minute) {
  const m = ((minute % 1440) + 1440) % 1440;
  const h = Math.floor(m / 60);
  return `${String(h).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}
function currentMinute() {
  const d = new Date();
  return d.getHours() * 60 + d.getMinutes();
}

// 与 Rust schedule.rs::interpolate 完全同源的昼夜余弦插值，返回 { temperature, brightness }
function schedInterpolate(minute) {
  const half = Math.max(1, state.transitionMinutes / 2);
  const diff = (center) => {
    let d = minute - center;
    if (d >= 720) d -= 1440;
    if (d < -720) d += 1440;
    return d;
  };
  const sd = diff(state.sunriseMinute);
  const ed = diff(state.sunsetMinute);
  let dayRatio;
  if (Math.abs(sd) < half) {
    const p = (sd + half) / (2 * half);
    dayRatio = (1 - Math.cos(Math.PI * p)) / 2;
  } else if (Math.abs(ed) < half) {
    const p = (ed + half) / (2 * half);
    dayRatio = (1 + Math.cos(Math.PI * p)) / 2;
  } else {
    const daylight = ((state.sunsetMinute - state.sunriseMinute) % 1440 + 1440) % 1440;
    const since = ((minute - state.sunriseMinute) % 1440 + 1440) % 1440;
    dayRatio = since < daylight ? 1 : 0;
  }
  return {
    temperature: Math.round(state.nightTemperature + (state.dayTemperature - state.nightTemperature) * dayRatio),
    brightness: state.nightBrightness + (state.dayBrightness - state.nightBrightness) * dayRatio,
  };
}

// 采样 145 个点画平滑曲线（只读）。曲线值 = 真正生效值，二者同源。
function renderSchedule() {
  const set = (id, key, val) => { const el = $(id); if (el) el.setAttribute(key, val); };
  const minutes = [];
  for (let m = 0; m <= 1440; m += 3) minutes.push(m);
  const vals = minutes.map((m) => schedInterpolate(Math.min(m, 1439)));

  const tempLine = minutes.map((m, i) =>
    `${i === 0 ? "M" : "L"}${schedX(m).toFixed(1)} ${schedTempY(vals[i].temperature).toFixed(1)}`).join(" ");
  const brightLine = minutes.map((m, i) =>
    `${i === 0 ? "M" : "L"}${schedX(m).toFixed(1)} ${schedBrightY(vals[i].brightness).toFixed(1)}`).join(" ");
  set("#schedTempLine", "d", tempLine);
  set("#schedTempArea", "d", `${tempLine} L600 ${SCHED_TEMP_TOP - 2} L0 ${SCHED_TEMP_TOP - 2} Z`);
  set("#schedBrightLine", "d", brightLine);
  set("#schedBrightArea", "d", `${brightLine} L600 120 L0 120 Z`);

  // 日出 / 日落 / 当前时刻三条竖线，颜色与图例一致
  const links = $("#schedLinks");
  if (links) {
    const mark = (m, cls) => `<line class="${cls}" x1="${schedX(m)}" y1="0" x2="${schedX(m)}" y2="120"></line>`;
    links.innerHTML = mark(state.sunriseMinute, "sunrise") + mark(state.sunsetMinute, "sunset");
  }

  // 时间轴固定刻度（曲线是连续展示，刻度用整点更易读）
  const axis = $("#scheduleAxis");
  if (axis) {
    axis.innerHTML = [0, 360, 720, 1080, 1440]
      .map((m) => `<span style="left:${(m / 1440) * 100}%">${padTime(m) === "24:00" ? "24:00" : padTime(m)}</span>`).join("");
  }

  updateScheduleNow();
}

// 把 state 中的昼夜参数回填到输入框/数值显示
function syncScheduleConfigUI() {
  const put = (id, val) => { const el = $(`#${id}`); if (el) el.value = val; };
  put("dayTempInput", state.dayTemperature);
  put("nightTempInput", state.nightTemperature);
  put("dayBrightInput", Math.round(state.dayBrightness * 100));
  put("nightBrightInput", Math.round(state.nightBrightness * 100));
  put("transitionInput", state.transitionMinutes);
  put("sunriseInput", padTime(state.sunriseMinute));
  put("sunsetInput", padTime(state.sunsetMinute));
}

/* ---------- 息屏设置 ---------- */
const IDLE_OPTIONS = [0, 5, 10, 15, 30, 60];
function normalizeIdleMinutes(value) {
  const n = Number(value) || 0;
  return IDLE_OPTIONS.includes(n)
    ? n
    : IDLE_OPTIONS.reduce((best, item) => (Math.abs(item - n) < Math.abs(best - n) ? item : best), 0);
}

function setHotkeyHint(registered) {
  const hint = $("#hotkeyHint");
  if (!hint) return;
  hint.textContent = registered
    ? "固定 Ctrl+Enter，全局生效"
    : "固定 Ctrl+Enter（被占用）";
  hint.classList.toggle("hint-error", !registered);
}

function refreshSleepStatus(showWarning = false) {
  return invoke("get_sleep_status").then((status) => {
    const ok = !status || status.hotkey_registered !== false;
    setHotkeyHint(ok);
    if (!ok && showWarning) toast("Ctrl+Enter 息屏快捷键被其他程序占用", true);
    return ok;
  }).catch((e) => {
    console.warn("sleep status failed:", e);
    return true; // 查询失败时不误报占用
  });
}

function initSleepUI() {
  syncSleepUI();
  // 启动时确认固定快捷键是否注册成功（可能被别的全局热键占用）
  refreshSleepStatus(true);
}

function syncSleepUI() {
  state.idleSleepMinutes = normalizeIdleMinutes(state.idleSleepMinutes);
  const idle = $("#idleSleepSelect");
  if (idle) idle.value = String(state.idleSleepMinutes);
}

// 写回空闲时长并立即生效；快捷键在后端固定，无需传参
function pushSleepSettings() {
  invoke("set_sleep_settings", {
    idleMinutes: state.idleSleepMinutes,
    lockOnSleep: state.lockOnSleep,
  })
    .then(() => setTimeout(() => refreshSleepStatus(true), 150))
    .catch((e) => toast(`息屏设置失败：${e}`, true));
}

// 写回后端昼夜配置
function pushScheduleConfig() {
  invoke("set_schedule_config", {
    config: {
      dayTemperature: state.dayTemperature,
      dayBrightness: state.dayBrightness,
      nightTemperature: state.nightTemperature,
      nightBrightness: state.nightBrightness,
      sunriseMinute: state.sunriseMinute,
      sunsetMinute: state.sunsetMinute,
      transitionMinutes: state.transitionMinutes,
    },
  }).catch(() => {});
}

function updateScheduleNow() {
  const nowLine = $("#schedNow");
  // 日程未启用时隐藏当前时刻指针，避免让人误以为曲线正在生效
  if (nowLine) nowLine.style.display = state.scheduleEnabled ? "" : "none";
  if (state.scheduleEnabled) {
    const x = (currentMinute() / 1440) * 600;
    nowLine.setAttribute("x1", `${x}`);
    nowLine.setAttribute("x2", `${x}`);
  }
  const label = $("#scheduleNowText");
  if (label) {
    if (state.scheduleEnabled) {
      const v = schedInterpolate(currentMinute());
      label.textContent = `此刻 ${v.temperature}K · ${Math.round(v.brightness * 100)}%`;
    } else {
      label.textContent = "开启后按此曲线自动调节";
    }
  }
  // 信息行：显示当前处于白天/夜晚/过渡，帮用户把曲线和读数对上
  const info = $("#schedSelected");
  if (info) {
    const now = currentMinute();
    const half = Math.max(1, state.transitionMinutes / 2);
    const near = (c) => { let d = now - c; if (d >= 720) d -= 1440; if (d < -720) d += 1440; return Math.abs(d) < half; };
    const daylight = ((state.sunsetMinute - state.sunriseMinute) % 1440 + 1440) % 1440;
    const since = ((now - state.sunriseMinute) % 1440 + 1440) % 1440;
    const phase = near(state.sunriseMinute) || near(state.sunsetMinute)
      ? "过渡中" : (since < daylight ? "白天" : "夜晚");
    info.textContent = `白天 ${state.dayTemperature}K·${Math.round(state.dayBrightness * 100)}% ｜ 夜晚 ${state.nightTemperature}K·${Math.round(state.nightBrightness * 100)}% ｜ 当前 ${phase}`;
  }
  const remaining = state.manualOverrideUntil - Date.now();
  const status = $("#scheduleStatus");
  const restoreBtn = $("#btnRestoreAuto");
  if (state.scheduleEnabled && remaining > 0) {
    const mins = Math.ceil(remaining / 60000);
    if (status) status.textContent = `手动覆盖中 · ${mins} 分钟后恢复`;
    if (restoreBtn) restoreBtn.style.display = "inline-block";
  } else if (state.scheduleEnabled) {
    if (status) status.textContent = "自动中";
    if (restoreBtn) restoreBtn.style.display = "none";
  } else {
    if (status) status.textContent = "按时间自动调节色温与亮度";
    if (restoreBtn) restoreBtn.style.display = "none";
  }
}

function applyScheduleNow() {
  if (!state.scheduleEnabled || state.manualOverrideUntil > Date.now()) return;
  const v = schedInterpolate(currentMinute());
  state.temperature = v.temperature;
  state.brightness = Math.round(v.brightness * 100) / 100;
  syncUI();
  applyNow();
}

/* ---------- 手动覆盖（与日程联动） ---------- */
const OVERRIDE_MINUTES = 30;
// 用户主动调节时打上“临时覆盖”标记：30 分钟内日程让位给手动值，到期自动恢复。
function markManualOverride() {
  if (!state.scheduleEnabled) return;
  state.manualOverrideUntil = Date.now() + OVERRIDE_MINUTES * 60 * 1000;
  invoke("set_manual_override", { until: state.manualOverrideUntil }).catch(() => {});
}
function clearManualOverride() {
  state.manualOverrideUntil = 0;
  invoke("set_manual_override", { until: 0 }).catch(() => {});
  // 恢复自动后，把日程此刻计算出的值拉回界面
  invoke("get_effective_settings").then((eff) => {
    if (eff && typeof eff === "object") {
      state.brightness = eff.brightness;
      state.temperature = eff.temperature;
      state.softness = eff.softness;
      syncUI();
      applyNow();
    }
  }).catch(() => {});
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
    markManualOverride();
    syncUI();
    applyNow();
    scheduleSave();
  });
  $("#temperatureRange").addEventListener("input", (e) => {
    state.temperature = Number(e.target.value);
    markManualOverride();
    syncUI();
    applyNow();
    scheduleSave();
  });
  const softRangeEl = $("#softnessRange");
  if (softRangeEl) softRangeEl.addEventListener("input", (e) => {
    state.softness = Number(e.target.value);
    markManualOverride();
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

  // 自动日程开关
  $("#scheduleSwitch").addEventListener("click", async () => {
    state.scheduleEnabled = !state.scheduleEnabled;
    setSwitch($("#scheduleSwitch"), state.scheduleEnabled);
    await invoke("set_schedule_enabled", { enabled: state.scheduleEnabled }).catch(() => {});
    if (state.scheduleEnabled) {
      // 开启后拉回当前时间对应的自动值
      invoke("get_effective_settings").then((eff) => {
        if (eff) {
          state.temperature = eff.temperature;
          state.brightness = eff.brightness;
          state.softness = eff.softness;
          syncUI();
          applyNow();
        }
      }).catch(() => {});
      toast("自动日程已开启");
    } else {
      clearManualOverride();
      toast("自动日程已关闭，恢复手动调节");
    }
    updateScheduleNow();
    scheduleSave();
  });
  $("#btnSchedulePreset").addEventListener("click", () => {
    state.dayTemperature = 6600;
    state.dayBrightness = 1.0;
    state.nightTemperature = 3900;
    state.nightBrightness = 0.85;
    state.sunriseMinute = 7 * 60 + 20;
    state.sunsetMinute = 18 * 60 + 30;
    state.transitionMinutes = 40;
    syncScheduleConfigUI();
    renderSchedule();
    pushScheduleConfig();
    applyScheduleNow();
    scheduleSave();
    toast("已恢复默认昼夜参数");
  });
  // 昼夜两态参数绑定：改动后本地重画曲线并写回后端
  const schedInputs = [
    ["dayTempInput", "dayTemperature", (v) => Math.round(Math.max(2700, Math.min(7000, v)))],
    ["nightTempInput", "nightTemperature", (v) => Math.round(Math.max(2700, Math.min(7000, v)))],
    // 输入框是百分比(10~100)，state 存 0~1 小数，必须除以 100（此前误按小数钳制，输入 85 被钳成 1.0）
    ["dayBrightInput", "dayBrightness", (v) => Math.max(0.1, Math.min(1, v / 100))],
    ["nightBrightInput", "nightBrightness", (v) => Math.max(0.1, Math.min(1, v / 100))],
    ["transitionInput", "transitionMinutes", (v) => Math.round(Math.max(0, Math.min(180, v)))],
  ];
  schedInputs.forEach(([id, key, norm]) => {
    const el = $(`#${id}`);
    if (!el) return;
    el.addEventListener("change", () => {
      state[key] = norm(Number(el.value));
      syncScheduleConfigUI();
      renderSchedule();
      pushScheduleConfig();
      applyScheduleNow();
      scheduleSave();
    });
  });
  // 日出/日落用 time input，value 为 HH:MM
  [["sunriseInput", "sunriseMinute"], ["sunsetInput", "sunsetMinute"]].forEach(([id, key]) => {
    const el = $(`#${id}`);
    if (!el) return;
    el.addEventListener("change", () => {
      const [h, m] = (el.value || "0:0").split(":").map(Number);
      if (!Number.isFinite(h) || !Number.isFinite(m)) return;
      state[key] = h * 60 + m;
      renderSchedule();
      pushScheduleConfig();
      applyScheduleNow();
      scheduleSave();
    });
  });
  $("#btnRestoreAuto").addEventListener("click", () => {
    clearManualOverride();
    toast("已恢复自动调节");
  });

  // 息屏设置：空闲时长 + 是否一并锁屏，快捷键固定 Ctrl+Enter
  $("#idleSleepSelect").addEventListener("change", (e) => {
    state.idleSleepMinutes = Number(e.target.value) || 0;
    pushSleepSettings();
    scheduleSave();
    toast(state.idleSleepMinutes ? `空闲 ${state.idleSleepMinutes} 分钟后自动息屏` : "已关闭空闲息屏");
  });
  $("#lockOnSleepSwitch").addEventListener("click", () => {
    state.lockOnSleep = !state.lockOnSleep;
    setSwitch($("#lockOnSleepSwitch"), state.lockOnSleep);
    pushSleepSettings();
    scheduleSave();
    toast(state.lockOnSleep ? "息屏时将一并锁定 Windows" : "息屏时不再锁定 Windows");
  });
  $("#btnSleepNow").addEventListener("click", () => {
    invoke("sleep_now")
      .then(() => toast(state.lockOnSleep
        ? "已发送锁屏与息屏指令"
        : "已发送息屏指令，移动鼠标即可唤醒"))
      .catch((e) => toast(e, true));
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

  const closeBtn = $("#btnClose");
  const clearCloseHover = () => {
    if (closeBtn) closeBtn.classList.remove("is-hovered");
  };
  if (closeBtn) {
    // 用 JS 类而不是 :hover：窗口隐藏时 WebView 收不到 mouseleave，:hover 会被冻结。
    closeBtn.addEventListener("pointerenter", () => closeBtn.classList.add("is-hovered"));
    closeBtn.addEventListener("pointerleave", clearCloseHover);
  }
  closeBtn.addEventListener("click", (ev) => {
    // 点击瞬间清掉悬停态与焦点，避免隐藏/恢复托盘后按钮看起来仍“被选中”
    clearCloseHover();
    if (ev.currentTarget && typeof ev.currentTarget.blur === "function") ev.currentTarget.blur();
    invoke("close_main_window").catch((e) => toast(e, true));
  });
  // 从托盘恢复或窗口失焦时，主动清掉可能残留的悬停态与焦点（Rust 侧也会清理）。
  window.addEventListener("focus", () => {
    clearCloseHover();
    if (closeBtn && typeof closeBtn.blur === "function") closeBtn.blur();
    if (document.activeElement && document.activeElement !== document.body &&
        typeof document.activeElement.blur === "function") {
      document.activeElement.blur();
    }
  });
  window.addEventListener("blur", clearCloseHover);
  document.addEventListener("pointerleave", clearCloseHover);

  $("#btnUpdate").addEventListener("click", () => checkUpdate(false));
  $("#btnChangelog").addEventListener("click", () => {
    invoke("open_external", { url: `https://github.com/${UPDATE_REPO}/releases` })
      .catch((e) => toast(e, true));
  });

  $("#btnLater").addEventListener("click", () => $("#updateModal").classList.remove("show"));
  $("#btnDownload").addEventListener("click", async () => {
    if (!latestRelease || !latestRelease.assetUrl) {
      // 没有可下载的安装包直链时，退回浏览器发布页
      const url = latestRelease && latestRelease.url
        ? latestRelease.url
        : `https://github.com/${UPDATE_REPO}/releases`;
      invoke("open_external", { url }).catch((e) => toast(e, true));
      $("#updateModal").classList.remove("show");
      return;
    }
    const btn = $("#btnDownload");
    const err = $("#updateError");
    btn.disabled = true;
    btn.textContent = "正在下载…";
    err.style.display = "none";
    try {
      await invoke("update_download_and_install", { assetUrl: latestRelease.assetUrl });
      $("#updateModal").classList.remove("show");
      toast("安装包已启动，请按提示完成安装");
    } catch (e) {
      err.textContent = typeof e === "string" ? e : "下载失败，请检查网络后重试";
      err.style.display = "block";
    } finally {
      btn.disabled = false;
      btn.textContent = "立即更新";
    }
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

// 弹窗显示完整更新日志，不再截断；超出弹窗范围的内容放在滚动区内查看
function renderUpdateNotes(release) {
  const notes = $("#modalNotes");
  const text = releaseNotesToText(release && release.notes);
  notes.textContent = text || "新版本已发布，包含体验优化与问题修复。建议更新以获得更好的护眼体验。";
  notes.scrollTop = 0;
}

// 版本比较在后端 check_update 命令中完成（hasUpdate），前端直接采用结果
async function checkUpdate(isStartup) {
  const btn = $("#btnUpdate");
  const result = $("#updateResult");
  if (!isStartup) {
    btn.disabled = true;
    result.textContent = "正在检查…";
  }
  try {
    // 检查与下载都收拢到后端命令：后端负责 GitHub 访问、限流处理与安装包直链解析
    const info = await invoke("check_update", { appVersion: APP_VERSION });
    if (!info || typeof info !== "object") return;
    latestRelease = info;
    const hasUpdate = !!info.hasUpdate;
    const latest = info.latest || "";
    if (!isStartup) {
      if (hasUpdate) {
        result.className = "update-result warn";
        result.textContent = `发现新版本 v${latest}`;
      } else {
        result.className = "update-result ok";
        result.textContent = `当前已是最新版本（${APP_VERSION}）`;
      }
    }
    if (hasUpdate) {
      // 启动检查时窗口可能藏在托盘，先唤起；手动检查时窗口已在前台，调用无副作用
      if (isStartup) invoke("show_main_window").catch(() => {});
      $("#modalVersion").textContent = `v${latest}`;
      renderUpdateNotes(info);
      $("#updateModal").classList.add("show");
    }
  } catch (error) {
    console.warn("update check failed:", error);
    if (!isStartup) {
      result.className = "update-result";
      const msg = typeof error === "string" ? error : "";
      result.textContent = msg || "暂时无法获取更新信息，请检查网络或更新源配置";
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
  if (typeof state.softness !== "number") state.softness = DEFAULT_SOFTNESS;
  if (typeof state.scheduleEnabled !== "boolean") state.scheduleEnabled = false;
  // 昼夜两态参数：旧配置没有这些字段时用默认值补齐（0 视为未初始化）
  if (!state.dayTemperature) state.dayTemperature = 6600;
  if (!(state.dayBrightness > 0)) state.dayBrightness = 1.0;
  if (!state.nightTemperature) state.nightTemperature = 3900;
  if (!(state.nightBrightness > 0)) state.nightBrightness = 0.85;
  if (!state.sunriseMinute) state.sunriseMinute = 7 * 60 + 20;
  if (!state.sunsetMinute) state.sunsetMinute = 18 * 60 + 30;
  if (!state.transitionMinutes) state.transitionMinutes = 40;
  if (typeof state.manualOverrideUntil !== "number") state.manualOverrideUntil = 0;
  // 息屏字段兼容旧配置：clamp 到界面提供的选项，避免出现下拉框显示空白
  state.idleSleepMinutes = normalizeIdleMinutes(state.idleSleepMinutes);
  if (typeof state.lockOnSleep !== "boolean") state.lockOnSleep = false;

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
  setSwitch($("#scheduleSwitch"), state.scheduleEnabled);
  syncScheduleConfigUI();
  initSleepUI();
  renderSchedule();
  $("#updateResult").textContent = `当前版本 v${APP_VERSION}`;
  applyNow();
  // 日程时间指针与覆盖状态每 30 秒刷新；覆盖到期后自动把界面拉回自动值
  let wasOverridden = state.manualOverrideUntil > Date.now();
  setInterval(() => {
    updateScheduleNow();
    const isOverridden = state.manualOverrideUntil > Date.now();
    if (wasOverridden && !isOverridden && state.scheduleEnabled) {
      state.manualOverrideUntil = 0;
      clearManualOverride();
    }
    wasOverridden = isOverridden;
  }, 30_000);
  if (state.checkUpdatesOnStartup) setTimeout(() => checkUpdate(true), 1200);
}

init();
