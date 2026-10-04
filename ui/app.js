/* Mvpn 前端逻辑（原生 JS，无框架、无打包） */
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let S = null;            // AppState
let boot = {};           // 引导信息
let running = false;
let startedAt = 0;
let nodeDelays = {};     // id -> ms
let selected = new Set();
let logLines = [];
let conns = [];          // 实时连接
let trafficData = null;  // 流量统计
let statTick = 0;
let editNodeId = null;
let editRuleId = null;
let promptOk = null;
let saveTimer = null;

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s == null ? '' : s)
  .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  .replace(/"/g, '&quot;');

function toast(msg, isErr) {
  const t = $('toast');
  t.textContent = msg;
  t.className = 'toast show' + (isErr ? ' err' : '');
  clearTimeout(t._h);
  t._h = setTimeout(() => { t.className = 'toast'; }, 2600);
}

function fmtBytes(n, perSec) {
  if (!n) return perSec ? '0 B/s' : '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  let v = Number(n);
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  const d = i === 0 ? 0 : (v < 10 ? 2 : 1);
  return v.toFixed(d) + ' ' + u[i] + (perSec ? '/s' : '');
}
const fmtRate = (n) => fmtBytes(n, true);
const fmtSize = (n) => fmtBytes(n, false);
const baseName = (p) => String(p || '').split(/[\\/]/).pop();

async function save() {
  if (!S) return;
  try { await invoke('save_state', { newState: S }); }
  catch (e) { toast('保存失败: ' + e, true); }
}
function saveSoon() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(save, 260);
}

/* ---------------------------------------------------------------- 初始化 */
async function init() {
  S = await invoke('get_state');
  boot = await invoke('get_bootstrap');
  running = boot.status === 'running';
  if (running) startedAt = Date.now();

  bindNav();
  bindTopbar();
  bindQuickSwitches();
  bindNodes();
  bindSubs();
  bindRules();
  bindTraffic();
  bindSettings();
  bindModals();
  bindLogs();

  renderAll();
  refreshStatus();

  await listen('traffic', (e) => {
    $('trUp').textContent = '↑ ' + fmtRate(e.payload.up);
    $('trDown').textContent = '↓ ' + fmtRate(e.payload.down);
    // 每 5 个采样顺带刷新一次累计统计，避免每秒都 invoke
    if (++statTick % 5 === 0) refreshTrafficStats();
  });
  await listen('connections', (e) => {
    conns = e.payload || [];
    renderConns();
  });
  await listen('core-log', (e) => {
    logLines.push(e.payload);
    if (logLines.length > 1500) logLines.splice(0, logLines.length - 1500);
    renderLogs();
  });
  await listen('node-delay', (e) => {
    nodeDelays[e.payload.id] = e.payload.delay;
    renderNodes();
  });
  await listen('core-status', () => refreshStatus());

  const lines = await invoke('get_logs');
  logLines = lines || [];
  renderLogs();

  await refreshTrafficStats();
  if (S.setting.autoCheckUpdate) setTimeout(() => checkAppUpdate(false), 4000);
  setInterval(() => {
    refreshStatus(true);
    if (running) renderUptime();
  }, 3000);
  setInterval(() => {
    if ($('trafficAuto') && $('trafficAuto').checked) refreshTrafficStats();
  }, 3000);
}

function renderAll() {
  renderTopbar();
  renderOverview();
  renderNodes();
  renderSubs();
  renderRules();
  renderSettings();
}

/* ---------------------------------------------------------------- 导航 */
function bindNav() {
  document.querySelectorAll('.nav-item').forEach((b) => {
    b.onclick = () => {
      document.querySelectorAll('.nav-item').forEach((x) => x.classList.remove('active'));
      document.querySelectorAll('.page').forEach((x) => x.classList.remove('active'));
      b.classList.add('active');
      $('page-' + b.dataset.page).classList.add('active');
      if (b.dataset.page === 'logs') renderLogs();
      if (b.dataset.page === 'settings') renderSettings();
      if (b.dataset.page === 'traffic') refreshTrafficStats();
    };
  });
}

/* ---------------------------------------------------------------- 顶栏 */
function bindTopbar() {
  document.querySelectorAll('#modeSwitch button').forEach((b) => {
    b.onclick = async () => {
      const mode = b.dataset.mode;
      S.setting.mode = mode;
      await save();
      renderTopbar();
      renderOverview();
      try {
        const r = await invoke('set_mode', { mode });
        if (r && r.restartNeeded && !running) toast('模式已保存，启动后生效');
        else toast('已切换到「' + mode + '」模式');
      } catch (e) { toast('切换失败: ' + e, true); }
    };
  });

  $('btnToggle').onclick = async () => {
    $('btnToggle').disabled = true;
    try {
      if (running) {
        await invoke('stop_core');
        toast('已停止');
      } else {
        await save();
        toast('正在启动…');
        const r = await invoke('start_core');
        toast('已启动 ' + (r.coreVersion || ''));
      }
    } catch (e) {
      toast(String(e), true);
    } finally {
      $('btnToggle').disabled = false;
      setTimeout(refreshStatus, 400);
    }
  };

  $('btnQuickTest').onclick = () => {
    const id = S.activeNode;
    if (!id) { toast('请先选择节点', true); return; }
    testNodes([id]);
  };
}

