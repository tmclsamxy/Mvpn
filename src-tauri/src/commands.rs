//! Tauri 命令：状态、内核生命周期、系统代理、更新、测速、订阅
use crate::core;
use crate::model::*;
use crate::singbox::{self, TAG_DIRECT, TAG_PROXY};
use crate::stats;
use crate::store::{self, Paths};
use crate::sub;
use crate::sysproxy;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};
use winreg::enums::*;
use winreg::RegKey;

pub struct Shared {
    pub paths: Paths,
    pub state: AppState,
    pub child: Option<Child>,
    pub status: String,
    pub logs: VecDeque<String>,
    pub core: Option<PathBuf>,
    pub core_version: String,
    pub admin: bool,
    pub monitor_stop: Arc<AtomicBool>,
    pub stats: stats::TrafficStats,
}

pub type SharedState = Arc<Mutex<Shared>>;

// ---------------------------------------------------------------- 对外包装（托盘复用）

pub fn do_start_pub(app: AppHandle, shared: SharedState) -> Result<Value, String> {
    do_start(app, shared)
}

pub fn do_stop_pub(shared: &SharedState, keep_proxy: bool) -> Value {
    do_stop(shared, keep_proxy);
    json!({ "status": "stopped" })
}

// ---------------------------------------------------------------- 基础状态

#[tauri::command]
pub fn get_bootstrap(state: State<'_, SharedState>) -> Value {
    let s = state.lock().unwrap();
    json!({
        "status": s.status,
        "admin": s.admin,
        "corePath": s.core.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "coreVersion": s.core_version,
        "sysProxy": sysproxy::current(),
        "dataDir": s.paths.data.to_string_lossy(),
        "appVersion": env!("CARGO_PKG_VERSION"),
    })
}

#[tauri::command]
pub fn get_state(state: State<'_, SharedState>) -> AppState {
    state.lock().unwrap().state.clone()
}

