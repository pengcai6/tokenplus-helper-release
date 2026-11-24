/* TokenPlus 助手 — Vue 3 + Vue Router renderer (global builds, no bundler).
 * Ported from ui.js — same DOM structure, same classes, same bridge contract.
 * Navigation is vue-router (hash mode): /home /tools /models /pets /diag /settings,
 * /tool/:id (nested under /tools), /login guarded by auth state.
 * Talks to the local bridge via fetch('/invoke') + EventSource('/events').
 * Append ?mock=1 to preview with fake data and no backend.
 */
(function () {
  'use strict';

  /* error collector (preview/debug aid) */
  window.__errs = '';
  window.addEventListener('error', function (e) {
    window.__errs += (e.message || e.type || 'error') + '\n';
  });
  window.addEventListener('unhandledrejection', function (e) {
    var r = e.reason;
    window.__errs += 'unhandledrejection: ' + ((r && r.message) || r || '') + '\n';
  });

  var createApp = Vue.createApp, reactive = Vue.reactive, watch = Vue.watch;
  var createRouter = VueRouter.createRouter, createWebHashHistory = VueRouter.createWebHashHistory;

  /* ================= bridge ================= */
  /* Three transports, same contract: Tauri IPC (desktop app), the local
     HTTP bridge (Qt shell / dev preview), or ?mock=1 (no backend). */
  var MOCK = /[?&]mock=1/.test(location.search);
  var TAURI = (typeof window !== 'undefined' && window.__TAURI__) || null;
  var subs = {};
  function openEvents() {
    try {
      new EventSource('/events').onmessage = function (e) {
        var m = JSON.parse(e.data);
        (subs[m.channel] || []).slice().forEach(function (f) { try { f(m.value); } catch (x) {} });
      };
    } catch (e) {}
  }
  if (!MOCK && !TAURI) openEvents();
  function invoke(channel, args) {
    if (MOCK) return mockInvoke(channel, args || []);
    if (TAURI) {
      return TAURI.core.invoke('dispatch', { channel: channel, args: args || [] })
        .catch(function (e) {
          throw new Error(typeof e === 'string' ? e : ((e && e.message) || String(e)));
        });
    }
    return fetch('/invoke', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ channel: channel, args: args || [] })
    }).then(function (r) { return r.json(); }).then(function (d) {
      if (!d.ok) throw new Error(d.error || '调用失败');
      return d.value;
    });
  }
  function on(channel, fn) {
    if (TAURI && !MOCK) {
      TAURI.event.listen(channel, function (e) { try { fn(e.payload); } catch (x) {} });
      return function () {};
    }
    (subs[channel] = subs[channel] || []).push(fn);
    return function () { var l = subs[channel], i = l.indexOf(fn); if (i >= 0) l.splice(i, 1); };
  }

  /* ================= tool meta ================= */
  var TOOL_META = {
    'claude-code':    { name: 'Claude Code',    kind: '命令行',          icon: 'claude',   restart: '新开一个终端窗口即可生效' },
    'claude-desktop': { name: 'Claude 桌面版',  kind: '桌面应用',        icon: 'claude',   restart: '完全退出 Claude 再重新打开后生效' },
    'codex':          { name: 'Codex',          kind: '命令行 + 桌面应用', icon: 'codex',    restart: '重新打开 Codex 后生效' },
    'grok':           { name: 'Grok Build',     kind: '命令行 + 桌面应用', icon: 'grok',     restart: '新开一个终端窗口或重启 Grok 后生效' },
    'deepseek':       { name: 'DeepSeek Harness', kind: '桌面应用',      icon: 'deepseek', restart: '重启 DeepSeek Harness 后生效' },
    'workbuddy':      { name: 'WorkBuddy',      kind: '桌面应用',        icon: 'workbuddy', restart: '重启 WorkBuddy 后生效' }
  };
  var TOOL_ORDER = ['claude-code', 'claude-desktop', 'codex', 'grok', 'deepseek', 'workbuddy'];
  var STATUS = {
    ok: '已接入', todo: '待接入', drift: '已被改动', missing: '未安装', error: '异常'
  };
  var STEPS = ['读取现有配置', '备份', '获取线路配置', '写入', '完成'];
  var ICON_FILES = {
    claude: 'DqQDuysG', codex: 'C8tCaqwc', deepseek: 'D_Yralcc',
    grok: 'R5s05a9o', workbuddy: 'D5AM3pO_'
  };
  var icon = function (t) { return './assets/' + (TOOL_META[t] || {}).icon + '-' + ICON_FILES[(TOOL_META[t] || {}).icon] + '.png'; };
  var TP_MARK = './assets/tp-mark.svg';
  var APP_NAME = 'TokenPlus 小助手';

  /* ================= store ================= */
  var S = reactive({
    ready: false, auth: { phase: 'out' }, tools: [], account: null,
    accountError: '', settings: {}, backups: [], version: '',
    deviceName: '', selfCheck: null, diagnostics: [], models: null,
    flow: null, toast: '', toastOut: false
  });
  var favs = {};
  try { favs = JSON.parse(localStorage.getItem('tp-pet-favs') || '{}'); } catch (e) {}
  /* module-level UI state (non-routed overlays + pets module) */
  var M = reactive({
    petsData: null, petTab: 'lib', petQuery: '', favs: favs,
    petDetail: null, petDetailRow: 0,
    logs: {},          // toolId -> [{text, err, at}]
    conn: null,        // {phase, plan:{loading,data,error}, queue, idx, step, cur, tools:{}, message, failures, retry}
    dropdownOpen: 0,   // open spring-select panels (Esc guard)
    detailClose: null, // registered by the expand-card tool detail
    tests: {}          // toolId -> {runId, state:'run'|'ok'|'fail', model, text, chunks, firstToken, latency, message, at}
  });
  function toast(msg) {
    S.toast = msg; S.toastOut = false;
    clearTimeout(toast._t);
    toast._t = setTimeout(function () { S.toastOut = true; }, 2600);
  }

  /* ================= svg icons ================= */
  // Lucide icons (ISC license) — lucide.dev
  var SVG = {
    home: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8"/><path d="M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
    tools: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/></svg>',
    model: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z"/><path d="M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12"/><path d="M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17"/></svg>',
    pet: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="4" r="2"/><circle cx="18" cy="8" r="2"/><circle cx="20" cy="16" r="2"/><path d="M9 10a5 5 0 0 1 5 5v3.5a3.5 3.5 0 0 1-6.84 1.045Q6.52 17.48 4.46 16.84A3.5 3.5 0 0 1 5.5 10Z"/></svg>',
    diag: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2"/></svg>',
    bolt: '<svg viewBox="0 0 24 24" fill="none" stroke="#10B981" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15.914 4a1.5 1.5 0 0 0-2.474-1.561l-9 9A1.5 1.5 0 0 0 5.5 14h4.002a.5.5 0 0 1 .471.666L8.086 20a1.5 1.5 0 0 0 2.475 1.56l9-9A1.5 1.5 0 0 0 18.5 10h-3.997a.5.5 0 0 1-.472-.667z"/></svg>',
    boltW: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="width:15px;height:15px;vertical-align:-2px"><path d="M15.914 4a1.5 1.5 0 0 0-2.474-1.561l-9 9A1.5 1.5 0 0 0 5.5 14h4.002a.5.5 0 0 1 .471.666L8.086 20a1.5 1.5 0 0 0 2.475 1.56l9-9A1.5 1.5 0 0 0 18.5 10h-3.997a.5.5 0 0 1-.472-.667z"/></svg>',
    heart: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 9.5a5.5 5.5 0 0 1 9.591-3.676.56.56 0 0 0 .818 0A5.49 5.49 0 0 1 22 9.5c0 2.29-1.5 4-3 5.5l-5.492 5.313a2 2 0 0 1-3 .019L5 15c-1.5-1.5-3-3.2-3-5.5"/></svg>',
    dl: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="width:13px;height:13px;vertical-align:-1px"><path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/></svg>',
    set: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"/><circle cx="12" cy="12" r="3"/></svg>',
    key: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z"/><circle cx="16.5" cy="7.5" r=".5" fill="currentColor"/></svg>',
    archive: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/></svg>'
  };
  var NAV = [['home', '首页'], ['tools', '工具'], ['models', '模型'], ['pets', '宠物'], ['diag', '连接诊断'], ['backups', '配置备份'], ['settings', '设置']];
  var NAV_ICON = { home: 'home', tools: 'tools', models: 'model', pets: 'pet', diag: 'diag', backups: 'archive', settings: 'set' };
  var PTABS = [['lib', '宠物库'], ['installed', '已安装'], ['fav', '我的收藏']];
  var SETTING_ITEMS = [
    ['boot', '开机自动启动', '登录电脑后在后台安静运行'],
    ['tray', '关闭窗口后留在菜单栏', '随时点图标看余额和状态'],
    ['watch', '配置被改动时提醒我', '定期自检一次，被别的服务商改走会自动改回并通知你'],
    ['update', '自动检查更新', '打开助手时检查，有新版本会提示，一键即可更新']
  ];
  var DIAG_DEFAULT = [
    ['network', '网络', '能连上 TokenPlus'],
    ['login', '登录', '本机保存的密钥有效'],
    ['balance', '余额', 'TokenPlus 账户可用'],
    ['claude', 'Claude 线路', 'Claude Code、Claude 桌面版使用'],
    ['codex', 'Codex 线路', 'Codex、WorkBuddy 使用'],
    ['general', '通用线路', 'Grok、DeepSeek 等使用']
  ];
  var DIAG_MARKS = { pending: '·', run: '…', done: '✓', fail: '✗' };
  var PET_ANIM = [ // [label, frames, duration]
    ['待机', 6, 1600], ['向右跑', 8, 760], ['向左跑', 8, 760], ['挥手', 4, 850],
    ['跳跃', 5, 820], ['失落', 8, 1200], ['等待', 6, 1250], ['工作', 6, 840], ['审阅', 6, 1050]
  ];
  var PET_BG = {
    'yituo-ember': '#e3e7fb', 'yituo-byte': '#f5f2d8', 'yituo-orbit': '#dbf3e8',
    'yituo-luna': '#fae3f0', 'yituo-jade': '#dff1ea', 'yituo-sprout': '#e8f5da',
    'yituo-nova': '#e7ebfb', 'yituo-toast': '#fbeed9'
  };

  /* ================= helpers ================= */
  function fmtSize(n) {
    n = +n || 0;
    if (n >= 1048576) return (n / 1048576).toFixed(1) + ' MB';
    if (n >= 1024) return (n / 1024).toFixed(1) + ' KB';
    return n + ' B';
  }
  /* models entries may be plain strings or objects {id,label,desc} */
  function modelId(m) {
    if (m == null) return '';
    return typeof m === 'object' ? (m.id || m.label || '') : String(m);
  }
  function modelLabel(m) {
    if (m == null) return '';
    return typeof m === 'object' ? (m.label || m.id || '') : String(m);
  }
  function modelDesc(m) {
    return (m && typeof m === 'object' && m.desc) ? String(m.desc) : '';
  }
  function authed() { return !!(S.auth && S.auth.phase === 'in'); }
  function accountName() {
    var u = ((S.auth || {}).user || {});
    return u.username || u.display_name || 'TokenPlus 用户';
  }
  function connected() { return S.tools.filter(function (t) { return t.status === 'ok'; }).length; }
  function needed() { return S.tools.filter(function (t) { return ['todo', 'drift', 'error'].indexOf(t.status) >= 0 && t.installed; }); }
  /* per-tool activity log, survives detail close/reopen */
  function finalizeTest(run, r) {
  if (!run) return;
  run.state = r.ok === false ? 'fail' : 'ok';
  if (r.message != null) run.message = r.message;
  if (r.latency != null) run.latency = r.latency;
  if (r.firstToken != null) run.firstToken = r.firstToken;
  if (r.model) run.model = r.model;
  if (run.state !== 'run') run.at = run.at || Date.now();
}
function runTest(id) {
  var t = S.tools.find(function (x) { return x.id === id; });
  var runId = Date.now() + '' + Math.floor(Math.random() * 1e6);
  var run = (M.tests[id] = {
    runId: runId, state: 'run', model: t ? modelId(t.model) : '',
    text: '', chunks: [], firstToken: null, latency: null, message: '', at: 0,
  });
  tlog(id, '正在测试连接（' + run.model + '）…');
  return invoke('tools:test', [id, runId]).then(function (r) {
    finalizeTest(run, { ok: true, message: r.message, latency: r.latency, firstToken: r.firstToken, model: r.model });
    tlog(id, '测试 ' + (run.model || r.model || '') + '：✓ ' + Math.round(r.latency || 0) + 'ms（首字 ' + Math.round(r.firstToken || 0) + 'ms）');
    return r;
  }).catch(function (e) {
    finalizeTest(run, { ok: false, message: e.message });
    tlog(id, '测试 ' + run.model + '：✗ ' + e.message);
    throw e;
  });
}
function tlog(id, text, err) {
    var l = M.logs[id] || (M.logs[id] = []);
    l.push({ text: text, err: !!err, at: Date.now() });
    if (l.length > 200) l.splice(0, l.length - 200);
  }
  function relTime(ts) {
    if (!ts) return '';
    var d = Date.now() - ts;
    if (d < 60000) return '刚刚';
    if (d < 3600000) return Math.floor(d / 60000) + ' 分钟前';
    if (d < 86400000) return Math.floor(d / 3600000) + ' 小时前';
    return Math.floor(d / 86400000) + ' 天前';
  }
  var REDUCED = typeof matchMedia === 'function' &&
    matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* ================= shared actions ================= */
  function applyBootstrap(b) {
    var patch = Object.assign({ ready: true }, b);
    if (!patch.auth) patch.auth = { phase: 'out' }; // real bridge may send auth:null
    Object.assign(S, patch);
  }
  function refresh() {
    invoke('app:bootstrap').then(applyBootstrap);
  }
  function loadPets() {
    invoke('pets:list').then(function (r) { M.petsData = r; });
  }
  function openDetail(tool) {
    if (!S.tools.find(function (x) { return x.id === tool; })) return;
    router.push('/tool/' + tool);
  }
  function closeDetail() {
    // prefer history back (restores the tools view); direct entry -> push /tools
    var st = window.history.state;
    if (st && st.back) router.back();
    else router.push('/tools');
  }
  function openPetDetail(id) { M.petDetail = id; M.petDetailRow = 0; }
  function petUse(id) {
    invoke('pets:use', [id]).then(function (r) {
      if (r) M.petsData = r;
      else {
        var list = (M.petsData && M.petsData.pets) || [];
        var p = list.find(function (x) { return x.id === id; });
        if (p) { p.installed = true; M.petsData.selectedId = p.id; }
      }
      var sel = ((M.petsData && M.petsData.pets) || []).find(function (x) { return x.id === M.petsData.selectedId; });
      toast(sel ? '已将「' + sel.displayName + '」设为默认宠物' : '已启用');
    }).catch(function (e) { toast(e.message); });
  }
  function petFav(id) {
    M.favs[id] = !M.favs[id];
    try { localStorage.setItem('tp-pet-favs', JSON.stringify(M.favs)); } catch (x) {}
  }

  /* ================= connect overlay (plan preview + run, one overlay) ================= */
  function normPlan(r) {
    var d = {};
    if (!r || typeof r !== 'object') return d;
    if (Array.isArray(r)) {
      r.forEach(function (it) {
        var k = it && (it.tool || it.id || it.toolId);
        if (k) d[k] = it;
      });
      return d;
    }
    return r;
  }
  function toolSort(a, b) {
    var ia = TOOL_ORDER.indexOf(a), ib = TOOL_ORDER.indexOf(b);
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
  }
  function connRow() {
    return { state: '', step: -1, stepAt: {}, t0: 0, elapsed: 0,
      open: false, manual: false, failMsg: '' };
  }
  function sameConn(c) { return M.conn && Vue.toRaw(M.conn) === Vue.toRaw(c); }
  function newConn(phase) {
    var ids = needed().map(function (t) { return t.id; });
    var c = { phase: phase, plan: { loading: false, data: {}, error: '' },
      queue: ids, idx: 0, step: -1, cur: ids[0] || '', tools: {},
      message: '', failures: [], retry: '' };
    ids.forEach(function (id) { c.tools[id] = connRow(); });
    return c;
  }
  function fetchPlan(c) {
    c.plan.loading = true;
    invoke('tools:plan').then(function (r) {
      if (!sameConn(c)) return;
      c.plan = { loading: false, data: normPlan(r), error: '' };
      Object.keys(c.plan.data).forEach(function (id) {
        if (c.queue.indexOf(id) < 0) {
          c.queue.push(id);
          c.tools[id] = connRow();
        }
      });
      c.queue.sort(toolSort);
    }).catch(function (e) {
      if (!sameConn(c)) return;
      c.plan = { loading: false, data: c.plan.data || {},
        error: (e && e.message) || '无法读取接入计划' };
    });
  }
  function openPlan() {
    var c = newConn('preview');
    c.queue.forEach(function (id) { c.tools[id].open = true; });
    M.conn = c;
    fetchPlan(M.conn);
  }
  function startConnectAll() {
    var ids = needed().map(function (t) { return t.id; });
    if (!ids.length) { toast('没有需要接入的工具'); return; }
    var c = (M.conn && M.conn.phase === 'preview') ? M.conn : newConn('run');
    M.conn = c;
    c = M.conn; // work through the proxy so mutations re-render
    c.phase = 'run';
    if (!c.plan.loading && !Object.keys(c.plan.data).length) fetchPlan(c);
    invoke('tools:connect', [c.queue.slice(), false]).then(function () {
      if (sameConn(c) && c.phase === 'run') c.phase = 'done';
    }).catch(function (e) {
      if (sameConn(c) && c.phase === 'run') {
        c.phase = 'fail'; c.message = e.message || '接入失败';
      } else if (!sameConn(c)) toast(e.message);
    });
  }
  function connRecompute(c) {
    var st = c.queue.map(function (id) { return (c.tools[id] || {}).state; });
    if (st.length && st.every(function (s) { return s === 'done'; })) c.phase = 'done';
    else if (st.some(function (s) { return s === 'fail'; })) c.phase = 'fail';
    else c.phase = 'run';
  }
  function connUpdate(f) {
    var conn = M.conn;
    if (!conn || !f || typeof f !== 'object') return;
    var q = Array.isArray(f.queue) ? f.queue : [];
    var sameQ = q.length && q.join(',') === conn.queue.join(',');
    var retryQ = conn.retry && q.join(',') === conn.retry;
    if (!sameQ && !retryQ && conn.phase !== 'run') return; // stale event for a finished queue
    if (sameQ) conn.queue = q.slice();
    q.forEach(function (id) {
      if (!conn.tools[id]) conn.tools[id] = connRow();
    });
    var now = performance.now();
    var idx = f.index != null ? +f.index
      : (f.tool ? conn.queue.indexOf(f.tool) : conn.idx);
    if (idx < 0 || isNaN(idx)) idx = conn.idx;
    if (sameQ) {
      conn.idx = idx;
      if (f.step != null) conn.step = +f.step;
      if (f.tool) conn.cur = f.tool;
    }
    var ct = f.tool && conn.tools[f.tool];
    if (ct) {
      if (!ct.t0) ct.t0 = now;
      ct.elapsed = now - ct.t0;
      if (f.step != null && ct.step !== +f.step) {
        ct.stepAt[+f.step] = now;
        ct.step = +f.step;
      }
    }
    var fails = Array.isArray(f.failures) ? f.failures : [];
    fails.forEach(function (x) {
      var tid = typeof x === 'string' ? x : (x && (x.tool || x.id));
      var ts = tid && conn.tools[tid];
      if (ts && ts.state !== 'fail') {
        ts.state = 'fail';
        ts.failMsg = (x && x.message) || ts.failMsg || '接入失败';
        if (!ts.manual) ts.open = false;
      }
    });
    if (fails.length) conn.failures = fails;
    if (f.toolState === 'fail' && ct && ct.state !== 'fail') {
      ct.state = 'fail';
      ct.failMsg = f.message || '接入失败';
      if (!ct.manual) ct.open = false;
    }
    if (f.state === 'run') {
      conn.phase = 'run';
      if (ct && ct.state !== 'fail') {
        ct.state = 'run';
        if (!ct.manual) ct.open = true;
      }
      if (sameQ) conn.queue.forEach(function (id, i) {
        var ts = conn.tools[id];
        if (i < idx && ts.state !== 'fail') {
          ts.state = 'done'; ts.step = STEPS.length - 1;
          if (!ts.manual) ts.open = false;
        }
      });
    } else if (f.state === 'done') {
      if (retryQ) {
        if (ct) { ct.state = 'done'; ct.step = STEPS.length - 1; ct.failMsg = ''; }
        conn.retry = '';
        connRecompute(conn);
      } else {
        conn.phase = 'done';
        conn.queue.forEach(function (id) {
          var ts = conn.tools[id];
          if (ts.state !== 'fail') { ts.state = 'done'; ts.step = STEPS.length - 1; }
          if (!ts.manual) ts.open = false;
        });
      }
    } else if (f.state === 'fail') {
      if (retryQ) {
        if (ct) { ct.state = 'fail'; ct.failMsg = f.message || '接入失败'; }
        conn.retry = '';
        connRecompute(conn);
      } else {
        conn.phase = 'fail';
        if (ct) { ct.state = 'fail'; ct.failMsg = f.message || '接入失败'; }
        conn.message = f.message || '接入失败';
      }
    }
  }

  /* ---------- micro spring engine (DesignBookmark-style motion) ---------- */
  function spring(x, k, c) { return { x: x, v: 0, t: x, k: k || 400, c: c || 30 }; }
  function springTune(s, k, c) { s.k = k; s.c = c; }
  function springStep(s, dt) {
    var a = -s.k * (s.x - s.t) - s.c * s.v;
    s.v += a * dt; s.x += s.v * dt;
  }
  function clamp01(v) { return Math.max(0, Math.min(1, v)); }

  /* ================= component option objects ================= */
  /* nav-item: router-link custom slot keeps a real <button> (CSS expects .nav > button) */
  var NavItem = {
    props: ['pid', 'label'],
    computed: {
      html: function () { return SVG[NAV_ICON[this.pid]] + '<span class="lb">' + this.label + '</span>'; }
    },
    template:
      '<router-link :to="\'/\' + pid" custom v-slot="{ navigate, isActive }">' +
      '<button :data-page="pid" :class="{ on: isActive }" @click="navigate" v-html="html"></button></router-link>'
  };

  /* app-shell: layout route — sidebar + <router-view> for pages, toast sibling.
     Sidebar: collapsible icon rail + spring highlight pill (db-sidebar-collapse
     + db-tab-pill motion studies). */
  var AppShell = {
    data: function () { return { collapsed: false }; },
    mounted: function () {
      var self = this;
      this._top = spring(0); this._bot = spring(0);
      this._W = spring(190, 260, 24);
      this._labels = []; this._last = performance.now();
      this._loop = function (now) { self._frame(now); };
      this.$nextTick(function () {
        self._labels = (self.$el && self.$el.querySelectorAll ? Array.prototype.slice.call(
          self.$el.querySelectorAll('.nav .lb, .side .fadeable')) : [])
          .map(function (el) { return { el: el, s: spring(1, 300, 26) }; });
        self.retarget(true);
        self._raf = requestAnimationFrame(self._loop);
      });
    },
    unmounted: function () { cancelAnimationFrame(this._raf); },
    watch: {
      '$route.path': function () {
        var self = this;
        this.$nextTick(function () { self.retarget(false); });
      }
    },
    methods: {
      retarget: function (snap) {
        var el = this.$el && this.$el.querySelector ? this.$el.querySelector('.nav button.on') : null;
        if (!el) return;
        var dir = el.offsetTop >= this._top.t ? 1 : -1;
        this._top.t = el.offsetTop;
        this._bot.t = el.offsetTop + el.offsetHeight;
        // edge in the direction of travel is stiff, the other soft -> stretch
        if (dir > 0) { springTune(this._bot, 560, 34); springTune(this._top, 220, 21); }
        else { springTune(this._top, 560, 34); springTune(this._bot, 220, 21); }
        if (snap) { this._top.x = this._top.t; this._bot.x = this._bot.t; }
      },
      toggleSide: function () {
        var self = this;
        this.collapsed = !this.collapsed;
        this._W.t = this.collapsed ? 58 : 190;
        var ls = this._labels;
        if (this.collapsed) {
          ls.forEach(function (l) { l.s.t = 0; });
        } else {
          ls.forEach(function (l, i) {
            setTimeout(function () { if (!self.collapsed) l.s.t = 1; }, 60 + i * 40);
          });
        }
        this.$nextTick(function () { self.retarget(false); });
      },
      _frame: function (now) {
        var dt = Math.min((now - this._last) / 1000, 1 / 30);
        this._last = now;
        springStep(this._top, dt); springStep(this._bot, dt); springStep(this._W, dt);
        var i, l, o;
        for (i = 0; i < this._labels.length; i++) springStep(this._labels[i].s, dt);
        var qs = this.$el && this.$el.querySelector ? this.$el : null;
        var pill = qs && qs.querySelector('.navpill');
        if (pill) {
          pill.style.transform = 'translateY(' + this._top.x.toFixed(2) + 'px)';
          pill.style.height = Math.max(0, this._bot.x - this._top.x).toFixed(2) + 'px';
        }
        var side = qs && qs.querySelector('.side');
        if (side) side.style.width = this._W.x.toFixed(1) + 'px';
        for (i = 0; i < this._labels.length; i++) {
          l = this._labels[i]; o = Math.max(0, Math.min(1, l.s.x));
          l.el.style.opacity = o.toFixed(3);
          l.el.style.transform = 'translateX(' + ((1 - o) * -10).toFixed(2) + 'px)';
        }
        this._raf = requestAnimationFrame(this._loop);
      }
    },
    template:
      '<div class="shell"><div class="side" :class="{ mini: collapsed }">' +
      '<div class="logo"><img :src="TP_MARK"><div class="fadeable"><b>TokenPlus</b><small>小助手 · AI 助手</small></div>' +
      '<button class="col" @click="toggleSide" :aria-expanded="!collapsed" title="收起/展开侧边栏">' +
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.5 6.5 9 12l5.5 5.5"/></svg></button></div>' +
      '<div class="nav"><span class="navpill"></span>' +
      '<nav-item v-for="n in NAV" :key="n[0]" :pid="n[0]" :label="n[1]"></nav-item></div>' +
      '<div class="user-chip fadeable"><span class="av" v-html="ic(\'key\')"></span><span>{{ (S.auth || {}).key_hint || accountName() }}</span><i>v{{ S.version }}</i></div></div>' +
      '<div class="main"><router-view></router-view></div></div>' +
      '<app-toast></app-toast>'
  };

  var AppToast = {
    template: '<div class="toast" v-if="S.toast" :class="{ out: S.toastOut }">{{ S.toast }}</div>'
  };

  /* ---------- login (route /login) ---------- */
  var PageLogin = {
    data: function () { return { hint: false, keyInput: '' }; },
    methods: {
      startLogin: function () {
        var self = this;
        invoke('auth:start').then(function () { self.hint = true; })
          .catch(function (e) { toast(e.message); });
      },
      openSignup: function () {
        var url = 'https://www.tokenplus.cn/';
        invoke('shell:open', [url]).catch(function () { window.open(url, '_blank'); });
      },
      saveKey: function () {
        var key = this.keyInput.trim();
        if (!key) return;
        invoke('auth:import-key', [key]).then(function () {
          toast('密钥已保存'); refresh();
        }).catch(function (e) { toast(e.message); });
      }
    },
    template:
      '<div class="login-wrap"><div class="login-card">' +
      '<img class="lmark" :src="TP_MARK"><div class="pill"><span class="dot"></span>{{ APP_NAME }}</div>' +
      '<h1>所有 AI 工具<br><em>一次接好</em></h1>' +
      '<p>在浏览器里登录 TokenPlus，创建 API 密钥后粘贴回来，为 Claude Code、Codex、Grok 等工具写好配置。改动前会自动备份。</p>' +
      '<button class="btn primary" id="tpLogin" @click="startLogin">在浏览器中打开 TokenPlus 登录</button>' +
      '<p class="hint" id="tpHint" v-show="hint">已打开。登录后到「API 密钥」页创建密钥，复制后粘贴到下面输入框。</p>' +
      '<div class="key-row"><input type="text" id="keyInput" placeholder="粘贴 sk- 密钥" v-model="keyInput"><button class="btn" id="keySave" @click="saveKey">保存</button></div>' +
      '<div class="signup">没有 TokenPlus 账号？' +
      '<a id="tpSignup" href="https://www.tokenplus.cn/" target="_blank" rel="noopener" @click.prevent="openSignup">前往官网注册</a></div>' +
      '</div></div>'
  };

  /* rolling number (spring) for stat cards */
  var SpringNum = {
    props: ['to'],
    data: function () { return { v: 0 }; },
    watch: {
      to: function (n) {
        this._s.t = +n || 0;
        if (REDUCED) { this._s.x = this._s.t; this.v = this._s.t; }
      }
    },
    mounted: function () {
      var self = this;
      this._s = spring(0, 140, 20);
      this._s.t = +this.to || 0;
      if (REDUCED) { this._s.x = this._s.t; this.v = this._s.t; }
      this._last = performance.now();
      this._loop = function (now) { self._frame(now); };
      this._raf = requestAnimationFrame(this._loop);
    },
    unmounted: function () { cancelAnimationFrame(this._raf); },
    methods: {
      _frame: function (now) {
        var dt = Math.min((now - this._last) / 1000, 1 / 30);
        this._last = now;
        springStep(this._s, dt);
        this.v = this._s.x;
        this._raf = requestAnimationFrame(this._loop);
      }
    },
    template: '<span>{{ Math.round(v) }}</span>'
  };

  /* svg progress ring (dashoffset on a spring) for 已接入 card */
  var StatRing = {
    props: ['frac'],
    watch: {
      frac: function (f) {
        this._s.t = Math.max(0, Math.min(1, +f || 0));
        if (REDUCED) { this._s.x = this._s.t; this._apply(); }
      }
    },
    mounted: function () {
      var self = this;
      this._s = spring(0, 160, 22);
      this._s.t = Math.max(0, Math.min(1, +this.frac || 0));
      if (REDUCED) this._s.x = this._s.t;
      this._last = performance.now();
      this._loop = function (now) { self._frame(now); };
      this._raf = requestAnimationFrame(this._loop);
      this._apply();
    },
    unmounted: function () { cancelAnimationFrame(this._raf); },
    methods: {
      _apply: function () {
        var c = this.$refs.fg;
        if (c) c.style.strokeDashoffset = (97.4 * (1 - clamp01(this._s.x))).toFixed(2);
      },
      _frame: function (now) {
        var dt = Math.min((now - this._last) / 1000, 1 / 30);
        this._last = now;
        springStep(this._s, dt);
        this._apply();
        this._raf = requestAnimationFrame(this._loop);
      }
    },
    template:
      '<span class="sring"><svg viewBox="0 0 36 36">' +
      '<circle class="bg" cx="18" cy="18" r="15.5"/>' +
      '<circle class="fg" ref="fg" cx="18" cy="18" r="15.5" stroke-dasharray="97.4" stroke-dashoffset="97.4"/>' +
      '</svg></span>'
  };

  /* ---------- home (route /home) ---------- */
  var PageHome = {
    data: function () { return { testing: false }; },
    computed: {
      neededN: function () { return needed().length; },
      connectedN: function () { return connected(); },
      issues: function () { return (S.selfCheck && S.selfCheck.issues) || []; },
      checkedAt: function () {
        return S.selfCheck && S.selfCheck.checkedAt ?
          new Date(S.selfCheck.checkedAt).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' }) : '';
      },
      checkText: function () {
        return !S.selfCheck ? '等待检查' : this.issues.length ? '发现 ' + this.issues.length + ' 项问题' : '环境正常';
      },
      diagText: function () {
        if (!S.diagnostics.length) return '尚未运行';
        var fails = S.diagnostics.filter(function (c) { return c.state === 'fail'; });
        var runs = S.diagnostics.filter(function (c) { return c.state === 'run'; });
        if (runs.length) return '正在诊断…';
        return fails.length ? fails.length + ' 项异常' : '全部正常';
      },
      modelSaved: function () {
        return S.models && S.models.savedAt ? relTime(S.models.savedAt) + '获取' : '尚未获取';
      },
      lastProbe: function () {
        var best = null;
        S.tools.forEach(function (t) {
          var p = t.lastProbe;
          if (p && p.at && (!best || p.at > best.at)) {
            best = { tool: t.id, at: p.at, ok: p.ok, latency: p.latency, model: p.model };
          }
        });
        return best;
      }
    },
    methods: {
      go: function (p) { this.$router.push(p); },
      toolName: function (id) { return (TOOL_META[id] || {}).name || id; },
      runSelfCheck: function () {
        toast('正在自检…');
        invoke('selfcheck:run').then(function (r) { S.selfCheck = r; toast('自检完成'); })
          .catch(function (e) { toast(e.message); });
      },
      testAll: function () {
        var ok = S.tools.filter(function (t) { return t.status === 'ok'; });
        if (!ok.length) { toast('还没有已接入的工具'); return; }
        var self = this, n = 0, m = 0;
        this.testing = true;
        (function next(i) {
          if (i >= ok.length) {
            self.testing = false;
            toast(n + ' 个正常，' + m + ' 个失败');
            return;
          }
          var t = ok[i];
          runTest(t.id).then(function () { n++; next(i + 1); })
            .catch(function () { m++; next(i + 1); });
        })(0);
      }
    },
    template:
      '<div class="page">' +
      '<div class="home-top"><span class="dot"></span><b>TokenPlus 已连接</b>' +
      '<i>{{ (S.auth || {}).key_hint || \'\' }}</i><em></em>' +
      '已接入 <b>{{ connectedN }}/{{ S.tools.length }}</b></div>' +
      '<div class="home-hero"><div class="left">' +
      '<h1 v-if="neededN">还有 <b>{{ neededN }}</b> 个工具<br>等你一键接入</h1>' +
      '<h1 v-else>所有工具<br>已经<b>接入完成</b></h1>' +
      '<p class="desc">Claude Code、Claude 桌面版、Codex、Grok Build、DeepSeek Harness、WorkBuddy 已经装在这台电脑上，但还没用上 TokenPlus。改动前会自动备份。</p>' +
      '<div class="cta"><template v-if="neededN">' +
      '<button class="btn primary" id="connectAll" @click="startConnectAll" v-html="ic(\'boltW\') + \' 一键接入\'"></button>' +
      '<button class="btn ghost" id="seeChanges" @click="openPlan">先看看会改什么</button></template>' +
      '<button v-else class="btn primary" id="testAll" :disabled="testing" @click="testAll">{{ testing ? \'正在测试…\' : \'全部测试一遍\' }}</button>' +
      '</div></div></div>' +
      '<div class="stat-row stat4">' +
      '<div class="card stat" @click="go(\'/tools\')">' +
      '<div class="stop"><stat-ring :frac="S.tools.length ? connectedN / S.tools.length : 0"></stat-ring>' +
      '<div class="lab">已接入</div><a class="stact">查看工具&nbsp;›</a></div>' +
      '<div class="val"><spring-num :to="connectedN"></spring-num><span class="of"> / {{ S.tools.length }}</span></div>' +
      '<div class="sub2">点开工具可换模型、测连接</div></div>' +
      '<div class="card stat" @click="go(\'/models\')">' +
      '<div class="stop"><div class="ic blue" v-html="ic(\'model\')"></div><div class="lab">可用模型</div><a class="stact">管理&nbsp;›</a></div>' +
      '<div class="val"><spring-num :to="(S.models || {}).count || 0"></spring-num></div>' +
      '<div class="sub2">{{ modelSaved }}</div></div>' +
      '<div class="card stat" @click="lastProbe && openDetail(lastProbe.tool)">' +
      '<div class="stop"><div class="ic" :class="lastProbe ? (lastProbe.ok ? \'green\' : \'orange\') : \'blue\'" v-html="ic(\'bolt\')"></div>' +
      '<div class="lab">最近测试</div><a class="stact" v-if="lastProbe">详情&nbsp;›</a></div>' +
      '<template v-if="lastProbe"><div class="val"><span class="vmark" :class="lastProbe.ok ? \'ok\' : \'bad\'">{{ lastProbe.ok ? \'✓\' : \'✗\' }}</span>' +
      ' {{ lastProbe.model || \'—\' }}</div>' +
      '<div class="sub2">{{ toolName(lastProbe.tool) }} · {{ Math.round(lastProbe.latency || 0) }}ms · {{ relTime(lastProbe.at) }}</div></template>' +
      '<template v-else><div class="val quiet">还没有测试过</div><div class="sub2">在工具详情里点「测试连接」</div></template></div>' +
      '<div class="card stat">' +
      '<div class="stop"><div class="ic" :class="issues.length ? \'orange\' : \'green\'" v-html="ic(\'diag\')"></div>' +
      '<div class="lab">环境状态</div><button class="stact" id="selfcheck" @click.stop="runSelfCheck">立即自检</button></div>' +
      '<div class="val" :class="{ green: !issues.length }">{{ checkText }}</div>' +
      '<div class="sub2">诊断：{{ diagText }}{{ checkedAt ? \' · \' + checkedAt : \'\' }}</div></div>' +
      '</div>' +
      '<div class="card" v-if="issues.length" style="margin-top:14px"><b style="font-size:13px">有设置把工具从 TokenPlus 改走了</b>' +
      '<ul style="margin:8px 0 0 18px;color:var(--sub);font-size:12px;line-height:2">' +
      '<li v-for="(i, k) in issues" :key="k">{{ i.text }}{{ i.fixable ? \'\' : \'（需要手动处理）\' }}</li></ul></div>' +
      '</div>'
  };

  /* ---------- tools (route /tools, nested /tool/:id view below) ---------- */
  var ToolCard = {
    props: ['t'],
    computed: {
      meta: function () { return TOOL_META[this.t.id] || { name: this.t.id, kind: '' }; },
      act: function () {
        if (!this.t.installed) return 'guide';
        if (this.t.status === 'ok') return 'open';
        return 'connect';
      },
      actLabel: function () {
        return { guide: '去下载安装', open: '查看详情', connect: '一键接入' }[this.act];
      },
      verText: function () {
        var t = this.t;
        return t.installed ? '已安装' + (t.version ? ' ' + t.version : '') : (t.message || '未检测到安装');
      }
    },
    methods: {
      onAct: function () {
        if (this.act === 'guide') invoke('tools:install-guide', [this.t.id]);
        else openDetail(this.t.id);
      }
    },
    template:
      '<div class="card tool-card" :data-tool="t.id" @click="openDetail(t.id)">' +
      '<div class="top"><div class="ticon"><img :src="icon(t.id)"></div>' +
      '<div class="tt"><b>{{ meta.name }}</b><small>{{ meta.kind }}</small></div>' +
      '<span class="badge" :class="t.status">{{ STATUS[t.status] }}</span></div>' +
      '<div class="tline"></div>' +
      '<div class="foot"><span class="ver">{{ verText }}</span>' +
      '<a class="go" :data-act="act" @click.stop="onAct">{{ actLabel }}&nbsp;›</a></div></div>'
  };

  var PageTools = {
    data: function () { return { busy: false, stage: null }; },
    mounted: function () { this.stage = this.$refs.stage; },
    methods: {
      reDetect: function () {
        var self = this;
        self.busy = true;
        invoke('tools:refresh').catch(function (e) { toast(e.message); })
          .finally(function () { self.busy = false; });
      },
      stageClick: function () {
        if (M.detailClose) M.detailClose();
      }
    },
    template:
      '<div class="page"><div class="ph"><div><h1>工具</h1><p>点开任意一个，换模型、测试连接或重新接入</p></div><span class="sp"></span>' +
      '<button class="btn" id="refreshTools" :disabled="busy" @click="reDetect">{{ busy ? \'正在检测…\' : \'重新检测\' }}</button></div>' +
      '<div class="tstage" ref="stage" @click.self="stageClick">' +
      '<div class="grid g3" :class="{ dim: $route.name === \'tool\' }"><tool-card v-for="t in S.tools" :key="t.id" :t="t"></tool-card></div>' +
      '<router-view v-slot="{ Component }"><component :is="Component" v-if="Component" :key="$route.fullPath" :stage="stage"/></router-view>' +
      '</div></div>'
  };

  /* ---------- spring-select: searchable dropdown, spring panel + active pill ---------- */
  var SpringSelect = {
    props: {
      options: { type: Array, default: function () { return []; } },
      modelValue: { default: '' },
      placeholder: { type: String, default: '请选择' },
      searchable: { type: Boolean, default: true }
    },
    emits: ['update:modelValue', 'change'],
    data: function () {
      return { open: false, q: '', active: -1,
        pos: { left: 0, top: 0, width: 280, flip: false } };
    },
    computed: {
      cur: function () {
        var mv = this.modelValue;
        return this.options.filter(function (o) { return o.value === mv; })[0] || null;
      },
      filtered: function () {
        var q = this.q.trim().toLowerCase();
        if (!q) return this.options;
        return this.options.filter(function (o) {
          return (String(o.value) + ' ' + String(o.label)).toLowerCase().indexOf(q) >= 0;
        });
      }
    },
    watch: {
      filtered: function () { this.active = this.filtered.length ? 0 : -1; }
    },
    mounted: function () {
      var self = this;
      this._s = spring(0, 420, 28);   // panel scaleY/opacity
      this._pill = spring(0, 420, 28); // active-row highlight top
      this._po = spring(0, 420, 28);   // pill opacity
      this._last = performance.now();
      this._loop = function (now) { self._frame(now); };
      this._raf = requestAnimationFrame(this._loop);
      this._doc = function (e) {
        if (!self.open) return;
        var p = self.$refs.panel;
        if (self.$el.contains(e.target) || (p && p.contains(e.target))) return;
        self.hide();
      };
      document.addEventListener('mousedown', this._doc);
    },
    unmounted: function () {
      cancelAnimationFrame(this._raf);
      document.removeEventListener('mousedown', this._doc);
      if (this.open) M.dropdownOpen--;
    },
    methods: {
      toggle: function () { this.open ? this.hide() : this.show(); },
      show: function () {
        var r = this.$refs.trig.getBoundingClientRect();
        var estH = 40 + Math.min(300, Math.max(1, this.options.length) * 34);
        var below = window.innerHeight - r.bottom;
        var flip = below < estH + 12 && r.top > below;
        this.pos = {
          left: r.left, width: Math.max(r.width, 280), flip: flip,
          top: flip ? Math.max(8, r.top - 6 - estH) : r.bottom + 6
        };
        this.open = true;
        this.q = '';
        var cur = this.cur;
        this.active = cur ? this.filtered.indexOf(cur) : (this.filtered.length ? 0 : -1);
        M.dropdownOpen++;
        if (REDUCED) { this._s.x = 1; this._po.x = 1; this._pill.x = this._pillT(); }
        this._s.t = 1;
        var self = this;
        this.$nextTick(function () {
          var i = self.$refs.q;
          if (i) i.focus();
          self.scrollActive();
        });
      },
      hide: function () {
        if (!this.open) return;
        this.open = false;
        M.dropdownOpen--;
        this._s.x = 0; this._s.t = 0;
      },
      pick: function (o) {
        if (!o) return;
        this.$emit('update:modelValue', o.value);
        this.$emit('change', o.value);
        this.hide();
      },
      onKey: function (e) {
        var k = e.key, n = this.filtered.length;
        if (k === 'ArrowDown' || k === 'ArrowUp') {
          e.preventDefault();
          if (!n) return;
          this.active = (this.active + (k === 'ArrowDown' ? 1 : -1) + n) % n;
          this.scrollActive();
        } else if (k === 'Enter') {
          e.preventDefault();
          this.pick(this.filtered[this.active]);
        } else if (k === 'Escape') {
          e.stopPropagation();
          this.hide();
        }
      },
      scrollActive: function () {
        var self = this;
        this.$nextTick(function () {
          var list = self.$refs.list;
          if (!list) return;
          var top = self.active * 34, bot = top + 34;
          if (top < list.scrollTop) list.scrollTop = top;
          else if (bot > list.scrollTop + list.clientHeight) list.scrollTop = bot - list.clientHeight;
        });
      },
      _pillT: function () { return this.active < 0 ? 0 : this.active * 34; },
      _frame: function (now) {
        var dt = Math.min((now - this._last) / 1000, 1 / 30);
        this._last = now;
        this._pill.t = this._pillT();
        this._po.t = this.active >= 0 ? 1 : 0;
        springStep(this._s, dt); springStep(this._pill, dt); springStep(this._po, dt);
        var p = this.$refs.panel;
        if (p) {
          var s = clamp01(this._s.x);
          p.style.opacity = s.toFixed(3);
          p.style.transform = 'scaleY(' + (0.92 + 0.08 * s).toFixed(4) + ')';
          var pill = p.querySelector('.ssel-pill');
          if (pill) {
            pill.style.transform = 'translateY(' + this._pill.x.toFixed(1) + 'px)';
            pill.style.opacity = clamp01(this._po.x).toFixed(3);
          }
        }
        this._raf = requestAnimationFrame(this._loop);
      }
    },
    template:
      '<span class="ssel">' +
      '<button type="button" class="ssel-trig" ref="trig" :class="{ open: open }" @click="toggle" @keydown="onKey">' +
      '<span class="ssel-cur" v-if="cur"><b>{{ cur.label }}</b><small v-if="cur.desc">{{ cur.desc }}</small></span>' +
      '<span class="ssel-ph" v-else>{{ placeholder }}</span>' +
      '<svg class="ssel-ch" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>' +
      '</button>' +
      '<Teleport to="body"><div v-if="open" class="ssel-panel" ref="panel" ' +
      ':class="{ flip: pos.flip }" :style="{ left: pos.left + \'px\', top: pos.top + \'px\', width: pos.width + \'px\' }">' +
      '<div class="ssel-search" v-if="searchable"><input ref="q" type="text" v-model="q" placeholder="搜索模型" @keydown="onKey"></div>' +
      '<div class="ssel-list" ref="list"><div class="ssel-pill"></div>' +
      '<div class="ssel-row" v-for="(o, i) in filtered" :key="o.value" :class="{ on: o.value === modelValue }" ' +
      '@click="pick(o)" @mouseenter="active = i">' +
      '<span class="ssel-check" v-if="o.value === modelValue">✓</span>' +
      '<b>{{ o.label }}</b><small v-if="o.desc">{{ o.desc }}</small></div>' +
      '<div class="ssel-empty" v-if="!filtered.length">没有匹配的模型</div>' +
      '</div></div></Teleport></span>'
  };

  /* ---------- models ---------- */
  var ModelSelect = {
    props: ['tool'],
    computed: {
      cur: function () { return modelId(this.tool.model); },
      opts: function () {
        return (this.tool.models || []).map(function (m) {
          return { value: modelId(m), label: modelLabel(m), desc: modelDesc(m) };
        });
      }
    },
    methods: {
      chg: function (v) {
        if (v === this.cur) return;
        invoke('tools:model', [this.tool.id, v]).then(function () {
          toast('模型已切换'); refresh();
        }).catch(function (x) { toast(x.message); });
      }
    },
    template:
      '<span class="msel"><spring-select :options="opts" :model-value="cur" ' +
      'placeholder="选择模型" :data-model-tool="tool.id" @change="chg"></spring-select></span>'
  };

  var PageModels = {
    data: function () { return { busy: false }; },
    computed: {
      installedTools: function () { return S.tools.filter(function (t) { return t.installed; }); }
    },
    methods: {
      modelCountText: function (t) {
        return t.models.length ? '已开通 ' + t.models.length + ' 个模型' : '接入后可选择模型';
      },
      refreshModels: function () {
        var self = this;
        self.busy = true;
        invoke('models:refresh').then(function (r) {
          var errs = Object.keys(r.errors || {}).length;
          toast('已获取 ' + (r.models || []).length + ' 个上游模型，更新 ' + (r.updated || []).length + ' 个工具' + (errs ? '，' + errs + ' 个失败' : ''));
          refresh();
        }).catch(function (e) { toast(e.message); })
          .finally(function () { self.busy = false; });
      }
    },
    template:
      '<div class="page"><div class="ph"><div><h1>模型</h1><p>每个工具各自选模型，改动立即写入配置</p></div><span class="sp"></span>' +
      '<button class="btn" :disabled="busy" @click="refreshModels">{{ busy ? \'获取中…\' : \'重新获取模型\' }}</button></div>' +
      '<div style="display:flex;flex-direction:column;gap:12px;max-width:760px">' +
      '<div class="card" v-for="t in installedTools" :key="t.id" style="display:flex;align-items:center;gap:14px">' +
      '<div class="ticon" style="width:40px;height:40px"><img :src="icon(t.id)" style="width:26px;height:26px"></div>' +
      '<div style="flex:1"><b>{{ (TOOL_META[t.id] || {}).name }}</b>' +
      '<div style="color:var(--sub);font-size:11px">{{ modelCountText(t) }}</div></div>' +
      '<model-select v-if="t.models.length" :tool="t"></model-select>' +
      '<span v-else class="badge" :class="t.status">{{ STATUS[t.status] }}</span>' +
      '</div></div></div>'
  };

  /* ---------- pets ---------- */
  var PetSprite = {
    props: ['pet', 'row'],
    data: function () { return { step: 0, timer: null }; },
    computed: {
      cols: function () { return 8; },
      rows: function () { return this.pet.spriteVersion === 2 ? 11 : 9; },
      anim: function () { return PET_ANIM[this.row] || PET_ANIM[0]; },
      spriteStyle: function () {
        var url = this.pet.source === 'builtin' ? './pets/' + this.pet.id + '.webp' : '';
        return {
          backgroundImage: url ? 'url(' + url + ')' : 'none',
          backgroundSize: (this.cols * 100) + '% ' + (this.rows * 100) + '%',
          backgroundPosition: ((this.step / (this.cols - 1)) * 100) + '% ' +
            ((this.row / (this.rows - 1)) * 100) + '%'
        };
      }
    },
    watch: {
      row: function () { this.restart(); },
      'pet.id': function () { this.restart(); }
    },
    mounted: function () { this.restart(); },
    beforeUnmount: function () { clearInterval(this.timer); },
    methods: {
      restart: function () {
        var self = this;
        clearInterval(this.timer);
        this.step = 0;
        this.timer = setInterval(function () {
          self.step = (self.step + 1) % self.anim[1];
        }, Math.max(60, this.anim[2] / this.anim[1]));
      }
    },
    template: '<div class="sprite" :data-pet="pet.id" :style="spriteStyle"></div>'
  };

  var PetCard = {
    props: ['p'],
    computed: {
      fav: function () { return !!M.favs[this.p.id]; },
      sel: function () { return M.petsData && this.p.id === M.petsData.selectedId; },
      useLabel: function () {
        return this.sel ? '已设为默认' : this.p.installed ? '启用' : '安装并启用';
      },
      useHtml: function () { return SVG.dl + ' ' + this.useLabel; },
      bg: function () { return PET_BG[this.p.id] || '#eef2fa'; }
    },
    template:
      '<div class="card pet-card">' +
      '<div class="pbanner" :style="{ background: bg }"><pet-sprite :pet="p" :row="0"></pet-sprite></div>' +
      '<div class="pbody">' +
      '<div class="prow"><b>{{ p.displayName }}</b></div>' +
      '<p>{{ p.description || \'\' }}</p>' +
      '<div class="tags"><i v-for="tg in (p.tags || [])" :key="tg">{{ tg }}</i><i>{{ p.spriteVersion === 2 ? 10 : 9 }} 组动作</i></div>' +
      '<div class="prow sub"><span>随助手提供</span><a class="go" :data-pet-detail="p.id" @click.stop="openPetDetail(p.id)">查看详情&nbsp;↗</a></div>' +
      '<div class="prow"><button class="btn dark" :data-pet-use="p.id" @click="petUse(p.id)" v-html="useHtml"></button>' +
      '<button class="heart" :class="{ on: fav }" :data-pet-fav="p.id" @click.stop="petFav(p.id)" v-html="ic(\'heart\')"></button></div>' +
      '</div></div>'
  };

  var PagePets = {
    computed: {
      petList: function () {
        var list = (M.petsData && M.petsData.pets) || [];
        if (M.petTab === 'installed') list = list.filter(function (p) { return p.installed; });
        if (M.petTab === 'fav') list = list.filter(function (p) { return M.favs[p.id]; });
        if (M.petQuery) list = list.filter(function (p) {
          var s = (p.displayName + ' ' + (p.description || '') + ' ' + (p.tags || []).join(',')).toLowerCase();
          return s.indexOf(M.petQuery.toLowerCase()) >= 0;
        });
        return list;
      },
      total: function () { return M.petsData ? M.petsData.pets.length : 0; }
    },
    mounted: function () {
      if (!M.petsData) loadPets();
    },
    methods: {
      importPets: function () {
        invoke('pets:import').then(function (r) {
          if (r) { M.petsData = r; toast('已导入并启用'); }
        }).catch(function (e) { toast(e.message); });
      }
    },
    template:
      '<div class="page"><div class="ph"><div class="pet-head"><span class="pcube" v-html="ic(\'pet\')"></span><b>Codex Pets</b>' +
      '<div class="ptabs"><button v-for="t in PTABS" :key="t[0]" :data-ptab="t[0]" :class="{ on: M.petTab === t[0] }" @click="M.petTab = t[0]">{{ t[1] }}</button></div></div>' +
      '<span class="sp"></span>' +
      '<button class="btn primary" id="petImport" @click="importPets" v-html="ic(\'dl\') + \' 导入宠物包\'"></button></div>' +
      '<div class="card pet-hero"><span class="tagchip">合集</span>' +
      '<h1>AI Codex 宠物</h1><p>为你的 Codex 选一只桌面伙伴，点一下就能安装并设为默认。</p>' +
      '<div class="prow"><span class="tagchip green">{{ total }} 个宠物</span><span class="sp"></span>' +
      '<input type="text" id="petSearch" placeholder="搜索宠物或标签" v-model="M.petQuery"></div></div>' +
      '<div class="pet-grid">' +
      '<template v-if="M.petsData"><pet-card v-for="p in petList" :key="p.id" :p="p"></pet-card></template>' +
      '<p v-else style="color:var(--sub)">正在加载宠物库…</p>' +
      '</div></div>'
  };

  /* ---------- diagnostics ---------- */
  var PageDiag = {
    computed: {
      list: function () {
        return S.diagnostics.length ? S.diagnostics : DIAG_DEFAULT.map(function (d) {
          return { id: d[0], name: d[1], description: d[2], state: 'pending', result: '' };
        });
      },
      running: function () { return this.list.some(function (c) { return c.state === 'run'; }); },
      ended: function () { return !!(S.diagnostics.length && !this.running); }
    },
    methods: {
      runDiag: function () {
        invoke('diagnostics:run').catch(function (e) { toast(e.message); });
      },
      copyReport: function () {
        invoke('diagnostics:copy').then(function () { toast('诊断报告已复制，不包含密钥'); });
      }
    },
    template:
      '<div class="page"><div class="ph"><div><h1>连接诊断</h1><p>哪个工具用不了，点一下就知道问题出在哪一环</p></div><span class="sp"></span>' +
      '<button class="btn" id="copyReport" v-if="ended" @click="copyReport">复制诊断报告</button>' +
      '<button class="btn primary" id="runDiag" :disabled="running" @click="runDiag">{{ running ? \'正在诊断…\' : \'开始诊断\' }}</button></div>' +
      '<div class="diag-list"><div class="card diag-item" v-for="c in list" :key="c.id">' +
      '<span class="dic" :class="c.id"></span>' +
      '<div><b>{{ c.name }}</b><small>{{ c.description }}</small></div>' +
      '<div class="res">{{ c.result || \'\' }}</div>' +
      '<div class="st" :class="c.state"></div></div></div></div>'
  };

  /* ---------- backups ---------- */
  var PageBackups = {
    data: function () { return { busy: '' }; },
    computed: {
      list: function () { return S.backups; }
    },
    mounted: function () {
      invoke('backups:list').then(function (r) { S.backups = r || []; });
    },
    methods: {
      time: function (t) {
        return new Date(t).toLocaleString('zh-CN',
          { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
      },
      toolName: function (id) { return (TOOL_META[id] || {}).name || id; },
      restore: function (b) {
        var self = this;
        self.busy = b.id;
        invoke('backups:restore', [b.tool, b.id]).then(function () {
          toast('已还原 ' + self.toolName(b.tool) + ' 的配置');
          refresh();
        }).catch(function (e) { toast(e.message); })
          .finally(function () { self.busy = ''; });
      }
    },
    template:
      '<div class="page"><div class="ph"><div><h1>配置备份</h1><p>每次写入工具配置前都会自动备份，选一份即可还原</p></div></div>' +
      '<div class="diag-list"><div class="card diag-item" v-for="b in list" :key="b.id">' +
      '<span class="bicon"><img :src="icon(b.tool)"></span>' +
      '<div><b>{{ toolName(b.tool) }}</b>' +
      '<small>{{ time(b.createdAt) }} · {{ b.files.length }} 个文件' +
      '<template v-if="b.original"> · 接入前的初始备份</template></small></div>' +
      '<div class="res">{{ b.files.join(\'，\') }}</div>' +
      '<button class="btn" :disabled="busy === b.id" @click="restore(b)">' +
      '{{ busy === b.id ? \'还原中…\' : \'还原\' }}</button></div>' +
      '<div class="card diag-item empty" v-if="!list.length">还没有备份——接入工具时会自动创建</div>' +
      '</div></div>'
  };

  /* ---------- settings ---------- */
  var PageSettings = {
    methods: {
      onSet: function (key, e) {
        var o = {};
        o[key] = e.target.checked;
        invoke('settings:update', [o]);
      },
      logout: function () {
        invoke('auth:logout').then(function () { toast('已退出'); })
          .catch(function (e) { toast(e.message); });
      },
      checkUpdate: function () {
        invoke('updates:check').then(function (r) {
          toast(r.available ? ('发现新版本 ' + r.version) : (r.message || '已是最新版本'));
        }).catch(function (e) { toast(e.message); });
      }
    },
    template:
      '<div class="page"><div class="ph"><div><h1>设置</h1></div></div>' +
      '<div class="set-grid"><div class="card set-card">' +
      '<div class="srow" v-for="i in SETTING_ITEMS" :key="i[0]"><div><b>{{ i[1] }}</b><p>{{ i[2] }}</p></div>' +
      '<label class="sw"><input type="checkbox" :data-set="i[0]" :checked="!!S.settings[i[0]]" @change="onSet(i[0], $event)"><span></span></label></div>' +
      '</div>' +
      '<div class="card acct-card">' +
      '<div class="acct"><img class="km" :src="TP_MARK">' +
      '<div><b>API 密钥</b><p>{{ (S.auth || {}).key_hint || \'已接入\' }} · 保存在系统钥匙串</p></div>' +
      '<button class="btn danger-out" id="logout" @click="logout">移除密钥</button></div>' +
      '<div class="aline"></div><p class="anote">移除只删除本机保存的密钥，已经接好的工具配置不受影响。</p>' +
      '<div class="aline"></div>' +
      '<div class="acct"><div><b>{{ APP_NAME }} {{ S.version }}</b><p>已是最新版本 {{ S.version }}</p></div>' +
      '<button class="btn" id="checkUpdate" @click="checkUpdate">检查更新</button></div>' +
      '</div></div></div>'
  };

  /* ---------- tool detail (route /tool/:id, nested under /tools) ----------
   * Expand card: the panel springs out of the clicked .tool-card inside
   * PageTools' .tstage; reverse spring on close. */
  var ToolDetail = {
    props: ['id', 'stage'],
    data: function () {
      return { running: false, logOpen: (M.logs[this.id] || []).length > 0 };
    },
    computed: {
      t: function () {
        var self = this;
        return S.tools.find(function (x) { return x.id === self.id; }) || null;
      },
      meta: function () {
        return TOOL_META[this.id] || { name: this.id, kind: '' };
      },
      backups: function () {
        var id = this.id;
        return (S.backups || []).filter(function (b) { return b.tool === id; });
      },
      flow: function () {
        return S.flow && S.flow.tool === this.id ? S.flow : null;
      },
      logs: function () { return M.logs[this.id] || []; },
      run: function () { return M.tests[this.id] || null; },
      probe: function () { return (this.t && this.t.lastProbe) || null; },
      showReply: function () { return !!(this.run || (this.probe && this.probe.reply)); },
      replyModel: function () { return (this.run && this.run.model) || (this.probe && this.probe.model) || '—'; },
      replyState: function () {
        if (this.run) return this.run.state;
        return this.probe && this.probe.ok ? 'ok' : 'fail';
      },
      testing: function () { return !!(this.run && this.run.state === 'run'); },
      runningWait: function () { return this.testing && !this.run.chunks.length; },
      replyChunks: function () {
        if (this.run) return this.run.chunks;
        return this.probe && this.probe.reply ? [this.probe.reply] : [];
      },
      replyFoot: function () {
        if (this.run) {
          var r = this.run;
          if (r.state === 'ok') {
            var s = '✓ ';
            if (r.firstToken != null) s += '首字 ' + Math.round(r.firstToken) + 'ms · ';
            s += '总 ' + Math.round(r.latency || 0) + 'ms';
            if (r.at) s += ' · ' + relTime(r.at);
            return s;
          }
          if (r.state === 'fail') return '✗ ' + (r.message || '连接失败');
          return '';
        }
        var p = this.probe;
        if (!p) return '';
        if (!p.ok) return '✗ ' + (p.message || '');
        var t = '✓ ';
        if (p.firstToken != null) t += '首字 ' + Math.round(p.firstToken) + 'ms · ';
        return t + '总 ' + Math.round(p.latency || 0) + 'ms · ' + relTime(p.at);
      },
      probeText: function () {
        var p = this.t && this.t.lastProbe;
        if (!p) return '还没有测试过';
        return (p.model || modelId(this.t.model) || '—') + ' · ' +
          (p.ok ? '✓' : '✗') + ' ' + (p.message || '') +
          (p.latency != null ? ' · ' + Math.round(p.latency) + 'ms' : '') +
          ' · ' + relTime(p.at);
      }
    },
    mounted: function () {
      var self = this;
      var stage = this.stage || this.$el.parentElement;
      this._stage = stage;
      this._closing = false;
      var sw = stage.clientWidth;
      var availH = function () {
        return Math.max(160, window.innerHeight - stage.getBoundingClientRect().top - 24);
      };
      var w = Math.min(760, sw);
      var to = { l: Math.max(0, (sw - w) / 2), t: 10, w: w, h: availH() };
      this._to = to;
      var card = stage.querySelector('.tool-card[data-tool="' + this.id + '"]');
      this._card = card;
      if (card) {
        var s = stage.getBoundingClientRect(), r = card.getBoundingClientRect();
        this._from = { l: r.left - s.left, t: r.top - s.top, w: r.width, h: r.height };
        card.classList.add('src-hide');
        this._l = spring(this._from.l, 260, 26);
        this._t = spring(this._from.t, 260, 26);
        this._w = spring(this._from.w, 260, 26);
        this._h = spring(this._from.h, 260, 26);
        this._l.t = to.l; this._t.t = to.t; this._w.t = to.w;
        this._s = spring(1, 320, 26);
        this._o = spring(1, 320, 26);
      } else {
        // deep link — no source card: pop from the target rect
        this._from = null;
        this._l = spring(to.l, 260, 26); this._t = spring(to.t, 260, 26);
        this._w = spring(to.w, 260, 26); this._h = spring(160, 260, 26);
        this._h.t = to.h;
        this._s = spring(0.96, 320, 26); this._s.t = 1;
        this._o = spring(0, 320, 26); this._o.t = 1;
      }
      this._c = spring(0, 300, 26); // content fade, released ~60% in
      this._last = performance.now();
      this._loop = function (now) { self._frame(now); };
      this._raf = requestAnimationFrame(this._loop);
      this.$nextTick(function () { self._measure(); });
      this._timer = setTimeout(function () { self._c.t = 1; }, REDUCED ? 0 : 260);
      this._onResize = function () {
        var st = self._stage;
        var w2 = Math.min(760, st.clientWidth);
        self._to = { l: Math.max(0, (st.clientWidth - w2) / 2), t: 10,
          w: w2, h: Math.max(160, window.innerHeight - st.getBoundingClientRect().top - 24) };
        self._measure();
        ['_l', '_t', '_w', '_h'].forEach(function (k) {
          self[k].t = self._to[{ _l: 'l', _t: 't', _w: 'w', _h: 'h' }[k]];
          self[k].x = self[k].t; self[k].v = 0;
        });
      };
      window.addEventListener('resize', this._onResize);
      M.detailClose = function () { self.requestClose(); };
      if (REDUCED) {
        var self2 = this;
        ['_l', '_t', '_w', '_h', '_s', '_o', '_c'].forEach(function (k) {
          self2[k].x = self2[k].t; self2[k].v = 0;
        });
      }
    },
    unmounted: function () {
      cancelAnimationFrame(this._raf);
      clearTimeout(this._timer);
      window.removeEventListener('resize', this._onResize);
      if (this._card) this._card.classList.remove('src-hide');
      if (M.detailClose) M.detailClose = null;
    },
    methods: {
      _measure: function () {
        var sc = this.$refs.scroll;
        if (!sc) return;
        var want = sc.scrollHeight + 2;
        var maxH = Math.max(160, window.innerHeight - this._stage.getBoundingClientRect().top - 24);
        this._to.h = Math.min(maxH, Math.max(this._from ? this._from.h : 160, want));
        if (!this._closing) this._h.t = this._to.h;
      },
      _apply: function (el) {
        el.style.left = this._l.x.toFixed(1) + 'px';
        el.style.top = this._t.x.toFixed(1) + 'px';
        el.style.width = this._w.x.toFixed(1) + 'px';
        el.style.height = this._h.x.toFixed(1) + 'px';
        el.style.opacity = clamp01(this._o.x).toFixed(3);
        el.style.transform = 'scale(' + this._s.x.toFixed(4) + ')';
        var sc = this.$refs.scroll;
        if (sc) sc.style.opacity = clamp01(this._c.x).toFixed(3);
      },
      _frame: function (now) {
        var dt = Math.min((now - this._last) / 1000, 1 / 30);
        this._last = now;
        var ks = ['_l', '_t', '_w', '_h', '_s', '_o', '_c'], i;
        for (i = 0; i < ks.length; i++) springStep(this[ks[i]], dt);
        var el = this.$el;
        if (el && el.style) this._apply(el);
        if (this._closing) {
          var settled = true;
          for (i = 0; i < ks.length; i++) {
            if (Math.abs(this[ks[i]].x - this[ks[i]].t) > 1 || Math.abs(this[ks[i]].v) > 12) settled = false;
          }
          if (settled) { closeDetail(); return; }
        }
        this._raf = requestAnimationFrame(this._loop);
      },
      requestClose: function () {
        if (this._closing) return;
        this._closing = true;
        this._c.t = 0;
        if (this._from) {
          this._l.t = this._from.l; this._t.t = this._from.t;
          this._w.t = this._from.w; this._h.t = this._from.h;
        } else {
          this._s.t = 0.96; this._o.t = 0;
        }
        if (REDUCED) { if (this._card) this._card.classList.remove('src-hide'); closeDetail(); }
      },
      close: function () { this.requestClose(); },
      backupLabel: function (b) {
        var d = new Date(b.createdAt);
        return (d.getMonth() + 1) + '-' + d.getDate() + ' ' +
          String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0') +
          ' · ' + b.files.length + ' 个文件';
      },
      connect: function () {
        var id = this.id, meta = this.meta, self = this;
        this.running = true;
        tlog(id, '正在连接 ' + meta.name + ' …');
        invoke('tools:connect', [[id], false]).then(function () {
          tlog(id, '接入完成：' + meta.restart);
          toast('已接入 ' + meta.name);
        }).catch(function (e) { tlog(id, '接入失败：' + e.message, true); })
          .finally(function () { self.running = false; });
      },
      test: function () {
        var id = this.id;
        runTest(id).catch(function () {});
      },
      openFolder: function () {
        invoke('tools:open-folder', [this.id]).catch(function (e) { toast(e.message); });
      },
      restore: function () {
        var id = this.id;
        var bid = this.$refs.dBackup.value;
        invoke('backups:restore', [id, bid]).then(function () {
          tlog(id, '已还原所选备份'); toast('已还原'); refresh();
        }).catch(function (e) { tlog(id, '还原失败：' + e.message, true); });
      }
    },
    template:
      '<div class="xpanel" v-if="t">' +
      '<div class="xscroll" ref="scroll">' +
      '<div class="xhead"><div class="ticon"><img :src="icon(id)"></div>' +
      '<div class="xt"><b>{{ meta.name }}</b><small>{{ meta.kind }} · {{ meta.restart }}</small></div>' +
      '<span class="badge" :class="t.status">{{ STATUS[t.status] }}</span>' +
      '<button class="xclose" id="dClose" @click="close">✕</button></div>' +
      '<p class="xver" v-if="t.version || t.message">{{ t.version }}' +
      '<span v-if="t.message" style="color:var(--orange)">{{ t.version ? \' · \' : \'\' }}{{ t.message }}</span></p>' +
      '<div class="dsec"><h4>模型与测试</h4>' +
      '<div class="drow"><model-select v-if="t.models && t.models.length" :tool="t"></model-select>' +
      '<span v-else style="color:var(--sub);font-size:12px">接入后可选择模型</span>' +
      '<button class="btn" id="dTest" :disabled="t.status !== \'ok\' || testing" @click="test">{{ testing ? \'测试中…\' : \'测试连接\' }}</button></div>' +
      '<div class="reply" v-if="showReply">' +
      '<div class="rbub user">hi</div>' +
      '<div class="rbub model"><div class="rlab">{{ replyModel }}</div>' +
      '<div class="rtext" v-if="runningWait"><span class="rdots"><i></i><i></i><i></i></span> 正在等待 {{ replyModel }} 回复…</div>' +
      '<div class="rtext" v-else><span class="chunk" v-for="(c, i) in replyChunks" :key="i">{{ c }}</span><i class="caret" v-if="testing"></i></div>' +
      '<div class="rfoot" :class="{ bad: replyState === \'fail\' }" v-if="replyState !== \'run\' && replyFoot">{{ replyFoot }}</div>' +
      '</div></div>' +
      '<div class="probe" v-else :class="{ bad: t.lastProbe && !t.lastProbe.ok }">{{ probeText }}</div></div>' +
      '<div class="dsec"><h4>配置</h4>' +
      '<div class="files"><template v-for="(f, i) in (t.files || [])" :key="i"><br v-if="i > 0">{{ f }}</template></div>' +
      '<div class="drow">' +
      '<button class="btn primary" id="dConnect" :disabled="!(t.installed && !running)" @click="connect">{{ t.status === \'ok\' ? \'重新连接\' : \'连接\' }}</button>' +
      '<button class="btn" id="dFolder" @click="openFolder">打开配置目录</button></div>' +
      '<div class="backup-row" v-if="backups.length">备份 ' +
      '<select id="dBackup" ref="dBackup"><option v-for="b in backups" :key="b.id" :value="b.id">{{ backupLabel(b) }}</option></select>' +
      '<button class="btn" id="dRestore" @click="restore">还原</button></div></div>' +
      '<step-progress v-if="flow" :steps="STEPS" :cur="Math.max(0, (flow || {}).step || 0)" :state="(flow || {}).state || \'run\'"></step-progress>' +
      '<div class="dlog"><button class="lhead" @click="logOpen = !logOpen">日志 ({{ logs.length }})' +
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" :style="{ transform: logOpen ? \'rotate(180deg)\' : \'\' }"><path d="m6 9 6 6 6-6"/></svg></button>' +
      '<div class="log" v-show="logOpen"><div v-for="(l, i) in logs" :key="i" :class="l.err ? \'err\' : \'ok\'">{{ l.text }}</div>' +
      '<div v-if="!logs.length" style="color:var(--sub)">还没有记录</div></div></div>' +
      '</div></div>'
  };

  /* ---------- step progress (db-step-progress motion study) ----------
     Filling bar on a spring, dots that become drawn checks, current dot pops. */
  var StepProgress = {
    props: ['steps', 'cur', 'state'],
    computed: {
      N: function () { return this.steps.length; }
    },
    mounted: function () {
      var self = this;
      this._L = spring(0, 160, 22);
      this._ds = this.steps.map(function () {
        return { f: spring(0, 260, 24), c: spring(0, 140, 20),
          p: spring(1, 500, 14), on: spring(0, 300, 26), err: false };
      });
      this._last = performance.now();
      this._loop = function (now) { self._frame(now); };
      this.retarget(true);
      this._raf = requestAnimationFrame(this._loop);
    },
    unmounted: function () { cancelAnimationFrame(this._raf); },
    watch: {
      cur: function () { this.retarget(false); },
      state: function () { this.retarget(false); }
    },
    methods: {
      pct: function (i) { return (this.N > 1 ? i / (this.N - 1) * 100 : 0) + '%'; },
      retarget: function (snap) {
        var N = this.N, doneAll = this.state === 'done';
        var cur = doneAll ? N - 1 : Math.max(0, Math.min(N - 1, this.cur == null ? 0 : this.cur));
        this._L.t = doneAll ? 1 : (N > 1 ? cur / (N - 1) : 0);
        var self = this;
        this._ds.forEach(function (s, i) {
          var done = doneAll || i < cur;
          s.f.t = done ? 1 : 0;
          s.err = !done && i === cur && self.state === 'fail';
          s.on.t = i === cur && !done ? 1 : 0;
          if (done) { setTimeout(function () { s.c.t = 1; }, 120); } else { s.c.t = 0; }
        });
        var d = this._ds[cur];
        if (d && !snap) { d.p.x = .7; d.p.v = 6; }
        if (snap) {
          this._ds.forEach(function (s) {
            s.f.x = s.f.t; s.c.x = s.c.t; s.on.x = s.on.t; s.p.x = 1;
          });
          this._L.x = this._L.t;
        }
      },
      _frame: function (now) {
        var dt = Math.min((now - this._last) / 1000, 1 / 30);
        this._last = now;
        springStep(this._L, dt);
        var dots = this.$el && this.$el.querySelectorAll ? this.$el.querySelectorAll('.sp-dot') : [];
        var names = this.$el && this.$el.querySelectorAll ? this.$el.querySelectorAll('.sp-name') : [];
        var bar = this.$el && this.$el.querySelector ? this.$el.querySelector('.sp-bar') : null;
        var self = this;
        this._ds.forEach(function (s, i) {
          springStep(s.f, dt); springStep(s.c, dt); springStep(s.p, dt); springStep(s.on, dt);
          var el = dots[i];
          if (!el) return;
          var f = clamp01(s.f.x), o = clamp01(s.on.x);
          el.style.background = s.err ? ('color-mix(in srgb, #F04438 ' + (f * 100).toFixed(1) + '%, #E8ECF4)')
            : f > .01 ? ('color-mix(in srgb, #10B981 ' + (f * 100).toFixed(1) + '%, #E8ECF4)')
            : o > .01 ? ('color-mix(in srgb, #6E29F6 ' + (o * 100).toFixed(1) + '%, #E8ECF4)') : '';
          var num = el.querySelector('b');
          if (num) { num.style.opacity = (1 - f).toFixed(3); num.style.color = o > .5 ? '#fff' : ''; }
          var path = el.querySelector('path');
          if (path) path.style.strokeDashoffset = (22 * (1 - clamp01(s.c.x))).toFixed(2);
          el.style.transform = 'scale(' + s.p.x.toFixed(4) + ')';
          if (names[i]) {
            var upto = self.state === 'done' ? self.N - 1 : Math.max(0, self.cur || 0);
            names[i].classList.toggle('on', i <= upto);
          }
        });
        if (bar) bar.style.width = (clamp01(this._L.x) * 100).toFixed(3) + '%';
        this._raf = requestAnimationFrame(this._loop);
      }
    },
    template:
      '<div class="sprog"><div class="track"><span class="bar sp-bar"></span>' +
      '<span class="dot sp-dot" v-for="(s, i) in steps" :key="i" :style="{ left: pct(i) }">' +
      '<b>{{ i + 1 }}</b>' +
      '<svg viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5" fill="none" stroke="#fff" ' +
      'stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round" ' +
      'stroke-dasharray="22" stroke-dashoffset="22"/></svg></span></div>' +
      '<div class="names"><span class="sp-name" v-for="(s, i) in steps" :key="i" ' +
      ':style="{ left: pct(i) }">{{ s }}</span></div></div>'
  };

  /* ---------- connect overlay: plan preview + step progress in one ----------
     phases: preview -> run -> done | fail. Per-tool accordion rows with
     vertical sub-steps, plan details under 「写入」, single-tool retry. */
  var ConnectOverlay = {
    data: function () { return { preOpen: {} }; },
    computed: {
      c: function () { return M.conn; },
      phase: function () { return M.conn ? M.conn.phase : ''; },
      ids: function () {
        return M.conn ? M.conn.queue.slice().sort(toolSort) : [];
      },
      N: function () { return this.ids.length; },
      pos: function () {
        if (!M.conn) return 0;
        var self = this;
        return this.ids.filter(function (id) { return self.ts(id).state === 'done'; }).length;
      },
      title: function () {
        if (!M.conn) return '';
        if (this.phase === 'preview') return '接入前会改这些地方';
        if (this.phase === 'run') return '正在接入';
        var nf = this.ids.filter(function (id) {
          return M.conn.tools[id] && M.conn.tools[id].state === 'fail';
        }).length;
        if (!nf) return '接入完成';
        return nf >= this.N ? '接入未完成' : '部分完成';
      },
      closeLabel: function () {
        return this.phase === 'run' ? '后台继续'
          : this.phase === 'done' ? '完成' : '关闭';
      }
    },
    mounted: function () {
      var self = this;
      this._hs = {};
      this._last = performance.now();
      this._loop = function (now) { self._frame(now); };
      this._raf = requestAnimationFrame(this._loop);
    },
    unmounted: function () { cancelAnimationFrame(this._raf); },
    methods: {
      _frame: function (now) {
        var dt = Math.min((now - this._last) / 1000, 1 / 30);
        this._last = now;
        var bodies = this.$el && this.$el.querySelectorAll ? this.$el.querySelectorAll('.crbody') : [];
        var self = this;
        Array.prototype.forEach.call(bodies, function (el) {
          var id = el.getAttribute('data-id');
          var s = self._hs[id] || (self._hs[id] = spring(0, 300, 26));
          var inner = el.firstElementChild;
          var ts = self.ts(id);
          s.t = ts.open && inner ? inner.scrollHeight : 0;
          if (REDUCED) { s.x = s.t; s.v = 0; }
          springStep(s, dt);
          el.style.height = Math.max(0, s.x).toFixed(1) + 'px';
        });
        this._raf = requestAnimationFrame(this._loop);
      },
      close: function () { M.conn = null; },
      ts: function (id) {
        return (M.conn && M.conn.tools[id]) || { state: '', step: -1, stepAt: {}, elapsed: 0, open: false };
      },
      toggle: function (id) {
        var ts = M.conn.tools[id];
        if (!ts) return;
        ts.open = !ts.open;
        ts.manual = true;
      },
      retryOne: function (id) {
        var conn = M.conn;
        if (!conn || conn.retry) return;
        conn.tools[id] = connRow();
        conn.retry = id;
        invoke('tools:connect', [[id], false]).then(function () {
          if (sameConn(conn)) { conn.retry = ''; connRecompute(conn); }
        }).catch(function (e) {
          if (sameConn(conn)) {
            conn.retry = '';
            var ts = conn.tools[id];
            ts.state = 'fail'; ts.failMsg = e.message || '接入失败';
            connRecompute(conn);
          } else toast(e.message);
        });
      },
      chipText: function (id) {
        var s = this.ts(id).state;
        return s === 'done' ? '完成' : s === 'fail' ? '失败'
          : s === 'run' ? '接入中' : '待开始';
      },
      vstate: function (id, i) {
        var ts = this.ts(id);
        if (ts.state === 'done') return 'done';
        if (ts.state === 'fail') return i < ts.step ? 'done' : i === ts.step ? 'fail' : '';
        if (ts.state === 'run') return i < ts.step ? 'done' : i === ts.step ? 'run' : '';
        return '';
      },
      stepMs: function (id, i) {
        var ts = this.ts(id);
        var a = ts.stepAt[i];
        if (a == null) return '';
        var b = ts.stepAt[i + 1];
        if (b == null) {
          if (ts.state === 'done' || ts.state === 'fail') return '';
          return Math.max(0, Math.round(ts.elapsed - (a - (ts.t0 || a)))) + 'ms';
        }
        return Math.max(0, Math.round(b - a)) + 'ms';
      },
      elapsed: function (id) {
        var e = this.ts(id).elapsed;
        return e ? (e / 1000).toFixed(1) + 's' : '';
      },
      pmeta: function (id) { return TOOL_META[id] || { name: id, kind: '' }; },
      pdata: function (id) { return (M.conn.plan.data && M.conn.plan.data[id]) || {}; },
      pfiles: function (id) {
        var p = this.pdata(id);
        var t = S.tools.find(function (x) { return x.id === id; }) || {};
        var files = Array.isArray(p.files) ? p.files
          : (t.files || []).map(function (f) { return { path: f, exists: true }; });
        return files.map(function (f) {
          return {
            path: typeof f === 'string' ? f : (f && f.path) || '',
            exists: typeof f === 'string' ? true : !!f.exists,
            size: (f && typeof f === 'object') ? f.size : null
          };
        });
      },
      fileSize: function (f) { return f.size != null ? fmtSize(f.size) : '已存在'; },
      pchanges: function (id) {
        var p = this.pdata(id);
        return Array.isArray(p.changes) ? p.changes
          : (p.changes ? [p.changes] : ['写入 TokenPlus 线路配置（写入前自动备份）']);
      },
      pmodels: function (id) {
        var p = this.pdata(id);
        return Array.isArray(p.models) ? p.models.map(function (m) {
          var s = modelLabel(m), d = modelDesc(m);
          return s + (d ? '（' + d + '）' : '');
        }).join('、') : '';
      },
      ppreview: function (id) {
        var p = this.pdata(id);
        return p.preview == null ? ''
          : (typeof p.preview === 'string' ? p.preview : JSON.stringify(p.preview, null, 2));
      },
      doneTail: function (id) {
        var meta = TOOL_META[id] || { name: id, restart: '' };
        var ts = this.ts(id);
        return ts.state === 'fail' ? '接入失败，可展开后点「重试这一个」' : (meta.restart || '已生效');
      }
    },
    template:
      '<div class="overlay conn-ov" v-if="c" @click.self="close">' +
      '<div class="panel conn wide">' +
      '<h2>{{ title }}<span class="cq">{{ phase === \'preview\' ? N + \' 个工具\' : pos + \' / \' + N }}</span></h2>' +
      '<p class="sub" v-if="phase === \'preview\'">列出的改动写入前都会先自动备份，随时可以还原。</p>' +
      '<p class="sub" v-if="c.plan.loading">正在读取接入计划…</p>' +
      '<p class="sub" v-else-if="c.plan.error" style="color:var(--orange)">没有拿到完整计划（{{ c.plan.error }}），下面是按本机检测到的改动估算的。</p>' +
      '<step-progress v-if="phase !== \'preview\'" :steps="STEPS" :cur="c.step" :state="phase"></step-progress>' +
      '<div class="crow" v-for="id in ids" :key="id">' +
      '<button class="crhead" :data-crow="id" @click="toggle(id)">' +
      '<img :src="icon(id)"><b>{{ pmeta(id).name }}</b>' +
      '<span class="chip" :class="ts(id).state"><i></i>{{ chipText(id) }}</span>' +
      '<span class="cela">{{ elapsed(id) }}</span>' +
      '<svg class="chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" ' +
      'stroke-linecap="round" stroke-linejoin="round" :style="{ transform: ts(id).open ? \'rotate(180deg)\' : \'\' }"><path d="m6 9 6 6 6-6"/></svg>' +
      '</button>' +
      '<div class="crbody" :data-id="id"><div class="crin">' +
      '<template v-if="phase !== \'preview\'">' +
      '<template v-for="(sn, i) in STEPS" :key="i">' +
      '<div class="vst" :class="vstate(id, i)"><span class="vic">' +
      '<svg viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5" fill="none" stroke="currentColor" ' +
      'stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" stroke-dasharray="22" stroke-dashoffset="22"/></svg></span>' +
      '<span class="vn">{{ sn }}</span><span class="vm">{{ stepMs(id, i) }}</span></div>' +
      '<div class="verr" v-if="ts(id).state === \'fail\' && ts(id).step === i">{{ ts(id).failMsg || \'接入失败\' }}' +
      '<button class="btn" :data-retry="id" :disabled="!!c.retry" @click.stop="retryOne(id)">重试这一个</button></div>' +
      '<div class="vplan" v-if="i === 3">' +
      '<div class="pfiles"><div class="pf" v-for="(f, fi) in pfiles(id)" :key="fi" :class="{ new: !f.exists }"><span>{{ f.path }}</span>' +
      '<i v-if="f.exists">{{ fileSize(f) }}</i><i class="nw" v-else>将新建</i></div>' +
      '<div class="pf" v-if="!pfiles(id).length"><span>（无需写文件）</span></div></div>' +
      '<ul class="pch"><li v-for="(ch, ci) in pchanges(id)" :key="ci">{{ ch }}</li></ul>' +
      '<div class="pmodels" v-if="pmodels(id)">接入后可用模型：{{ pmodels(id) }}</div>' +
      '<template v-if="ppreview(id)"><button class="prev-t" @click.stop="preOpen[id] = !preOpen[id]">{{ preOpen[id] ? \'收起预览\' : \'查看写入预览\' }}</button>' +
      '<pre class="pplan-pre" v-show="preOpen[id]">{{ ppreview(id) }}</pre></template>' +
      '</div></template>' +
      '</template>' +
      '<template v-else>' +
      '<div class="vplan">' +
      '<div class="pfiles"><div class="pf" v-for="(f, fi) in pfiles(id)" :key="fi" :class="{ new: !f.exists }"><span>{{ f.path }}</span>' +
      '<i v-if="f.exists">{{ fileSize(f) }}</i><i class="nw" v-else>将新建</i></div>' +
      '<div class="pf" v-if="!pfiles(id).length"><span>（无需写文件）</span></div></div>' +
      '<ul class="pch"><li v-for="(ch, ci) in pchanges(id)" :key="ci">{{ ch }}</li></ul>' +
      '<div class="pmodels" v-if="pmodels(id)">接入后可用模型：{{ pmodels(id) }}</div>' +
      '<template v-if="ppreview(id)"><button class="prev-t" @click.stop="preOpen[id] = !preOpen[id]">{{ preOpen[id] ? \'收起预览\' : \'查看写入预览\' }}</button>' +
      '<pre class="pplan-pre" v-show="preOpen[id]">{{ ppreview(id) }}</pre></template>' +
      '</div></template>' +
      '</div></div></div>' +
      '<p class="sub" v-if="!ids.length">没有需要接入的工具。</p>' +
      '<div class="cdone" v-if="phase === \'done\'"><div v-for="id in ids" :key="id">{{ pmeta(id).name }}：{{ doneTail(id) }}</div></div>' +
      '<div class="cfail" v-else-if="phase === \'fail\'">{{ c.message || \'有工具接入失败，展开对应条目可重试。\' }}</div>' +
      '<div class="actions">' +
      '<button v-if="phase === \'preview\'" class="btn primary" id="pStart" :disabled="c.plan.loading || !ids.length" @click="startConnectAll">开始接入</button>' +
      '<span class="sp"></span>' +
      '<button class="btn" :class="{ primary: phase === \'done\' || phase === \'fail\' }" id="cClose" @click="close">{{ closeLabel }}</button></div>' +
      '</div></div>'
  };

  /* ---------- pet detail overlay (not routed — preview popup) ---------- */
  var PetDetail = {
    computed: {
      pet: function () {
        if (!M.petDetail || !M.petsData) return null;
        return M.petsData.pets.find(function (x) { return x.id === M.petDetail; }) || null;
      },
      fav: function () { return this.pet ? !!M.favs[this.pet.id] : false; },
      sel: function () {
        return !!(this.pet && M.petsData && this.pet.id === M.petsData.selectedId);
      },
      useLabel: function () {
        return this.sel ? '已设为默认' : (this.pet && this.pet.installed) ? '启用' : '安装并启用';
      },
      useHtml: function () { return SVG.dl + ' ' + this.useLabel; },
      bg: function () { return this.pet ? (PET_BG[this.pet.id] || '#eef2fa') : '#eef2fa'; }
    },
    methods: {
      close: function () { M.petDetail = null; },
      setRow: function (i) { M.petDetailRow = i; },
      use: function () {
        var p = this.pet;
        invoke('pets:use', [p.id]).then(function (r) {
          if (r) M.petsData = r;
          else { p.installed = true; M.petsData.selectedId = p.id; }
          toast('已将「' + p.displayName + '」设为默认宠物');
        }).catch(function (e) { toast(e.message); });
      }
    },
    template:
      '<div class="overlay pet-ov" v-if="pet" @click.self="close">' +
      '<div class="panel pdetail">' +
      '<div class="pbanner" :style="{ background: bg }">' +
      '<pet-sprite class="big" :pet="pet" :row="M.petDetailRow"></pet-sprite>' +
      '<button class="px" @click="close">✕</button></div>' +
      '<div class="pbody">' +
      '<div class="prow"><b>{{ pet.displayName }}</b></div>' +
      '<p>{{ pet.description || \'\' }}</p>' +
      '<div class="tags"><i v-for="tg in (pet.tags || [])" :key="tg">{{ tg }}</i><i>{{ pet.spriteVersion === 2 ? 10 : 9 }} 组动作</i></div>' +
      '<div class="animchips"><button v-for="(a, i) in PET_ANIM" :key="i" :data-anim="i" :class="{ on: i === M.petDetailRow }" @click="setRow(i)">{{ a[0] }}</button></div>' +
      '<div class="prow"><button class="btn dark" :data-pet-use="pet.id" @click="use" v-html="useHtml"></button>' +
      '<button class="heart" :class="{ on: fav }" :data-pet-fav="pet.id" @click="petFav(pet.id)" v-html="ic(\'heart\')"></button></div>' +
      '</div></div></div>'
  };

  /* ================= router ================= */
  function authedRoute() { return authed(); }
  var router = createRouter({
    history: createWebHashHistory(),
    routes: [
      { path: '/login', component: PageLogin },
      {
        path: '/', component: AppShell,
        children: [
          { path: '', redirect: '/home' },
          { path: 'home', component: PageHome },
          {
            path: 'tools', component: PageTools,
            children: [
              { path: '/tool/:id', name: 'tool', component: ToolDetail, props: true }
            ]
          },
          { path: 'models', component: PageModels },
          { path: 'pets', component: PagePets },
          { path: 'diag', component: PageDiag },
          { path: 'backups', component: PageBackups },
          { path: 'settings', component: PageSettings },
          { path: '/:pathMatch(.*)*', redirect: '/home' }
        ]
      }
    ]
  });
  router.beforeEach(function (to) {
    if (!S.ready) return true; // bootstrap pending — stay on whatever resolved route; root gates render
    if (!authed() && to.path !== '/login') return '/login';
    if (authed() && to.path === '/login') return '/home';
    if (to.name === 'tool' && !S.tools.find(function (x) { return x.id === to.params.id; })) return '/tools';
    return true;
  });
  // auth/bootstrap arriving after mount: enforce the right route then too
  watch(function () { return S.ready + '|' + authed(); }, function () {
    if (!S.ready) return;
    var r = router.currentRoute.value;
    if (!authed() && r.path !== '/login') { router.replace('/login'); return; }
    if (authed() && r.path === '/login') { router.replace('/home'); return; }
    if (r.name === 'tool' && !S.tools.find(function (x) { return x.id === r.params.id; })) {
      router.replace('/tools');
    }
  });

  /* ================= Vue app ================= */
  var app = createApp({
    mounted: function () {
      document.addEventListener('keydown', escHandler);
    },
    template:
      '<router-view v-if="S.ready"></router-view>' +
      '<pet-detail></pet-detail><connect-overlay></connect-overlay>'
  });
  app.config.errorHandler = function (e) {
    window.__errs += 'vue: ' + ((e && e.message) || e || 'error') + '\n';
  };
  var GP = app.config.globalProperties;
  Object.assign(GP, {
    S: S, M: M, STEPS: STEPS, NAV: NAV, SVG: SVG, PET_ANIM: PET_ANIM,
    PET_BG: PET_BG, TOOL_META: TOOL_META, TOOL_ORDER: TOOL_ORDER, STATUS: STATUS,
    DIAG_DEFAULT: DIAG_DEFAULT, DIAG_MARKS: DIAG_MARKS, PTABS: PTABS,
    SETTING_ITEMS: SETTING_ITEMS, TP_MARK: TP_MARK, APP_NAME: APP_NAME,
    fmtSize: fmtSize, modelId: modelId, modelLabel: modelLabel, modelDesc: modelDesc,
    authed: authedRoute, accountName: accountName, needed: needed, connected: connected,
    invoke: invoke, toast: toast, refresh: refresh, loadPets: loadPets,
    openDetail: openDetail, openPlan: openPlan, startConnectAll: startConnectAll,
    petUse: petUse, petFav: petFav, openPetDetail: openPetDetail,
    tlog: tlog, relTime: relTime, runTest: runTest,
    icon: icon, ic: function (k) { return SVG[k]; }
  });
  app.component('nav-item', NavItem);
  app.component('app-shell', AppShell);
  app.component('app-toast', AppToast);
  app.component('page-login', PageLogin);
  app.component('page-home', PageHome);
  app.component('spring-num', SpringNum);
  app.component('stat-ring', StatRing);
  app.component('page-tools', PageTools);
  app.component('tool-card', ToolCard);
  app.component('spring-select', SpringSelect);
  app.component('model-select', ModelSelect);
  app.component('page-models', PageModels);
  app.component('pet-sprite', PetSprite);
  app.component('pet-card', PetCard);
  app.component('page-pets', PagePets);
  app.component('page-diag', PageDiag);
  app.component('page-backups', PageBackups);
  app.component('page-settings', PageSettings);
  app.component('tool-detail', ToolDetail);
  app.component('step-progress', StepProgress);
  app.component('connect-overlay', ConnectOverlay);
  app.component('pet-detail', PetDetail);
  app.use(router);

  /* ================= events ================= */
  on('tools:changed', function (tools) { S.tools = tools; });
  on('connect:progress', function (flow) {
    S.flow = flow;
    if (flow && flow.tool && flow.state === 'run') {
      tlog(flow.tool, '· ' + (STEPS[flow.step] || '') + (flow.message ? '：' + flow.message : ''));
    }
    if (flow.state === 'fail' && flow.message) toast(flow.message.slice(0, 120));
    if (M.conn) connUpdate(flow);
  });
  on('tools:test-chunk', function (e) {
    if (!e || !e.tool) return;
    var run = M.tests[e.tool];
    if (!run || run.runId !== e.runId) return;
    if (e.state === 'start') {
      run.model = e.model || run.model;
    } else if (e.delta != null) {
      run.chunks.push(e.delta);
      run.text += e.delta;
    } else if (e.state === 'end') {
      finalizeTest(run, e);
    }
  });
  on('selfcheck:changed', function (r) { S.selfCheck = r; });
  on('diagnostics:changed', function (d) { S.diagnostics = d; });
  on('auth:changed', function (a) {
    S.auth = a || { phase: 'out' };
    if (authed()) refresh();
  });
  on('account:changed', function (a) { S.account = a; });
  on('app:bootstrap', applyBootstrap);

  /* Esc closes whichever overlay is open (dropdown handles its own Esc first) */
  function escHandler(e) {
    if (e.key !== 'Escape' && e.keyCode !== 27) return;
    if (M.dropdownOpen) return;
    if (M.petDetail) { M.petDetail = null; }
    else if (M.conn) { M.conn = null; }
    else if (M.detailClose) { M.detailClose(); }
  }

  /* ================= mock ================= */
  function mockInvoke(channel, args) {
    var mtools = TOOL_ORDER.map(function (id, i) {
      return {
        id: id, status: ['todo', 'ok', 'missing', 'ok', 'missing', 'todo'][i],
        installed: i !== 2 && i !== 4, supported: true, running: false,
        version: i === 0 ? 'claude 2.0.31' : '', model: i === 1 ? 'claude-sonnet-4-5' : '',
        models: i === 1 ? ['claude-sonnet-4-5', 'claude-opus-4-1', 'claude-haiku-4-0'] : [],
        files: ['~/mock/' + id], message: i === 2 ? '请先安装 Codex' : '',
        lastProbe: i === 1 ? { at: Date.now() - 480000, ok: true,
          message: '连接正常', latency: 812, model: 'claude-sonnet-4-5',
          firstToken: 300, reply: 'Hi! How can I help you today?' } : null
      };
    });
    var mock = {
      'app:bootstrap': {
        auth: { phase: 'in', user: { username: 'demo@example.com' }, key_hint: 'sk-…test' },
        tools: mtools, account: {
          user: {}, balance: { amount: '¥ 78.49' }, today: { amount: '¥ 0.00' },
          recent: [], daily: []
        },
        accountError: '', settings: { boot: true, tray: true, watch: true, update: true },
        backups: [{ id: '1-x', tool: 'codex', createdAt: Date.now() - 86400000, files: ['a.toml'], original: false }],
        version: '0.1.4', platform: 'darwin', deviceName: 'caideMacBook-Pro.local',
        selfCheck: { checkedAt: Date.now() - 3600000, running: false, issues: [], fixed: [] },
        models: { count: 38, savedAt: Date.now() - 540000 }
      },
      'tools:refresh': mtools,
      'pets:list': {
        pets: ['yituo-ember|赤柚邮差|背着小邮包的小熊猫，跑起来时，条纹尾巴总比围巾慢半拍。|小熊猫,动物',
          'yituo-byte|薄荷小栈|一台穿着橘色雨靴的迷你终端。|机器人,复古',
          'yituo-orbit|云游鲸|枕着一朵云漂过桌面的蓝鲸。|鲸鱼,幻想',
          'yituo-luna|月芽|紫色星帽底下藏着一对长耳朵。|兔子,魔法',
          'yituo-jade|青瓷小龙|青瓷色的小龙，金角和云鬃之间还留着雾气。|东方,神兽',
          'yituo-sprout|竹芽团子|戴着嫩芽帽的熊猫园丁。|熊猫,田园',
          'yituo-nova|星图奈奈|披着金色斗篷的观星员。|人物,星空',
          'yituo-toast|焦糖可颂|短腿面包师，一听到烤箱响就系上红领巾。|柯基,烘焙'].map(function (s) {
            var p = s.split('|');
            return { id: p[0], displayName: p[1], description: p[2], tags: p[3].split(','), source: 'builtin', installed: false, spriteVersion: 1 };
          }),
        selectedId: null
      },
      'selfcheck:run': { checkedAt: Date.now(), running: false, issues: [{ id: 'x', text: '.zshrc 里的 OPENAI_BASE_URL 会让工具绕过 TokenPlus', fixable: true }], fixed: [] },
      'diagnostics:run': DIAG_DEFAULT.map(function (d, i) {
        return { id: d[0], name: d[1], description: d[2], state: i === 3 ? 'fail' : 'done', result: i === 3 ? '连接失败' : '正常' };
      }),
      'tools:plan': {
        'claude-code': {
          files: [
            { path: '~/.claude/settings.json', exists: true, size: 2140 },
            { path: '~/.claude.json', exists: true, size: 18326 },
            { path: '~/.tokenplus/backups/claude-code/', exists: false }
          ],
          changes: [
            '备份现有配置到 ~/.tokenplus/backups/',
            '把 ANTHROPIC_BASE_URL 指向 TokenPlus 的 Claude 线路',
            '写入本机登录密钥的引用，密钥本身不落地'
          ],
          preview: '{\n  "env": {\n    "ANTHROPIC_BASE_URL": "https://api.tokenplus.cn/claude",\n    "ANTHROPIC_AUTH_TOKEN": "sk-••••••"\n  },\n  "model": "claude-sonnet-4-5"\n}',
          models: [
            { id: 'claude-sonnet-4-5', label: 'Claude Sonnet 4.5', desc: '均衡快速，日常编码推荐' },
            { id: 'claude-opus-4-1', label: 'Claude Opus 4.1', desc: '更强推理，适合复杂任务' }
          ]
        },
        'grok': {
          files: [
            { path: '~/.grok/config.json', exists: false },
            { path: '~/.zshrc', exists: true, size: 5312 }
          ],
          changes: [
            '新建 ~/.grok/config.json 写入通用线路',
            '在 ~/.zshrc 末尾追加 GROK_BASE_URL 环境变量'
          ],
          preview: 'export GROK_BASE_URL="https://api.tokenplus.cn/general"\nexport GROK_API_KEY="sk-••••••"'
        },
        'workbuddy': {
          files: [
            { path: '~/Library/Application Support/WorkBuddy/settings.json', exists: true, size: 874 }
          ],
          changes: [
            '把 API 提供商切换为 TokenPlus（Codex 线路）',
            '保留已有的快捷键与主题设置'
          ]
        }
      },
      'tools:connect': null, 'tools:model': null, 'tools:test': null,
      'models:refresh': { models: ['claude-sonnet-4-5', 'claude-opus-4-1', 'gpt-5.2'],
        updated: ['claude-desktop'], errors: {} },
      'settings:update': null, 'pets:use': null, 'shell:open': null,
      'diagnostics:copy': null, 'updates:check': { available: false, version: '0.1.4', message: '已是最新版本 0.1.4' },
      'backups:list': [], 'auth:start': { needCode: false }
    };
    var v = Object.prototype.hasOwnProperty.call(mock, channel) ? mock[channel] : null;
    if (channel === 'pets:use') {
      var id = args[0];
      mock['pets:list'].selectedId = id;
      // real bridge returns fresh JSON each call — clone so Vue sees a new ref
      return Promise.resolve(JSON.parse(JSON.stringify(mock['pets:list'])));
    }
    if (channel === 'tools:plan') {
      return new Promise(function (r) {
        setTimeout(function () { r(JSON.parse(JSON.stringify(mock['tools:plan']))); }, 420);
      });
    }
    if (channel === 'tools:test') {
      var tool = args[0], runId = args[1] || '';
      var model = 'claude-sonnet-4-5';
      var emitT = function (p) {
        (subs['tools:test-chunk'] || []).forEach(function (f) {
          f(Object.assign({ tool: tool, runId: runId }, p));
        });
      };
      var fail = tool === 'grok';
      var parts = ['Hi! ', 'How ', 'can I ', 'help ', 'you ', 'today?'];
      return new Promise(function (resolve, reject) {
        setTimeout(function () { emitT({ state: 'start', model: model }); }, 50);
        if (fail) {
          setTimeout(function () {
            emitT({ state: 'end', ok: false, message: '专用密钥已失效，请重新接入',
              latency: 540, firstToken: null, model: model });
          }, 500);
          setTimeout(function () { reject(new Error('专用密钥已失效，请重新接入')); }, 560);
          return;
        }
        parts.forEach(function (p, i) {
          setTimeout(function () { emitT({ delta: p }); }, 400 + i * 120);
        });
        var endAt = 400 + parts.length * 120 + 80;
        var reply = parts.join('');
        setTimeout(function () {
          emitT({ state: 'end', ok: true, message: '连接正常',
            latency: endAt, firstToken: 400, model: model });
          var t = mtools.find(function (x) { return x.id === tool; });
          if (t) t.lastProbe = { at: Date.now(), ok: true, message: '连接正常',
            latency: endAt, model: model, firstToken: 400, reply: reply };
          (subs['tools:changed'] || []).forEach(function (f) { f(JSON.parse(JSON.stringify(mtools))); });
        }, endAt);
        setTimeout(function () {
          resolve({ ok: true, message: '连接正常', latency: endAt,
            firstToken: 400, model: model, reply: reply });
        }, endAt + 40);
      });
    }
    if (channel === 'tools:connect') {
      var q = args[0] || [];
      // multi-tool run: second tool fails at step 3; solo run (retry) always ok
      var failTool = q.length > 1 ? q[1] : null;
      q.forEach(function (t, i) {
        var lastStep = t === failTool ? 3 : 4;
        for (var j = 0; j <= lastStep; j++) {
          (function (t, i, j, fail) {
            setTimeout(function () {
              (subs['connect:progress'] || []).forEach(function (f) {
                f({ state: 'run', queue: q, index: i, step: j, tool: t,
                  toolState: fail && j === lastStep ? 'fail' : '',
                  message: fail && j === lastStep ? '写入失败：配置文件被占用' : '',
                  failures: [] });
              });
            }, i * 1500 + j * 260);
          })(t, i, j, t === failTool);
        }
        if (t !== failTool) {
          setTimeout(function () {
            var tool = mtools.find(function (x) { return x.id === t; });
            if (tool) { tool.status = 'ok'; tool.model = 'claude-sonnet-4-5'; tool.models = ['claude-sonnet-4-5', 'claude-opus-4-1']; }
            // a real bridge serializes each event — emit a clone so same-ref assigns still trigger
            (subs['tools:changed'] || []).forEach(function (f) { f(JSON.parse(JSON.stringify(mtools))); });
          }, i * 1500 + 1300);
        }
      });
      setTimeout(function () {
        (subs['connect:progress'] || []).forEach(function (f) {
          f({ state: 'done', queue: q, index: q.length - 1, step: 4, tool: q[q.length - 1],
            message: '', failures: failTool ? [{ tool: failTool, message: '写入失败：配置文件被占用' }] : [] });
        });
      }, q.length * 1500 + 400);
      return new Promise(function (r) { setTimeout(r, q.length * 1500 + 500); });
    }
    return Promise.resolve(JSON.parse(JSON.stringify(v)));
  }

  /* ================= mount & boot ================= */
  app.mount('#app');

  function boot() {
    refresh();
    if (!MOCK) {
      invoke('pets:list').then(function (r) { M.petsData = r; });
      invoke('selfcheck:run').then(function (r) { if (r) S.selfCheck = r; }).catch(function () {});
    }
  }
  if (MOCK || TAURI) {
    boot();
  } else {
    // No bridge behind this origin (e.g. static preview) -> fall back to mock data
    fetch('/invoke', { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: '{"channel":"app:bootstrap","args":[]}' })
      .then(function (r) { return r.json(); })
      .then(function (d) {
        if (d && Object.prototype.hasOwnProperty.call(d, 'ok')) boot();
        else { MOCK = true; boot(); }
      })
      .catch(function () { MOCK = true; boot(); });
  }
})();