async function checkAppUpdate(loud) {
  try {
    const r = await invoke('check_app_update');
    const b = $('updBanner');
    if (r.enabled && r.hasUpdate) {
      b.style.display = '';
      $('updText').textContent = `发现新版本 v${r.latest}（当前 v${r.current}）${r.notes ? ' · ' + r.notes : ''}`;
      if (loud) toast(`发现新版本 v${r.latest}`);
    } else {
      b.style.display = 'none';
      if (loud) toast(r.enabled ? `已是最新版本 v${r.current}` : (r.reason || '未配置更新清单'));
    }
  } catch (e) {
    $('updBanner').style.display = 'none';
    if (loud) toast('检查失败: ' + e, true);
  }
}

/* ---------------------------------------------------------------- 流量 / 实时连接 */
async function refreshTrafficStats() {
  try {
    trafficData = await invoke('get_traffic_stats');
    renderTraffic();
  } catch (e) { /* 忽略 */ }
}

function renderConns() {
  const tb = $('connBody');
  tb.innerHTML = conns.slice(0, 60).map((c) => {
    const cls = /direct/i.test(c.chain) ? 'direct' : (/proxy|selector/i.test(c.chain) ? 'proxy' : '');
    const proc = c.process ? ` <span class="conn-sub">${esc(baseName(c.process))}</span>` : '';
    return `<tr>
      <td title="${esc(c.target)}${c.ip && c.ip !== c.target ? ' → ' + esc(c.ip) : ''}">${esc(c.target)}${proc}</td>
      <td>${esc(c.port)}</td>
      <td>${esc(String(c.network || '').toUpperCase())}</td>
      <td><span class="chain-tag ${cls}">${esc(c.chain || '-')}</span></td>
      <td class="num">${fmtSize(c.down)}</td>
      <td class="num">${fmtSize(c.up)}</td>
    </tr>`;
  }).join('');
  $('connEmpty').classList.toggle('show', conns.length === 0);
}

function renderTraffic() {
  const t = trafficData;
  if (!t) return;
  $('ovTodayDown').textContent = fmtSize(t.today.down);
  $('ovTodayUp').textContent = fmtSize(t.today.up);
  $('ovTotalDown').textContent = fmtSize(t.lifetime.down);
  $('ovTotalUp').textContent = fmtSize(t.lifetime.up);

  $('trTodayDown').textContent = fmtSize(t.today.down);
  $('trTodayUp').textContent = fmtSize(t.today.up);
  $('trTodayTotal').textContent = fmtSize(t.today.total);
  $('trLifeDown').textContent = fmtSize(t.lifetime.down);
  $('trLifeUp').textContent = fmtSize(t.lifetime.up);
  $('trLifeTotal').textContent = fmtSize(t.lifetime.total);
  $('trafficUpdated').textContent = '更新于 ' + new Date().toLocaleTimeString();

  // 近 7 天柱状（days 为倒序）
  const days = (t.days || []).slice(0, 7).reverse();
  const max = Math.max(1, ...days.map((d) => d.up + d.down));
  $('trBars').innerHTML = days.length
    ? days.map((d) => {
        const tot = d.up + d.down;
        return `<div class="bar-col" title="${esc(d.date)}">
          <div class="bar-val">${fmtSize(tot)}</div>
          <div class="bar-stack">
            <div class="bar-u" style="height:${(d.up / max * 100).toFixed(1)}%"></div>
            <div class="bar-d" style="height:${(d.down / max * 100).toFixed(1)}%"></div>
          </div>
          <div class="bar-lbl">${esc(String(d.date).slice(5))}</div>
        </div>`;
      }).join('')
    : '<div class="empty show">还没有数据</div>';

  // 分节点
  const nodes = t.nodes || [];
  const tot = nodes.reduce((a, n) => a + n.total, 0) || 1;
  $('trNodeBody').innerHTML = nodes.map((n) => `<tr>
      <td>${esc(n.name)}</td>
      <td class="num">${fmtSize(n.down)}</td>
      <td class="num">${fmtSize(n.up)}</td>
      <td class="num"><b>${fmtSize(n.total)}</b></td>
      <td class="num">${(n.total / tot * 100).toFixed(1)}%</td>
    </tr>`).join('');
  $('trNodeEmpty').classList.toggle('show', nodes.length === 0);
}

function bindTraffic() {
  $('btnRefreshTraffic').onclick = refreshTrafficStats;
}

/* ---------------------------------------------------------------- 概览快捷开关 */
async function applySysProxy(on) {
  try {
    await invoke('toggle_sys_proxy', { on });
    S.setting.sysProxy = on;
    await save();
    toast(on ? `已开启系统代理 127.0.0.1:${S.setting.mixedPort}` : '已关闭系统代理');
  } catch (err) {
    S.setting.sysProxy = !on;
    toast('设置系统代理失败: ' + err, true);
  }
  renderOverview();
  renderSettings();
}

function bindQuickSwitches() {
  $('swSysProxy').onchange = (e) => applySysProxy(e.target.checked);

  $('swTun').onchange = async (e) => {
    const on = e.target.checked;
    if (on && !boot.admin) {
      toast('TUN 模式需要管理员权限，请以管理员身份重新运行 Mvpn', true);
      renderOverview();
      return;
    }
    S.setting.tunEnabled = on;
    await save();
    renderOverview();
    toast(on ? 'TUN 已开启，点右上角「重启」生效' : 'TUN 已关闭，点右上角「重启」生效');
  };

  $('swLan').onchange = async (e) => {
    const on = e.target.checked;
    S.setting.mixedAllowLan = on;
    await save();
    renderOverview();
    toast(on
      ? `局域网共享已开启，点右上角「重启」后其他设备可用「本机IP:${S.setting.mixedPort}」`
      : '已改为仅本机可访问，点右上角「重启」生效');
  };
}