#[tauri::command]
pub fn save_state(state: State<'_, SharedState>, new_state: AppState) -> Result<(), String> {
    let mut s = state.lock().unwrap();
    s.state = new_state;
    let p = s.paths.clone();
    let st = s.state.clone();
    store::save_state(&p, &st).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn generate_config(state: State<'_, SharedState>) -> Result<String, String> {
    let s = state.lock().unwrap();
    let cfg = singbox::build_config(&s.state, &s.paths.runtime);
    serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_logs(state: State<'_, SharedState>) -> Vec<String> {
    state.lock().unwrap().logs.iter().cloned().collect()
}

#[tauri::command]
pub fn clear_logs(state: State<'_, SharedState>) {
    let mut s = state.lock().unwrap();
    s.logs.clear();
}

#[tauri::command]
pub fn is_admin() -> bool {
    core::is_admin()
}

// ---------------------------------------------------------------- 内核生命周期

fn push_line(app: &AppHandle, shared: &SharedState, raw: &str) {
    let tagged = format!("{}  {}", store::now_ts(), raw);
    let mut s = shared.lock().unwrap();
    let p = s.paths.clone();
    store::append_log(&p, &tagged);
    store::push_log(&mut s.logs, tagged, Some(app));
}

fn pipe_logs<R: std::io::Read + Send + 'static>(app: AppHandle, shared: SharedState, r: R) {
    std::thread::spawn(move || {
        let reader = BufReader::new(r);
        for line in reader.lines().map_while(|l| l.ok()) {
            if line.trim().is_empty() {
                continue;
            }
            push_line(&app, &shared, &line);
        }
    });
}

fn stop_monitor(shared: &SharedState) {
    let stop = shared.lock().unwrap().monitor_stop.clone();
    stop.store(true, Ordering::Relaxed);
    // 退出前把统计落盘，避免丢数据
    let s = shared.lock().unwrap();
    let _ = stats::save(&s.paths.stats_file(), &s.stats);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 一次连接轮询的结果
pub struct ConnPoll {
    /// 本轮新增流量（相对上一次采样）
    pub delta: stats::Bytes,
    /// 按节点归因的本轮新增流量
    pub per_node: HashMap<String, stats::Bytes>,
    /// 供 UI 展示的连接列表
    pub rows: Vec<Value>,
}

/// 拉取 Clash API `/connections` 并做差分。
/// 抽成独立函数以便 `--monitortest` 用同一条代码路径做验证。
pub fn poll_connections(
    client: &reqwest::blocking::Client,
    port: u16,
    prev_totals: &mut stats::Bytes,
    prev_conn: &mut HashMap<String, (String, u64, u64)>,
    tag2id: &HashMap<String, String>,
) -> Result<ConnPoll, String> {
    let v: Value = client
        .get(format!("http://127.0.0.1:{}/connections", port))
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    let totals = stats::Bytes {
        up: v["uploadTotal"].as_u64().unwrap_or(0),
        down: v["downloadTotal"].as_u64().unwrap_or(0),
    };
    // 内核重启后累计值会归零，此时按「从 0 重新累计」处理
    let delta = if totals.up >= prev_totals.up && totals.down >= prev_totals.down {
        stats::Bytes {
            up: totals.up - prev_totals.up,
            down: totals.down - prev_totals.down,
        }
    } else {
        totals
    };
    *prev_totals = totals;

    let empty: Vec<Value> = Vec::new();
    let list = v["connections"].as_array().unwrap_or(&empty);
    let mut cur: HashMap<String, (String, u64, u64)> = HashMap::new();
    let mut per_node: HashMap<String, stats::Bytes> = HashMap::new();
    let mut rows: Vec<(u64, Value)> = Vec::new();

    for c in list.iter() {
        let id = c["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let m = &c["metadata"];
        let chains: Vec<String> = c["chains"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let last_tag = chains.last().cloned().unwrap_or_default();
        let node_id = tag2id.get(&last_tag).cloned().unwrap_or_default();
        let up = c["upload"].as_u64().unwrap_or(0);
        let down = c["download"].as_u64().unwrap_or(0);

        // 按连接做差分再归因到节点：连接关闭时不会丢量（关闭前已累计完）
        if let Some((_, pu, pd)) = prev_conn.get(&id) {
            let du = up.saturating_sub(*pu);
            let dd = down.saturating_sub(*pd);
            if du > 0 || dd > 0 {
                per_node
                    .entry(node_id.clone())
                    .or_default()
                    .add(&stats::Bytes { up: du, down: dd });
            }
        }
        cur.insert(id.clone(), (node_id.clone(), up, down));

        let host = m["host"].as_str().unwrap_or("").to_string();
        let dip = m["destinationIP"].as_str().unwrap_or("").to_string();
        let target = if !host.is_empty() {
            host
        } else if !dip.is_empty() {
            dip.clone()
        } else {
            "-".to_string()
        };
        rows.push((
            up + down,
            json!({
                "id": id,
                "target": target,
                "ip": dip,
                "port": m["destinationPort"].as_str().unwrap_or(""),
                "network": m["network"].as_str().unwrap_or(""),
                "type": m["type"].as_str().unwrap_or(""),
                "process": m["processPath"].as_str().unwrap_or(""),
                "chain": chains.join(" → "),
                "nodeId": node_id,
                "up": up,
                "down": down,
                "start": c["start"].as_str().unwrap_or(""),
                "rule": c["rule"].as_str().unwrap_or(""),
            }),
        ));
    }
    *prev_conn = cur;

    rows.sort_by(|a, b| b.0.cmp(&a.0));
    rows.truncate(60);
    Ok(ConnPoll {
        delta,
        per_node,
        rows: rows.into_iter().map(|(_, v)| v).collect(),
    })
}

/// 监控线程：轮询 Clash API `/connections`
/// 一份数据同时供给三处：① 右上角实时速率 ② 分节点/今日/历史用量累计 ③ 正在访问的连接列表
fn start_monitor(app: AppHandle, shared: SharedState) {
    let (stop, port, tag2id) = {
        let s = shared.lock().unwrap();
        let st = s.state.clone();
        let nodes: Vec<crate::model::Node> = st
            .nodes
            .iter()
            .filter(|n| n.enabled && !n.server.is_empty() && n.port > 0)
            .cloned()
            .collect();
        let m: HashMap<String, String> = singbox::node_tags(&nodes).into_iter().collect();
        (s.monitor_stop.clone(), st.setting.clash_api_port, m)
    };
    stop.store(false, Ordering::Relaxed);

    std::thread::spawn(move || {
        let client = match reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut prev_totals = stats::Bytes::default();
        let mut prev_conn: HashMap<String, (String, u64, u64)> = HashMap::new();
        let mut ticks: u64 = 0;
        let mut reported = false;

        while !stop.load(Ordering::Relaxed) {
            match poll_connections(&client, port, &mut prev_totals, &mut prev_conn, &tag2id) {
                Ok(p) => {
                    if !reported {
                        reported = true;
                        push_line(
                            &app,
                            &shared,
                            &format!("INFO  流量监控已连接 Clash API 127.0.0.1:{port}"),
                        );
                    }
                    let today = store::today();
                    {
                        let mut s = shared.lock().unwrap();
                        s.stats.rollover_series(&today);
                        s.stats.accumulate(&today, p.delta, &p.per_node);
                        s.stats.push_sample(now_secs(), p.delta.up, p.delta.down);
                    }
                    let _ = app.emit("traffic", json!({ "up": p.delta.up, "down": p.delta.down }));
                    let _ = app.emit("connections", json!(p.rows));

                    ticks += 1;
                    if ticks % 10 == 0 {
                        let s = shared.lock().unwrap();
                        let f = s.paths.stats_file();
                        let _ = stats::save(&f, &s.stats);
                    }
                }
                Err(_) => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }
    });
}

/// 供其它模块在 tokio 上下文外安全调用 reqwest::blocking 的包装：
/// reqwest::blocking 内部使用 tokio Runtime::block_on，若调用线程已处于
/// tauri::async_runtime::spawn_blocking 这类运行时上下文会直接 panic。
pub fn blocking_http<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv().map_err(|e| e.to_string())?
}

fn do_stop(shared: &SharedState, keep_proxy: bool) {
    stop_monitor(shared);
    let (mut child, had_proxy) = {
        let mut s = shared.lock().unwrap();
        let had = s.state.setting.sys_proxy;
        (s.child.take(), had)
    };
    if let Some(mut c) = child.take() {
        core::kill_child(&mut c);
    }
    if had_proxy && !keep_proxy {
        let _ = sysproxy::disable();
    }
    let mut s = shared.lock().unwrap();
    s.status = "stopped".into();
}

#[tauri::command]
pub async fn start_core(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Value, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || do_start(app, shared))
        .await
        .map_err(|e| e.to_string())?
}

fn do_start(app: AppHandle, shared: SharedState) -> Result<Value, String> {
    let (setting, app_state) = {
        let s = shared.lock().unwrap();
        (s.state.setting.clone(), s.state.clone())
    };

    let paths = shared.lock().unwrap().paths.clone();
    let core_path = {
        let s = shared.lock().unwrap();
        core::find_core(&s.paths, &setting)
    }
    .ok_or_else(|| "未找到 sing-box 内核，请先在「设置 → 内核」中下载或指定路径".to_string())?;

    let admin = core::is_admin();
    if setting.tun_enabled && !admin {
        return Err("TUN 模式需要管理员权限，请右键以管理员身份运行 Mvpn".into());
    }
    if setting.tun_enabled {
        let core_dir = core_path.parent().unwrap_or(&paths.core).to_path_buf();
        let proxy = core::update_proxy_url(&setting, false);
        if let Err(e) = core::ensure_wintun(&core_dir, proxy.as_deref()) {
            push_line(&app, &shared, &format!("WARN  {e}"));
        }
    }
    if !setting.mixed_enabled && !setting.tun_enabled {
        return Err("至少需要启用一种入站（本地/局域网代理 或 TUN）".into());
    }

    // 先停旧进程
    do_stop(&shared, true);

    // 生成 & 校验配置
    let cfg = {
        let s = shared.lock().unwrap();
        serde_json::to_string_pretty(&singbox::build_config(&app_state, &s.paths.runtime))
            .map_err(|e| e.to_string())?
    };
    let config_file = paths.config_file();
    std::fs::write(&config_file, &cfg).map_err(|e| format!("写入配置失败: {e}"))?;

    if let Err(e) = core::check_config(&core_path, &config_file) {
        push_line(&app, &shared, &format!("ERROR 配置校验失败: {e}"));
        return Err(format!("配置校验失败: {e}"));
    }

    {
        let mut s = shared.lock().unwrap();
        s.status = "starting".into();
        s.core = Some(core_path.clone());
    }

    let mut child = core::spawn_core(&core_path, &config_file, &paths.runtime)
        .map_err(|e| format!("启动内核失败: {e}"))?;

    if let Some(out) = child.stdout.take() {
        pipe_logs(app.clone(), shared.clone(), out);
    }
    if let Some(er) = child.stderr.take() {
        pipe_logs(app.clone(), shared.clone(), er);
    }

    {
        let mut s = shared.lock().unwrap();
        s.child = Some(child);
        s.status = "running".into();
    }
    push_line(&app, &shared, "INFO  内核已启动");

    // 系统代理（TUN 模式下无需，系统已由 TUN 全局接管）
    if setting.sys_proxy && !setting.tun_enabled && setting.mixed_enabled {
        if let Err(e) = set_sys_proxy_internal(&app, &shared, true) {
            push_line(&app, &shared, &format!("WARN  {e}"));
        }
    }

    // 代理模式（clash_api 运行时切换）
    if setting.mode != "rule" {
        let _ = set_clash_mode(setting.clash_api_port, &setting.mode);
    }

    start_monitor(app.clone(), shared.clone());

    let s = shared.lock().unwrap();
    Ok(json!({ "status": s.status, "coreVersion": s.core_version }))
}

#[tauri::command]
pub fn stop_core(state: State<'_, SharedState>) -> Value {
    let shared = state.inner().clone();
    do_stop(&shared, false);
    json!({ "status": "stopped" })
}

#[tauri::command]
pub async fn restart_core(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Value, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        do_stop(&shared, true);
        std::thread::sleep(std::time::Duration::from_millis(300));
        do_start(app, shared)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn core_status(state: State<'_, SharedState>) -> Value {
    let s = state.lock().unwrap();
    let running = s.child.is_some();
    json!({
        "status": s.status,
        "running": running,
        "coreVersion": s.core_version,
        "clashApi": format!("127.0.0.1:{}", s.state.setting.clash_api_port),
        "sysProxy": sysproxy::current(),
        "sysProxyWanted": s.state.setting.sys_proxy,
        "admin": core::is_admin(),
    })
}

/// 流量统计：今日 / 历史 / 分节点累计
#[tauri::command]
pub fn get_traffic_stats(state: State<'_, SharedState>) -> Value {
    let s = state.lock().unwrap();
    let today = store::today();
    let t = s.stats.today(&today);
    let life = s.stats.lifetime();

    let mut ids: Vec<&String> = s.stats.nodes.keys().collect();
    ids.sort();
    let nodes: Vec<Value> = ids
        .iter()
        .map(|id| {
            let b = s.stats.nodes.get(*id).copied().unwrap_or_default();
            let name = s
                .state
                .nodes
                .iter()
                .find(|n| &n.id == *id)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| (*id).clone());
            json!({ "id": id, "name": name, "up": b.up, "down": b.down, "total": b.total() })
        })
        .collect();

    let days: Vec<Value> = s
        .stats
        .days
        .iter()
        .rev()
        .take(30)
        .map(|d| json!({ "date": d.date, "up": d.up, "down": d.down }))
        .collect();

    json!({
        "today": { "up": t.up, "down": t.down, "total": t.total() },
        "lifetime": { "up": life.up, "down": life.down, "total": life.total() },
        "days": days,
        "series": s.stats.series,
        "nodes": nodes,
    })
}

// ---------------------------------------------------------------- 系统代理

/// 设置/关闭系统代理，并同步持久化设置（保证 UI 与实际状态一致）
pub fn set_sys_proxy_internal(
    app: &AppHandle,
    shared: &SharedState,
    on: bool,
) -> Result<Value, String> {
    let (port, bypass) = {
        let s = shared.lock().unwrap();
        (s.state.setting.mixed_port, s.state.setting.sys_proxy_bypass.clone())
    };
    if on {
        sysproxy::enable(&format!("127.0.0.1:{}", port), &bypass)
            .map_err(|e| format!("开启系统代理失败: {e}"))?;
        push_line(app, shared, &format!("INFO  已开启系统代理 127.0.0.1:{port}"));
    } else {
        sysproxy::disable().map_err(|e| format!("关闭系统代理失败: {e}"))?;
        push_line(app, shared, "INFO  已关闭系统代理");
    }
    {
        let mut s = shared.lock().unwrap();
        s.state.setting.sys_proxy = on;
        let p = s.paths.clone();
        let st = s.state.clone();
        let _ = store::save_state(&p, &st);
    }
    Ok(json!({ "sysProxy": on, "server": format!("127.0.0.1:{}", port) }))
}

#[tauri::command]
pub fn toggle_sys_proxy(
    app: AppHandle,
    state: State<'_, SharedState>,
    on: bool,
) -> Result<Value, String> {
    let shared = state.inner().clone();
    set_sys_proxy_internal(&app, &shared, on)
}

/// 启动时清理「上次退出残留」的系统代理：
/// 若注册表里的代理指向本机但内核并未运行，那个代理是死的，会让用户上不了网。
pub fn cleanup_stale_sys_proxy(setting: &Setting) {
    if let Some(server) = sysproxy::current() {
        let want = format!("127.0.0.1:{}", setting.mixed_port);
        if server.contains(&want) {
            let _ = sysproxy::disable();
            eprintln!("[mvpn] 检测到残留系统代理 {server}，已自动关闭");
        }
    }
}

fn set_clash_mode(port: u16, mode: &str) -> Result<(), String> {
    let m = mode.to_string();
    blocking_http(move || {
        let c = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;
        c.patch(format!("http://127.0.0.1:{}/configs", port))
            .json(&json!({ "mode": m }))
            .send()
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn set_mode(state: State<'_, SharedState>, mode: String) -> Result<Value, String> {
    let (port, running) = {
        let mut s = state.lock().unwrap();
        s.state.setting.mode = mode.clone();
        let p = s.paths.clone();
        let st = s.state.clone();
        let _ = store::save_state(&p, &st);
        (st.setting.clash_api_port, s.child.is_some())
    };
    if running {
        if let Err(e) = set_clash_mode(port, &mode) {
            return Ok(json!({ "mode": mode, "restartNeeded": true, "error": e }));
        }
    }
    Ok(json!({ "mode": mode, "restartNeeded": !running }))
}

// ---------------------------------------------------------------- 内核更新

#[tauri::command]
pub fn check_core_update(state: State<'_, SharedState>) -> Result<Value, String> {
    let s = state.lock().unwrap();
    let proxy = core::update_proxy_url(&s.state.setting, s.child.is_some());
    let current = s.core_version.clone();
    drop(s);
    let (latest, url) = core::latest_release(proxy.as_deref()).map_err(|e| e.to_string())?;
    let cur_tag = current
        .split_whitespace()
        .last()
        .unwrap_or("")
        .to_string();
    Ok(json!({
        "current": cur_tag,
        "latest": latest,
        "hasUpdate": !cur_tag.is_empty() && cur_tag != latest,
        "url": url
    }))
}

#[tauri::command]
pub async fn download_core(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Value, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || do_download_core(app, shared))
        .await
        .map_err(|e| e.to_string())?
}

fn do_download_core(app: AppHandle, shared: SharedState) -> Result<Value, String> {
    let (proxy, core_dir) = {
        let s = shared.lock().unwrap();
        (
            core::update_proxy_url(&s.state.setting, s.child.is_some()),
            s.paths.core.clone(),
        )
    };
    push_line(&app, &shared, "INFO  正在查询 sing-box 最新版本…");
    let (ver, url) = core::latest_release(proxy.as_deref()).map_err(|e| e.to_string())?;
    push_line(&app, &shared, &format!("INFO  下载内核 v{ver} …"));

    let was_running = shared.lock().unwrap().child.is_some();
    if was_running {
        do_stop(&shared, true);
    }

    let bytes = core::download_bytes(&url, proxy.as_deref()).map_err(|e| e.to_string())?;
    let exe = core::install_core_zip(&bytes, &core_dir).map_err(|e| e.to_string())?;
    let v = core::core_version(&exe);
    push_line(&app, &shared, &format!("INFO  内核安装完成：{v}"));

    {
        let mut s = shared.lock().unwrap();
        s.core = Some(exe.clone());
        s.core_version = v.clone();
        s.state.setting.core_path = exe.to_string_lossy().to_string();
        let p = s.paths.clone();
        let st = s.state.clone();
        let _ = store::save_state(&p, &st);
    }

    if was_running {
        let app2 = app.clone();
        let shared2 = shared.clone();
        std::thread::spawn(move || {
            let _ = do_start(app2, shared2);
        });
    }
    Ok(json!({ "version": v }))
}

// ---------------------------------------------------------------- 节点测速

fn clash_json(port: u16, path: &str) -> Result<Value, String> {
    let p = path.to_string();
    blocking_http(move || {
        let c = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| e.to_string())?;
        c.get(format!("http://127.0.0.1:{}{}", port, p))
            .send()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub async fn test_nodes(
    app: AppHandle,
    state: State<'_, SharedState>,
    ids: Vec<String>,
) -> Result<Value, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || do_test_nodes(app, shared, ids))
        .await
        .map_err(|e| e.to_string())?
}

fn do_test_nodes(app: AppHandle, shared: SharedState, ids: Vec<String>) -> Result<Value, String> {
    let (app_state, core_path, set) = {
        let s = shared.lock().unwrap();
        let setting = s.state.setting.clone();
        (
            s.state.clone(),
            core::find_core(&s.paths, &setting),
            setting,
        )
    };
    let core_path = core_path.ok_or_else(|| "未找到内核，无法测速".to_string())?;

    let nodes: Vec<Node> = app_state
        .nodes
        .iter()
        .filter(|n| n.enabled && !n.server.is_empty() && n.port > 0)
        .cloned()
        .collect();
    if nodes.is_empty() {
        return Err("没有可用的节点".into());
    }
    let tags: Vec<(String, String)> = singbox::node_tags(&nodes);

    let targets: Vec<(usize, Node)> = nodes
        .iter()
        .cloned()
        .enumerate()
        .filter(|(_, n)| ids.is_empty() || ids.contains(&n.id))
        .collect();
    if targets.is_empty() {
        return Err("没有匹配的节点".into());
    }

    let probe_clash = if set.clash_api_port >= 65000 {
        set.clash_api_port - 1
    } else {
        set.clash_api_port + 1
    };
    let probe_mixed = core::free_port();

    let cfg = singbox::build_probe_config(&app_state, probe_clash, probe_mixed);
    let paths = shared.lock().unwrap().paths.clone();
    let probe_file = paths.probe_file();
    std::fs::write(&probe_file, serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    if let Err(e) = core::check_config(&core_path, &probe_file) {
        return Err(format!("测速配置无效: {e}"));
    }

    let mut child = core::spawn_core(&core_path, &probe_file, &paths.runtime)
        .map_err(|e| format!("启动测速内核失败: {e}"))?;

    // 等待 API 就绪
    let mut ready = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if clash_json(probe_clash, "/version").is_ok() {
            ready = true;
            break;
        }
    }

    let mut result = serde_json::Map::new();
    if ready {
        for (idx, node) in &targets {
            let tag = &tags[*idx].1;
            let path = format!(
                "/proxies/{}/delay?timeout=5000&url={}",
                urlencoding::encode(tag),
                urlencoding::encode("http://www.gstatic.com/generate_204")
            );
            let delay = match clash_json(probe_clash, &path) {
                Ok(v) => v["delay"].as_i64().unwrap_or(-1),
                Err(_) => -1,
            };
            result.insert(node.id.clone(), json!(delay));
            let _ = app.emit("node-delay", json!({ "id": node.id, "delay": delay }));
        }
    }
    core::kill_child(&mut child);
    let _ = std::fs::remove_file(&probe_file);

    if !ready {
        return Err("测速内核启动超时，请检查网络或节点配置".into());
    }
    Ok(Value::Object(result))
}

// ---------------------------------------------------------------- 订阅 / 导入

#[tauri::command]
pub fn parse_links(text: String) -> Vec<Node> {
    sub::parse_subscription(&text)
}

#[tauri::command]
pub async fn update_subscription(
    app: AppHandle,
    state: State<'_, SharedState>,
    id: String,
) -> Result<Value, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || do_update_sub(app, shared, id))
        .await
        .map_err(|e| e.to_string())?
}

fn do_update_sub(app: AppHandle, shared: SharedState, id: String) -> Result<Value, String> {
    let (url, proxy, all_nodes_len) = {
        let s = shared.lock().unwrap();
        let sub = s
            .state
            .subscriptions
            .iter()
            .find(|x| x.id == id)
            .ok_or_else(|| "订阅不存在".to_string())?;
        (
            sub.url.clone(),
            core::update_proxy_url(&s.state.setting, s.child.is_some()),
            s.state.nodes.len(),
        )
    };
    let _ = all_nodes_len;

    push_line(&app, &shared, &format!("INFO  更新订阅: {url}"));
    let c = core::client(proxy.as_deref()).map_err(|e| e.to_string())?;
    let text = c
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .map_err(|e| e.to_string())?;

    let mut nodes = sub::parse_subscription(&text);
    if nodes.is_empty() {
        return Err("订阅内容解析后为空，请检查链接".into());
    }
    // 生成 id
    for (i, n) in nodes.iter_mut().enumerate() {
        n.id = format!("{}-{}", id, i);
        n.group = id.clone();
    }

    let count = nodes.len();
    {
        let mut s = shared.lock().unwrap();
        // 移除该订阅旧节点
        let keep: Vec<Node> = s
            .state
            .nodes
            .iter()
            .filter(|n| n.group != id)
            .cloned()
            .collect();
        let mut merged = keep;
        merged.extend(nodes.clone());
        s.state.nodes = merged;
        if let Some(sb) = s.state.subscriptions.iter_mut().find(|x| x.id == id) {
            sb.node_ids = nodes.iter().map(|n| n.id.clone()).collect();
            sb.last_update = store::now_datetime();
        }
        let p = s.paths.clone();
        let st = s.state.clone();
        let _ = store::save_state(&p, &st);
    }
    push_line(&app, &shared, &format!("INFO  订阅更新完成，共 {count} 个节点"));
    Ok(json!({ "count": count, "nodes": nodes }))
}

// ---------------------------------------------------------------- 客户端自身更新

/// 解析 "v1.2.3" / "1.2" / "1.2.3-beta" → (major, minor, patch)
fn parse_ver(v: &str) -> (u32, u32, u32) {
    let s = v.trim().trim_start_matches('v');
    let core = s.split(['-', '+']).next().unwrap_or("0");
    let mut it = core.split('.').map(|x| x.trim());
    let a = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let b = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let c = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    (a, b, c)
}

fn ver_gt(a: &str, b: &str) -> bool {
    parse_ver(a) > parse_ver(b)
}

/// 拉取版本清单。格式：
/// { "version": "0.2.0", "notes": "更新说明", "url": "https://.../Mvpn-0.2.0-win-x64.zip" }
fn fetch_manifest(url: &str, proxy: Option<&str>) -> Result<Value, String> {
    let u = url.to_string();
    let px = proxy.map(|s| s.to_string());
    blocking_http(move || {
        let c = core::client(px.as_deref()).map_err(|e| e.to_string())?;
        let v: Value = c
            .get(&u)
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| format!("版本清单格式错误: {e}"))?;
        Ok(v)
    })
}

#[tauri::command]
pub async fn check_app_update(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Value, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || do_check_app_update(app, shared))
        .await
        .map_err(|e| e.to_string())?
}

fn do_check_app_update(app: AppHandle, shared: SharedState) -> Result<Value, String> {
    let (url, proxy) = {
        let s = shared.lock().unwrap();
        (
            s.state.setting.update_manifest_url.trim().to_string(),
            core::update_proxy_url(&s.state.setting, s.child.is_some()),
        )
    };
    let current = env!("CARGO_PKG_VERSION").to_string();
    if url.is_empty() {
        return Ok(json!({
            "enabled": false, "hasUpdate": false, "current": current,
            "reason": "未配置版本清单地址（设置 → 高级）"
        }));
    }
    let m = fetch_manifest(&url, proxy.as_deref())?;
    let latest = m["version"].as_str().unwrap_or("").trim().to_string();
    if latest.is_empty() {
        return Err("版本清单缺少 version 字段".into());
    }
    let has = ver_gt(&latest, &current);
    let msg = format!(
        "{} 客户端更新：{} → {}",
        if has { "发现" } else { "已是最新" },
        current,
        latest
    );
    push_line(&app, &shared, &format!("INFO  {msg}"));
    Ok(json!({
        "enabled": true, "hasUpdate": has, "current": current, "latest": latest,
        "notes": m["notes"].as_str().unwrap_or(""), "url": m["url"].as_str().unwrap_or(""),
        "manifest": url
    }))
}

#[tauri::command]
pub async fn update_app(app: AppHandle, state: State<'_, SharedState>) -> Result<Value, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || do_update_app(app, shared))
        .await
        .map_err(|e| e.to_string())?
}

fn do_update_app(app: AppHandle, shared: SharedState) -> Result<Value, String> {
    let (url, proxy) = {
        let s = shared.lock().unwrap();
        (
            s.state.setting.update_manifest_url.trim().to_string(),
            core::update_proxy_url(&s.state.setting, s.child.is_some()),
        )
    };
    if url.is_empty() {
        return Err("未配置版本清单地址".into());
    }
    let m = fetch_manifest(&url, proxy.as_deref())?;
    let latest = m["version"].as_str().unwrap_or("").trim().to_string();
    let asset = m["url"].as_str().unwrap_or("").trim().to_string();
    if asset.is_empty() {
        return Err("版本清单缺少下载地址".into());
    }
    let current = env!("CARGO_PKG_VERSION");
    if !ver_gt(&latest, current) {
        return Ok(json!({ "updated": false, "reason": "已是最新版本" }));
    }

    // 先停内核，避免占用文件与网络
    do_stop(&shared, false);

    push_line(&app, &shared, &format!("INFO  开始下载客户端 {latest} …"));
    let bytes = core::download_bytes(&asset, proxy.as_deref()).map_err(|e| e.to_string())?;

    // 解包出 exe（兼容 zip 与裸 exe）
    let staged = stage_new_exe(&bytes)?;

    // 替换：Windows 允许重命名运行中的 exe
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let old = exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| {
        format!(
            "无法替换正在运行的程序（{}）。已把新版本保存到: {}",
            e,
            staged.display()
        )
    })?;
    if let Err(e) = std::fs::rename(&staged, &exe) {
        let _ = std::fs::rename(&old, &exe); // 回滚
        return Err(format!("替换失败: {e}"));
    }
    push_line(&app, &shared, &format!("INFO  已更新到 {latest}，正在重启"));
    let _ = std::fs::remove_file(&old);

    // 重启新版本
    let _ = std::process::Command::new(&exe).spawn();
    Ok(json!({ "updated": true, "version": latest }))
}