function renderTopbar() {
  document.querySelectorAll('#modeSwitch button').forEach((b) => {
    b.classList.toggle('active', b.dataset.mode === (S.setting.mode || 'rule'));
  });
  $('coreVersion').textContent = boot.coreVersion || 'sing-box 未就绪';
  const sel = $('activeNode');
  const opts = ['<option value="">自动选择（延迟最低）</option>']
    .concat(S.nodes.filter((n) => n.enabled).map((n) =>
      `<option value="${esc(n.id)}"${n.id === S.activeNode ? ' selected' : ''}>${esc(n.name)}</option>`));
  sel.innerHTML = opts.join('');
  sel.onchange = async () => { S.activeNode = sel.value; await save(); renderOverview(); toast('已切换节点'); };
}

async function refreshStatus(quiet) {
  try {
    const st = await invoke('core_status');
    running = st.running;
    if (running && !startedAt) startedAt = Date.now();
    if (!running) startedAt = 0;
    const dot = $('statusDot');
    dot.className = 'dot ' + (running ? 'on' : (st.status === 'starting' ? 'pending' : ''));
    $('statusText').textContent = running ? '运行中' : (st.status === 'starting' ? '启动中' : '已停止');
    $('btnToggle').textContent = running ? '停止' : '启动';
    $('btnToggle').className = 'btn ' + (running ? 'danger' : 'primary');
    $('ovCore').textContent = st.coreVersion || '-';
    $('sbAdmin').textContent = st.admin ? '管理员' : '普通权限';
    $('sbData').textContent = boot.dataDir || '';

    // 系统代理：状态栏显示注册表里的真实状态
    // （配置写"开"但实际没写进去时，这里会红字提示，一眼看出问题）
    const px = $('sbProxy');
    if (st.sysProxy) {
      px.textContent = '系统代理 ' + st.sysProxy;
      px.style.color = 'var(--green)';
    } else {
      px.textContent = st.sysProxyWanted ? '系统代理 期望开启但未生效' : '系统代理 未开启';
      px.style.color = st.sysProxyWanted ? 'var(--red)' : 'var(--txt-3)';
    }
    if (!quiet) renderOverview();
  } catch (e) { /* ignore */ }
}

function renderUptime() {
  if (!startedAt) { $('ovUptime').textContent = '-'; return; }
  const s = Math.floor((Date.now() - startedAt) / 1000);
  const h = String(Math.floor(s / 3600)).padStart(2, '0');
  const m = String(Math.floor((s % 3600) / 60)).padStart(2, '0');
  const ss = String(s % 60).padStart(2, '0');
  $('ovUptime').textContent = `${h}:${m}:${ss}`;
}

/* ---------------------------------------------------------------- 概览 */
function renderOverview() {
  const st = S.setting;
  $('swSysProxy').checked = !!st.sysProxy;
  $('swTun').checked = !!st.tunEnabled;
  $('swLan').checked = !!st.mixedAllowLan;

  const host = st.mixedAllowLan ? '0.0.0.0' : '127.0.0.1';
  $('mixedAddr').textContent = `${host}:${st.mixedPort}`;
  $('ovInbound').textContent = st.mixedEnabled ? `mixed :${st.mixedPort}` : '未启用';

  const lanHint = $('lanHint');
  if (st.mixedAllowLan) {
    lanHint.textContent = `局域网设备可用「本机IP:${st.mixedPort}」作为 HTTP/SOCKS 代理（注意放行防火墙）。`;
  } else {
    lanHint.textContent = '当前仅本机可访问。开启「允许局域网连接」后，同网段设备可共用此代理。';
  }

  const node = S.nodes.find((n) => n.id === S.activeNode);
  $('activeNodeMeta').textContent = node
    ? `${node.protocol.toUpperCase()} · ${node.server}:${node.port}${nodeDelays[node.id] != null ? ' · ' + (nodeDelays[node.id] < 0 ? '超时' : nodeDelays[node.id] + ' ms') : ''}`
    : (S.nodes.length ? '自动选择延迟最低的节点' : '尚未添加节点');

  const modeTxt = { rule: '规则分流：按下方规则表匹配，未命中走当前节点', global: '全局代理：所有流量走当前节点', direct: '直连：不使用代理（仅保留 DNS 劫持）' };
  $('modeHint').textContent = modeTxt[st.mode] || '';
  renderTopbar();
}

/* ---------------------------------------------------------------- 节点 */
function bindNodes() {
  $('btnAddNode').onclick = () => openNodeModal(null);
  $('btnTestAll').onclick = () => {
    if (!S.nodes.length) { toast('没有节点', true); return; }
    testNodes([]);
  };
  $('btnDelNodes').onclick = async () => {
    if (!selected.size) { toast('请先勾选节点', true); return; }
    S.nodes = S.nodes.filter((n) => !selected.has(n.id));
    selected.clear();
    await save(); renderNodes(); renderTopbar(); renderOverview();
    toast('已删除');
  };
  $('btnImportClipboard').onclick = async () => {
    let text = '';
    try { text = await navigator.clipboard.readText(); } catch (e) { text = ''; }
    if (!text) {
      openPrompt('粘贴分享链接或订阅内容', [
        { key: 'text', label: '内容', type: 'textarea', value: '', placeholder: 'vmess://... / vless://... 可多行' },
      ], async (v) => {
        if (v.text) await importLinks(v.text);
      });
      return;
    }
    await importLinks(text);
  };
  $('nodeSearch').oninput = renderNodes;
  $('chkAllNodes').onchange = (e) => {
    selected.clear();
    if (e.target.checked) S.nodes.forEach((n) => selected.add(n.id));
    renderNodes();
  };
}

async function importLinks(text) {
  try {
    const nodes = await invoke('parse_links', { text });
    if (!nodes || !nodes.length) { toast('未解析到有效节点', true); return; }
    nodes.forEach((n, i) => { n.id = 'n' + Date.now() + '-' + i; });
    S.nodes = S.nodes.concat(nodes);
    await save(); renderNodes(); renderTopbar();
    toast(`导入 ${nodes.length} 个节点`);
  } catch (e) { toast('导入失败: ' + e, true); }
}

function delayClass(d) {
  if (d == null) return 'none';
  if (d < 0) return 'bad';
  if (d < 300) return 'ok';
  if (d < 800) return 'mid';
  return 'bad';
}

function renderNodes() {
  const q = ($('nodeSearch').value || '').trim().toLowerCase();
  const list = S.nodes.filter((n) => !q ||
    n.name.toLowerCase().includes(q) || n.server.toLowerCase().includes(q) || n.protocol.includes(q));
  const tb = $('nodeBody');
  tb.innerHTML = list.map((n) => {
    const d = nodeDelays[n.id];
    const dTxt = d == null ? '—' : (d < 0 ? '超时' : d + ' ms');
    const active = n.id === S.activeNode ? ' style="box-shadow: inset 3px 0 0 var(--primary)"' : '';
    return `<tr${active}>
      <td><input type="checkbox" data-sel="${esc(n.id)}"${selected.has(n.id) ? ' checked' : ''}/></td>
      <td><b>${esc(n.name || '(未命名)')}</b>${n.id === S.activeNode ? ' <span class="link">当前</span>' : ''}</td>
      <td><span class="proto-chip ${esc(n.protocol)}">${esc(n.protocol)}</span></td>
      <td>${esc(n.server)}</td>
      <td>${esc(n.port)}</td>
      <td><span class="delay ${delayClass(d)}">${dTxt}</span></td>
      <td><div class="row-acts">
        <span class="link" data-use="${esc(n.id)}">使用</span>
        <span class="link" data-edit="${esc(n.id)}">编辑</span>
        <span class="link danger" data-del="${esc(n.id)}">删除</span>
      </div></td></tr>`;
  }).join('');
  $('nodeEmpty').classList.toggle('show', list.length === 0);

  tb.querySelectorAll('[data-sel]').forEach((c) => c.onchange = () => {
    c.checked ? selected.add(c.dataset.sel) : selected.delete(c.dataset.sel);
  });
  tb.querySelectorAll('[data-use]').forEach((a) => a.onclick = async () => {
    S.activeNode = a.dataset.use; await save(); renderNodes(); renderTopbar(); renderOverview();
    toast('已切换节点');
  });
  tb.querySelectorAll('[data-edit]').forEach((a) => a.onclick = () =>
    openNodeModal(S.nodes.find((n) => n.id === a.dataset.edit)));
  tb.querySelectorAll('[data-del]').forEach((a) => a.onclick = async () => {
    if (!confirm('确定删除该节点？')) return;
    S.nodes = S.nodes.filter((n) => n.id !== a.dataset.del);
    await save(); renderNodes(); renderTopbar();
  });
}

async function testNodes(ids) {
  if (!S.nodes.length) { toast('没有节点', true); return; }
  toast('正在测速…');
  try {
    await invoke('test_nodes', { ids });
    toast('测速完成');
  } catch (e) { toast(String(e), true); }
}

const PROTOCOLS = ['vless', 'vmess', 'trojan', 'shadowsocks', 'hysteria2', 'tuic'];

function openNodeModal(node) {
  editNodeId = node ? node.id : null;
  $('nodeModalTitle').textContent = node ? '编辑节点' : '添加节点';
  const n = node || { protocol: 'vless', port: 443, alterId: 0, security: 'auto', transport: 'tcp', cc: 'bbr' };
  const set = (id, v) => { const e = $(id); if (e.tagName === 'INPUT' && e.type === 'checkbox') e.checked = !!v; else e.value = v == null ? '' : v; };
  set('f_name', n.name); set('f_protocol', n.protocol || 'vless'); set('f_server', n.server); set('f_port', n.port);
  set('f_uuid', n.uuid); set('f_flow', n.flow); set('f_aid', n.alterId); set('f_security', n.security || 'auto');
  set('f_password', n.password); set('f_method', n.method); set('f_obfs', n.obfs); set('f_obfspw', n.obfsPassword);
  set('f_up', n.upMbps); set('f_down', n.downMbps); set('f_cc', n.congestionControl || 'bbr');
  set('f_transport', n.transport || 'tcp'); set('f_wspath', n.wsPath); set('f_wshost', n.wsHost); set('f_grpc', n.grpcService);
  set('f_tls', n.tls); set('f_sni', n.sni); set('f_alpn', n.alpn); set('f_fp', n.fingerprint);
  set('f_insecure', n.allowInsecure); set('f_reality', n.reality); set('f_pbk', n.realityPublicKey); set('f_sid', n.realityShortId);
  syncNodeForm();
  show('nodeModal');
}