/// 从 zip（取第一个 .exe）或裸二进制中取出新 exe，返回落盘路径
fn stage_new_exe(bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir());
    let staged = dir.join("Mvpn.new.exe");

    if bytes.len() > 2 && bytes[0] == b'P' && bytes[1] == b'K' {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| format!("解压失败: {e}"))?;
        let mut found = false;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = f.name().to_ascii_lowercase();
            let base = name.rsplit('/').next().unwrap_or("");
            if base.ends_with(".exe") && !base.contains("webview2") && !base.contains("sing-box") {
                let mut w = std::fs::File::create(&staged).map_err(|e| e.to_string())?;
                std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
                found = true;
                break;
            }
        }
        if !found {
            return Err("压缩包内未找到 Mvpn.exe".into());
        }
    } else {
        std::fs::write(&staged, bytes).map_err(|e| e.to_string())?;
    }
    Ok(staged)
}

// ---------------------------------------------------------------- 开机自启

#[tauri::command]
pub fn set_autostart(on: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (k, _) = hkcu
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .map_err(|e| e.to_string())?;
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let val = format!("\"{}\" --minimized", exe.to_string_lossy());
        k.set_value("Mvpn", &val).map_err(|e| e.to_string())?;
    } else {
        let _ = k.delete_value("Mvpn");
    }
    Ok(())
}

#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
    Ok(())
}

/// 供前端确认当前出口
#[tauri::command]
pub fn outbound_tags(state: State<'_, SharedState>) -> Value {
    let s = state.lock().unwrap();
    let nodes: Vec<Node> = s.state.nodes.iter().filter(|n| n.enabled).cloned().collect();
    let tags: Vec<String> = singbox::node_tags(&nodes)
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    json!({ "direct": TAG_DIRECT, "proxy": TAG_PROXY, "nodes": tags })
}