function syncNodeForm() {
  const p = $('f_protocol').value;
  const t = $('f_transport').value;
  document.querySelectorAll('#nodeModal [data-p]').forEach((el) => {
    el.style.display = el.dataset.p.split(',').includes(p) ? '' : 'none';
  });
  document.querySelectorAll('#nodeModal [data-t]').forEach((el) => {
    el.style.display = el.dataset.t.split(',').includes(t) ? '' : 'none';
  });
  // hysteria2 / tuic 强制 TLS
  if (p === 'hysteria2' || p === 'tuic') $('f_tls').checked = true;
  const re = $('f_reality');
  re.parentElement.style.display = '';
}

async function saveNode() {
  const get = (id) => { const e = $(id); return e.type === 'checkbox' ? e.checked : e.value.trim(); };
  const server = get('f_server');
  if (!server) { toast('请填写服务器地址', true); return; }
  const port = parseInt(get('f_port') || '0', 10);
  if (!port) { toast('请填写有效端口', true); return; }

  const obj = {
    id: editNodeId || ('n' + Date.now()),
    name: get('f_name') || `${get('f_protocol')}-${server}`,
    protocol: get('f_protocol'), server, port,
    uuid: get('f_uuid'), password: get('f_password'), method: get('f_method'),
    flow: get('f_flow'), alterId: parseInt(get('f_aid') || '0', 10), security: get('f_security'),
    tls: get('f_tls'), sni: get('f_sni'), alpn: get('f_alpn'), fingerprint: get('f_fp'),
    allowInsecure: get('f_insecure'), reality: get('f_reality'),
    realityPublicKey: get('f_pbk'), realityShortId: get('f_sid'),
    transport: get('f_transport'), wsPath: get('f_wspath'), wsHost: get('f_wshost'),
    grpcService: get('f_grpc'), earlyData: false,
    obfs: get('f_obfs'), obfsPassword: get('f_obfspw'),
    upMbps: parseInt(get('f_up') || '0', 10), downMbps: parseInt(get('f_down') || '0', 10),
    congestionControl: get('f_cc'), udpRelayMode: 'native',
    group: '', enabled: true,
  };
  if (editNodeId) {
    const i = S.nodes.findIndex((x) => x.id === editNodeId);
    if (i >= 0) S.nodes[i] = Object.assign({}, S.nodes[i], obj);
  } else {
    S.nodes.push(obj);
  }
  await save();
  close('nodeModal');
  renderNodes(); renderTopbar(); renderOverview();
  toast('已保存');
}

/* ---------------------------------------------------------------- 订阅 */
function bindSubs() {
  $('btnAddSub').onclick = () => {
    openPrompt('添加订阅', [
      { key: 'name', label: '名称', value: '' },
      { key: 'url', label: '订阅地址', value: '', placeholder: 'https://...' },
    ], async (v) => {
      if (!v.url) { toast('请填写订阅地址', true); return; }
      S.subscriptions.push({
        id: 's' + Date.now(), name: v.name || '订阅', url: v.url, nodeIds: [], lastUpdate: '',
      });
      await save(); renderSubs();
      toast('已添加，正在拉取…');
      await updateSub(S.subscriptions[S.subscriptions.length - 1].id);
    });
  };
  $('btnUpdateAllSubs').onclick = async () => {
    for (const s of S.subscriptions) await updateSub(s.id, true);
    toast('全部更新完成');
  };
}

async function updateSub(id, quiet) {
  try {
    const r = await invoke('update_subscription', { id });
    const fresh = await invoke('get_state');
    S.nodes = fresh.nodes; S.subscriptions = fresh.subscriptions;
    await save();
    renderSubs(); renderNodes(); renderTopbar();
    if (!quiet) toast(`订阅更新完成，共 ${r.count} 个节点`);
  } catch (e) { if (!quiet) toast('更新失败: ' + e, true); }
}

function renderSubs() {
  const tb = $('subBody');
  tb.innerHTML = S.subscriptions.map((s) => `
    <tr>
      <td><b>${esc(s.name)}</b></td>
      <td style="max-width:320px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(s.url)}</td>
      <td>${(s.nodeIds || []).length}</td>
      <td>${esc(s.lastUpdate || '—')}</td>
      <td><div class="row-acts">
        <span class="link" data-upd="${esc(s.id)}">更新</span>
        <span class="link danger" data-delsub="${esc(s.id)}">删除</span>
      </div></td></tr>`).join('');
  $('subEmpty').classList.toggle('show', S.subscriptions.length === 0);
  tb.querySelectorAll('[data-upd]').forEach((a) => a.onclick = () => updateSub(a.dataset.upd));
  tb.querySelectorAll('[data-delsub]').forEach((a) => a.onclick = async () => {
    if (!confirm('删除订阅及其节点？')) return;
    const id = a.dataset.delsub;
    S.nodes = S.nodes.filter((n) => n.group !== id);
    S.subscriptions = S.subscriptions.filter((s) => s.id !== id);
    await save(); renderSubs(); renderNodes(); renderTopbar();
  });
}

/* ---------------------------------------------------------------- 分流 */
function bindRules() {
  $('btnAddRule').onclick = () => openRuleModal(null);
  $('cnDirect').onchange = async (e) => { S.setting.cnDirect = e.target.checked; await save(); toast('已保存，重启内核生效'); };
  $('rulesetSource').onchange = async (e) => {
    S.setting.rulesetSource = e.target.value;
    await save();
    // 内置的 geosite-ads 规则集地址跟随切换
    const urls = S.setting.rulesetSource === 'github'
      ? 'https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ads-all.srs'
      : 'https://cdn.jsdelivr.net/gh/SagerNet/sing-geosite@rule-set/geosite-category-ads-all.srs';
    S.ruleSets = (S.ruleSets || []).map(r => r.tag === 'geosite-ads' ? Object.assign({}, r, { url: urls }) : r);
    await save();
    toast('已切换规则集源，重启内核生效');
  };
  $('btnManageRuleSet').onclick = () => {
    openPrompt('规则集（每行：标签|地址）', [
      { key: 'list', label: '规则集', type: 'textarea', value: (S.ruleSets || []).map((r) => `${r.tag}|${r.url}`).join('\n') },
    ], async (v) => {
      const arr = (v.list || '').split('\n').map((l) => l.trim()).filter(Boolean).map((l) => {
        const i = l.indexOf('|');
        const tag = i < 0 ? l : l.slice(0, i);
        const url = i < 0 ? '' : l.slice(i + 1);
        const kind = url ? 'remote' : 'local';
        return { tag: tag.trim(), kind, format: 'binary', url: url.trim(), path: '' };
      }).filter((x) => x.tag);
      S.ruleSets = arr;
      await save(); toast('已保存规则集');
    });
  };
}

function ruleSummary(r) {
  const parts = [];
  const add = (label, v) => { if (v && v.trim()) parts.push(label + ': ' + v.trim().split('\n').map((x) => x.trim()).filter(Boolean).join(', ')); };
  add('规则集', r.ruleSet); add('域名后缀', r.domainSuffix); add('关键字', r.domainKeyword);
  add('域名', r.domain); add('IP', r.ipCidr); add('源IP', r.sourceIpCidr);
  add('进程', r.processName); add('端口', r.port); add('协议', r.protocol); add('入站', r.inbound);
  if (r.clashMode) parts.push('模式: ' + r.clashMode);
  const s = parts.join(' · ') || '(无匹配条件)';
  return r.invert ? '非 (' + s + ')' : s;
}

function renderRules() {
  const tb = $('ruleBody');
  const rules = S.rules || [];
  tb.innerHTML = rules.map((r, i) => `
    <tr>
      <td><input type="checkbox" data-rx="${i}"${r.enabled ? ' checked' : ''}/></td>
      <td>${esc(r.remark || '(未命名)')}</td>
      <td class="hint" style="color:var(--txt-2);font-size:12px">${esc(ruleSummary(r))}</td>
      <td>${r.action === 'reject'
        ? '<span class="proto-chip" style="background:#fdeaf3;color:#be185d">拦截</span>'
        : `<span class="proto-chip" style="background:#e8f0ff;color:#2f6fed">${esc(r.outbound)}</span>`}</td>
      <td><div class="row-acts">
        <span class="link" data-rup="${i}">↑</span>
        <span class="link" data-rdown="${i}">↓</span>
        <span class="link" data-redit="${i}">编辑</span>
        <span class="link danger" data-rdel="${i}">删除</span>
      </div></td></tr>`).join('');
  $('ruleEmpty').classList.toggle('show', rules.length === 0);
  $('cnDirect').checked = !!S.setting.cnDirect;
  $('rulesetSource').value = S.setting.rulesetSource || 'jsdelivr';

  tb.querySelectorAll('[data-rx]').forEach((c) => c.onchange = async () => {
    S.rules[+c.dataset.rx].enabled = c.checked; await save();
  });
  const move = async (i, d) => {
    const j = i + d;
    if (j < 0 || j >= S.rules.length) return;
    const t = S.rules[i]; S.rules[i] = S.rules[j]; S.rules[j] = t;
    await save(); renderRules();
  };
  tb.querySelectorAll('[data-rup]').forEach((a) => a.onclick = () => move(+a.dataset.rup, -1));
  tb.querySelectorAll('[data-rdown]').forEach((a) => a.onclick = () => move(+a.dataset.rdown, 1));
  tb.querySelectorAll('[data-redit]').forEach((a) => a.onclick = () => openRuleModal(S.rules[+a.dataset.redit]));
  tb.querySelectorAll('[data-rdel]').forEach((a) => a.onclick = async () => {
    S.rules.splice(+a.dataset.rdel, 1); await save(); renderRules();
  });
}

function openRuleModal(rule) {
  editRuleId = rule ? rule.id : null;
  $('ruleModalTitle').textContent = rule ? '编辑规则' : '添加规则';
  const r = rule || { action: 'route', outbound: 'proxy' };
  $('r_remark').value = r.remark || '';
  $('r_action').value = r.action || 'route';
  $('r_outbound').value = r.outbound || 'proxy';
  $('r_ruleset').value = r.ruleSet || '';
  $('r_dsuffix').value = r.domainSuffix || '';
  $('r_dkeyword').value = r.domainKeyword || '';
  $('r_domain').value = r.domain || '';
  $('r_ipcidr').value = r.ipCidr || '';
  $('r_process').value = r.processName || '';
  $('r_port').value = r.port || '';
  $('r_protocol').value = r.protocol || '';
  $('r_invert').checked = !!r.invert;
  syncRuleForm();
  show('ruleModal');
}

function syncRuleForm() {
  $('r_outWrap').style.display = $('r_action').value === 'route' ? '' : 'none';
}

async function saveRule() {
  const g = (id) => $(id).value.trim();
  const obj = {
    id: editRuleId || ('r' + Date.now()),
    enabled: true,
    remark: g('r_remark'),
    action: g('r_action'),
    outbound: g('r_action') === 'route' ? g('r_outbound') : '',
    ruleSet: g('r_ruleset'), domainSuffix: g('r_dsuffix'), domainKeyword: g('r_dkeyword'),
    domain: g('r_domain'), ipCidr: g('r_ipcidr'), sourceIpCidr: '', processName: g('r_process'),
    port: g('r_port'), protocol: g('r_protocol'), inbound: '', clashMode: '',
    invert: $('r_invert').checked,
  };
  if (editRuleId) {
    const i = S.rules.findIndex((x) => x.id === editRuleId);
    if (i >= 0) S.rules[i] = Object.assign({}, S.rules[i], obj, { enabled: S.rules[i].enabled });
  } else {
    S.rules.push(obj);
  }
  await save(); close('ruleModal'); renderRules();
  toast('已保存，重启内核生效');
}

/* ---------------------------------------------------------------- 设置 */
function bindSettings() {
  const b = (id, fn) => { const e = $(id); if (!e) return; e.addEventListener(e.type === 'text' || e.type === 'number' || e.tagName === 'TEXTAREA' ? 'input' : 'change', fn); };
  const st = () => S.setting;

  b('setSysProxy', () => applySysProxy($('setSysProxy').checked));
  b('setMixedEnabled', () => { st().mixedEnabled = $('setMixedEnabled').checked; saveSoon(); renderOverview(); });
  b('setMixedPort', () => { st().mixedPort = parseInt($('setMixedPort').value) || 2080; saveSoon(); renderOverview(); });
  b('setLan', () => { st().mixedAllowLan = $('setLan').checked; saveSoon(); renderOverview(); });
  b('setMixedAuth', () => { st().mixedAuth = $('setMixedAuth').checked; saveSoon(); });
  b('setMixedUser', () => { st().mixedUsername = $('setMixedUser').value.trim(); saveSoon(); });
  b('setMixedPass', () => { st().mixedPassword = $('setMixedPass').value.trim(); saveSoon(); });

  b('setTun', () => { st().tunEnabled = $('setTun').checked; saveSoon(); renderOverview(); });
  b('setTunStack', () => { st().tunStack = $('setTunStack').value; saveSoon(); });
  b('setTunMtu', () => { st().tunMtu = parseInt($('setTunMtu').value) || 9000; saveSoon(); });
  b('setTunAuto', () => { st().tunAutoRoute = $('setTunAuto').checked; saveSoon(); });
  b('setTunStrict', () => { st().tunStrictRoute = $('setTunStrict').checked; saveSoon(); });

  b('setDnsLocal', () => { st().dnsLocal = $('setDnsLocal').value.trim(); saveSoon(); });
  b('setDnsRemote', () => { st().dnsRemote = $('setDnsRemote').value.trim(); saveSoon(); });
  b('setDnsStrategy', () => { st().dnsStrategy = $('setDnsStrategy').value; saveSoon(); });
  b('setDnsCn', () => { st().dnsCnDirect = $('setDnsCn').checked; saveSoon(); });

  b('setCorePath', () => { st().corePath = $('setCorePath').value.trim(); saveSoon(); });
  b('setClashPort', () => { st().clashApiPort = parseInt($('setClashPort').value) || 9900; saveSoon(); });

  b('setAutostart', async () => {
    st().autostart = $('setAutostart').checked; await save();
    try { await invoke('set_autostart', { on: st().autostart }); toast('已' + (st().autostart ? '开启' : '关闭') + '开机自启'); }
    catch (e) { toast('设置自启失败: ' + e, true); }
  });
  b('setStartMin', () => { st().startMinimized = $('setStartMin').checked; saveSoon(); });
  b('setAutoConn', () => { st().autoConnect = $('setAutoConn').checked; saveSoon(); });
  b('setLogLevel', () => { st().logLevel = $('setLogLevel').value; saveSoon(); });
  b('setBypass', () => { st().sysProxyBypass = $('setBypass').value.trim(); saveSoon(); });

  $('btnCheckUpdate').onclick = async () => {
    toast('正在检查…');
    try {
      const r = await invoke('check_core_update');
      toast(r.hasUpdate ? `发现新版本 ${r.latest}（当前 ${r.current}）` : `已是最新版本 ${r.latest}`);
    } catch (e) { toast('检查失败: ' + e, true); }
  };
  $('btnDownloadCore').onclick = async () => {
    toast('开始下载内核…');
    $('btnDownloadCore').disabled = true;
    try {
      const r = await invoke('download_core');
      boot.coreVersion = r.version;
      toast('内核已更新：' + r.version);
      renderSettings(); refreshStatus();
    } catch (e) { toast('下载失败: ' + e, true); }
    finally { $('btnDownloadCore').disabled = false; }
  };
  b('setUpdUrl', () => { st().updateManifestUrl = $('setUpdUrl').value.trim(); saveSoon(); });
  b('setAutoUpd', () => { st().autoCheckUpdate = $('setAutoUpd').checked; saveSoon(); });
  $('btnCheckAppUpd').onclick = () => checkAppUpdate(true);
  $('btnDoUpdate').onclick = async () => {
    if (!confirm('将下载并替换当前程序（会自动重启），确定继续？')) return;
    toast('正在更新…');
    try {
      const r = await invoke('update_app');
      if (r.updated) toast(`已更新到 ${r.version}，程序将重启`);
      else toast(r.reason || '无需更新');
    } catch (e) { toast('更新失败: ' + e, true); }
  };

  $('btnPreviewConfig').onclick = async () => {
    try {
      await save();
      $('configBox').textContent = await invoke('generate_config');
      show('configModal');
    } catch (e) { toast('生成失败: ' + e, true); }
  };
  $('btnOpenData').onclick = () => invoke('open_path', { path: boot.dataDir });
  $('btnOpenLog').onclick = () => invoke('open_path', { path: boot.dataDir + '\\logs\\mvpn.log' });
  $('btnRestart').onclick = async () => {
    try { await invoke('restart_core'); toast('已重启'); setTimeout(refreshStatus, 500); }
    catch (e) { toast(String(e), true); }
  };
  $('btnReset').onclick = async () => {
    if (!confirm('恢复默认设置？节点与规则将保留。')) return;
    const fresh = await invoke('get_state');
    void fresh;
    toast('请在外部删除 state.json 后重启以完全重置');
  };
}

function renderSettings() {
  const st = S.setting;
  const set = (id, v) => { const e = $(id); if (!e) return; if (e.type === 'checkbox') e.checked = !!v; else e.value = v == null ? '' : v; };
  set('setSysProxy', st.sysProxy); set('setMixedEnabled', st.mixedEnabled); set('setMixedPort', st.mixedPort); set('setLan', st.mixedAllowLan);
  set('setMixedAuth', st.mixedAuth); set('setMixedUser', st.mixedUsername); set('setMixedPass', st.mixedPassword);
  set('setTun', st.tunEnabled); set('setTunStack', st.tunStack || 'mixed'); set('setTunMtu', st.tunMtu);
  set('setTunAuto', st.tunAutoRoute); set('setTunStrict', st.tunStrictRoute);
  set('setDnsLocal', st.dnsLocal); set('setDnsRemote', st.dnsRemote); set('setDnsStrategy', st.dnsStrategy);
  set('setDnsCn', st.dnsCnDirect);
  set('setCorePath', st.corePath); set('setClashPort', st.clashApiPort);
  set('setAutostart', st.autostart); set('setStartMin', st.startMinimized); set('setAutoConn', st.autoConnect);
  set('setLogLevel', st.logLevel || 'info'); set('setBypass', st.sysProxyBypass);
  set('setUpdUrl', st.updateManifestUrl); set('setAutoUpd', st.autoCheckUpdate);
  $('setAppVer').textContent = boot.appVersion || '-';
  $('setCoreVer').textContent = boot.coreVersion || '未找到内核';
  $('setClashAddr').textContent = '127.0.0.1:' + st.clashApiPort;
  $('tunHint').textContent = boot.admin
    ? 'TUN 已具备管理员权限，可直接启用。'
    : '⚠ 当前非管理员运行，启用 TUN 需要以管理员身份启动 Mvpn。';
}

/* ---------------------------------------------------------------- 日志 */
function bindLogs() {
  $('btnClearLog').onclick = async () => { logLines = []; await invoke('clear_logs'); renderLogs(); };
}
function renderLogs() {
  const box = $('logBox');
  box.textContent = logLines.join('\n');
  $('logCount').textContent = logLines.length + ' 行';
  if ($('logAutoScroll').checked) box.scrollTop = box.scrollHeight;
}

/* ---------------------------------------------------------------- 弹窗 */
function show(id) { $(id).classList.add('show'); }
function close(id) { $(id).classList.remove('show'); }
function bindModals() {
  document.querySelectorAll('[data-close]').forEach((b) => b.onclick = () => close(b.dataset.close));
  document.querySelectorAll('.modal-mask').forEach((m) => m.addEventListener('mousedown', (e) => {
    if (e.target === m) m.classList.remove('show');
  }));
  $('f_protocol').onchange = syncNodeForm;
  $('f_transport').onchange = syncNodeForm;
  $('r_action').onchange = syncRuleForm;
  $('btnSaveNode').onclick = saveNode;
  $('btnSaveRule').onclick = saveRule;
  $('btnPromptOk').onclick = async () => {
    const vals = {};
    document.querySelectorAll('#promptBody [data-key]').forEach((e) => { vals[e.dataset.key] = e.value; });
    close('promptModal');
    if (promptOk) await promptOk(vals);
  };
}

function openPrompt(title, fields, onOk) {
  $('promptTitle').textContent = title;
  $('promptBody').innerHTML = fields.map((f) => {
    const inner = f.type === 'textarea'
      ? `<textarea data-key="${esc(f.key)}" rows="7" placeholder="${esc(f.placeholder || '')}" style="width:100%">${esc(f.value || '')}</textarea>`
      : `<input data-key="${esc(f.key)}" type="text" value="${esc(f.value || '')}" placeholder="${esc(f.placeholder || '')}" style="width:100%"/>`;
    return `<label style="display:block;margin-bottom:10px;font-size:12.5px;color:var(--txt-2)">
      ${esc(f.label)}<div style="margin-top:5px">${inner}</div></label>`;
  }).join('');
  promptOk = onOk;
  show('promptModal');
}

window.addEventListener('DOMContentLoaded', init);
