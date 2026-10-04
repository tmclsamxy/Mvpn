#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod core;
mod model;
mod singbox;
mod stats;
mod store;
mod sub;
mod sysproxy;

use commands::{Shared, SharedState};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};

/// 分享链接解析自检
fn selftest_links() -> i32 {
    use base64::Engine;
    let vmess_json = r#"{"v":"2","ps":"测试-VMess","add":"vm.example.com","port":"443","id":"bf000d23-0752-40b4-affe-68f7707a9661","aid":"0","scy":"auto","net":"ws","type":"none","host":"cdn.example.com","path":"/vm","tls":"tls","sni":"vm.example.com"}"#;
    let vmess_link = format!(
        "vmess://{}",
        base64::engine::general_purpose::STANDARD.encode(vmess_json)
    );
    let ss_creds = base64::engine::general_purpose::STANDARD.encode("aes-256-gcm:secretpass");
    // 旧式：整段 base64 编码的 method:password@host:port
    let ss_legacy = base64::engine::general_purpose::STANDARD
        .encode("aes-256-gcm:secretpass@ss.example.com:8389");

    let samples: Vec<String> = vec![
        "vless://bf000d23-0752-40b4-affe-68f7707a9661@vl.example.com:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.microsoft.com&fp=chrome&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS9&sid=0123abcd&type=tcp#VLESS-Reality".into(),
        vmess_link,
        "trojan://trpw@tj.example.com:443?security=tls&sni=tj.example.com&type=ws&path=/tj&host=cdn.example.com#Trojan-WS".into(),
        format!("ss://{}@ss.example.com:8388#SS-AEAD", ss_creds),
        format!("ss://{}#SS-Legacy", ss_legacy),
        "hysteria2://hypw@hy.example.com:8443?sni=hy.example.com&obfs=salamander&obfs-password=obfspw&insecure=1#Hysteria2".into(),
        "tuic://bf000d23-0752-40b4-affe-68f7707a9661:tuicpw@tu.example.com:443?sni=tu.example.com&congestion_control=bbr&udp_relay_mode=native#TUIC".into(),
    ];

    println!("[selftest] 分享链接解析:");
    let mut fail = 0;
    for s in &samples {
        match sub::parse_link(s) {
            Some(n) => {
                let short: String = s.chars().take(28).collect();
                println!(
                    "  ✅ {}… -> {} | {}:{} | tls={} reality={} transport={} sni={}",
                    short,
                    n.protocol,
                    n.server,
                    n.port,
                    n.tls,
                    n.reality,
                    if n.transport.is_empty() { "-" } else { &n.transport },
                    if n.sni.is_empty() { "-" } else { &n.sni }
                );
            }
            None => {
                fail += 1;
                let short: String = s.chars().take(40).collect();
                println!("  ❌ 解析失败: {short}…");
            }
        }
    }

    // 整体 Base64 订阅
    let all: String = samples.join("\n");
    let b64 = base64::engine::general_purpose::STANDARD.encode(all.as_bytes());
    let nodes = sub::parse_subscription(&b64);
    println!(
        "  {} Base64 订阅 -> 解析出 {} 个节点（期望 {}）",
        if nodes.len() == samples.len() { "✅" } else { "❌" },
        nodes.len(),
        samples.len()
    );
    if nodes.len() != samples.len() {
        fail += 1;
    }
    if fail == 0 {
        println!("[selftest] ✅ 分享链接解析全部通过");
    } else {
        println!("[selftest] ❌ 有 {fail} 项失败");
    }
    fail
}

/// 冒烟测试：不启动 GUI，直接走「生成配置 → 拉起内核 → 端口可用 → 代理可转发」全链路
fn smoke() -> i32 {
    let tmp = std::env::temp_dir().join("mvpn-smoke");
    let _ = std::fs::remove_dir_all(&tmp);
    let paths = store::Paths::new(tmp);

    // 优先使用真实配置（%APPDATA%\com.mvpn.client\state.json），这样能直接验证用户真实节点
    let real = std::env::var("APPDATA")
        .ok()
        .map(|a| std::path::Path::new(&a).join("com.mvpn.client").join("state.json"))
        .filter(|p| p.is_file());
    let mut st = match &real {
        Some(p) => {
            let s = store::load_state(&store::Paths::new(
                p.parent().unwrap().to_path_buf(),
            ));
            println!("[smoke] 使用真实配置: {}", p.display());
            s
        }
        None => {
            println!("[smoke] 未找到真实配置，使用内置样例");
            store::default_state()
        }
    };
    let nodes: Vec<String> = st
        .nodes
        .iter()
        .filter(|n| n.enabled)
        .map(|n| format!("{}({}→{})", n.name, n.protocol, n.server))
        .collect();
    println!("[smoke] 节点: {}", if nodes.is_empty() { "无".into() } else { nodes.join(", ") });
    println!("[smoke] 模式: {}", st.setting.mode);

    st.setting.tun_enabled = false;
    st.setting.mixed_enabled = true;
    st.setting.mixed_allow_lan = false;
    st.setting.sys_proxy = false;
    st.setting.mixed_port = 28080;

    let cfg_path = paths.config_file();
    let text = serde_json::to_string_pretty(&singbox::build_config(&st, &paths.runtime)).unwrap();
    std::fs::write(&cfg_path, &text).unwrap();

    let core = match std::env::var("MVP_CORE")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| core::find_core(&paths, &st.setting))
    {
        Some(c) => c,
        None => {
            println!("[smoke] ❌ 未找到内核（设 MVP_CORE 或先下载内核）");
            return 1;
        }
    };
    println!("[smoke] 内核: {}", core_version(&core));

    if let Err(e) = core::check_config(&core, &cfg_path) {
        println!("[smoke] ❌ 配置校验失败: {e}");
        return 1;
    }
    println!("[smoke] ✅ 配置校验通过");

    let mut child = match core::spawn_core(&core, &cfg_path, &paths.runtime) {
        Ok(c) => c,
        Err(e) => {
            println!("[smoke] ❌ 启动内核失败: {e}");
            return 1;
        }
    };

    let addr = format!("127.0.0.1:{}", st.setting.mixed_port);
    let mut up = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if std::net::TcpStream::connect(&addr).is_ok() {
            up = true;
            break;
        }
    }
    if !up {
        core::kill_child(&mut child);
        println!("[smoke] ❌ 混合代理端口未就绪 ({addr})");
        return 1;
    }
    println!("[smoke] ✅ 混合代理端口已监听 {addr}");

    // 实际发起请求：先直连（对照组），再经代理
    let probe = |extra: Vec<&str>| -> String {
        let mut args: Vec<String> = vec![
            "-sS".into(), "-o".into(), "NUL".into(), "-w".into(), "%{http_code}".into(),
            "--max-time".into(), "20".into(),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        args.push("https://www.gstatic.com/generate_204".into());
        match std::process::Command::new("curl").args(&args).output() {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(e) => format!("ERR({e})"),
        }
    };

    let direct = probe(vec![]);
    println!("[smoke] 直连 gstatic: {direct}");
    let via = probe(vec!["-x", &format!("http://{addr}")]);
    println!("[smoke] 经代理 gstatic: {via}");

    if via == "204" {
        println!("[smoke] ✅ 代理链路可用（节点已成功承载外网流量）");
        core::kill_child(&mut child);
        println!("[smoke] 🎉 全链路冒烟测试通过");
        0
    } else {
        core::kill_child(&mut child);
        if direct == "204" {
            println!("[smoke] ❌ 直连能通但经代理不通（返回 {via}）→ 代理链路有问题");
        } else {
            println!("[smoke] ❌ 经代理未拿到 204（返回 {via}）→ 节点/协议/分流需排查");
        }
        1
    }
}

/// 系统代理底层机制自检：直接验证「写 HKCU + InternetSetOption 通知」是否真的生效
fn proxtest(on: bool) -> i32 {
    let bypass = "localhost;127.*;10.*;172.16.*;192.168.*;<local>";
    let r = if on {
        sysproxy::enable("127.0.0.1:2080", bypass)
    } else {
        sysproxy::disable()
    };
    match r {
        Ok(_) => {
            match sysproxy::current() {
                Some(v) => println!("[proxytest] ✅ 操作成功，读回 ProxyServer = {v}"),
                None => println!(
                    "[proxytest] {} 操作返回成功，但读回为空（ProxyEnable=0）",
                    if on { "❌" } else { "✅" }
                ),
            }
            0
        }
        Err(e) => {
            println!("[proxytest] ❌ 操作失败: {e}");
            1
        }
    }
}

/// 探针：验证 reqwest::blocking 在「普通线程」与「tokio 运行时上下文」中的行为差异
fn rtcheck() -> i32 {
    let call = || -> String {
        match core::client(None) {
            Ok(c) => match c.get("http://127.0.0.1:9/").send() {
                Ok(_) => "send-ok".to_string(),
                Err(e) => format!("send-err({})", first_line(&e.to_string())),
            },
            Err(e) => format!("client-err({})", first_line(&e.to_string())),
        }
    };

    // A) 普通 std::thread
    let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(call);
        let _ = tx.send(r.map_err(|_| "PANIC".to_string()));
    });
    let a: Result<String, String> = rx.recv().unwrap_or(Err("JOIN-FAIL".into()));
    println!("[rtcheck] A 普通 std::thread            -> {a:?}");

    // B) tauri::async_runtime::spawn_blocking（tokio 运行时上下文）
    let (tx2, rx2) = std::sync::mpsc::channel::<Result<String, String>>();
    tauri::async_runtime::spawn_blocking(move || {
        let r = std::panic::catch_unwind(call);
        let _ = tx2.send(r.map_err(|_| "PANIC".to_string()));
    });
    let b: Result<String, String> = rx2.recv().unwrap_or(Err("JOIN-FAIL".into()));
    println!("[rtcheck] B spawn_blocking(运行时上下文) -> {b:?}");

    let ok = a.is_ok() && b.is_ok();
    if ok {
        println!("[rtcheck] ✅ 两处调用均未 panic");
        0
    } else {
        println!("[rtcheck] ❌ 存在 panic —— reqwest::blocking 不能直接在 tokio 上下文里调用");
        1
    }
}

/// 监控链路自检：拉起内核 → 造流量 → 用生产同款 poll_connections 验证速率/连接/归因
fn monitortest() -> i32 {
    use std::collections::HashMap;
    let tmp = std::env::temp_dir().join("mvpn-monitortest");
    let _ = std::fs::remove_dir_all(&tmp);
    let paths = store::Paths::new(tmp);

    let mut st = store::default_state();
    st.setting.tun_enabled = false;
    st.setting.mixed_enabled = true;
    st.setting.mixed_port = 28081;
    st.setting.clash_api_port = 19997;

    let cfg = singbox::build_config(&st, &paths.runtime);
    let cfg_path = paths.config_file();
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();

    let core = match std::env::var("MVP_CORE")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| core::find_core(&paths, &st.setting))
    {
        Some(c) => c,
        None => {
            println!("[monitor] ❌ 未找到内核");
            return 1;
        }
    };
    if let Err(e) = core::check_config(&core, &cfg_path) {
        println!("[monitor] ❌ 配置校验失败: {e}");
        return 1;
    }
    let mut child = match core::spawn_core(&core, &cfg_path, &paths.runtime) {
        Ok(c) => c,
        Err(e) => {
            println!("[monitor] ❌ 启动失败: {e}");
            return 1;
        }
    };
    std::thread::sleep(std::time::Duration::from_millis(2500));

    // 后台持续造流量
    std::thread::spawn(|| {
        for _ in 0..6 {
            let _ = std::process::Command::new("curl")
                .args([
                    "-sS", "-o", "NUL", "--max-time", "6", "-x", "http://127.0.0.1:28081",
                    "https://speed.cloudflare.com/__down?bytes=2000000",
                ])
                .output();
        }
    });

    let client = match reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            println!("[monitor] ❌ 建 client 失败: {e}");
            core::kill_child(&mut child);
            return 1;
        }
    };

    let mut prev_totals = stats::Bytes::default();
    let mut prev_conn: HashMap<String, (String, u64, u64)> = HashMap::new();
    let tag2id: HashMap<String, String> = HashMap::new();
    let mut acc = stats::TrafficStats::default();
    let mut peak = 0u64;
    let mut rows_seen = 0usize;
    let mut sample_row = String::new();

    for i in 0..18 {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        match commands::poll_connections(&client, 19997, &mut prev_totals, &mut prev_conn, &tag2id) {
            Ok(p) => {
                if p.delta.down > peak {
                    peak = p.delta.down;
                }
                if !p.rows.is_empty() {
                    rows_seen = p.rows.len();
                    if sample_row.is_empty() {
                        if let Some(r) = p.rows.first() {
                            sample_row = format!(
                                "{} | 进程 {} | 链路 {}",
                                r["target"].as_str().unwrap_or("-"),
                                r["process"].as_str().unwrap_or("-"),
                                r["chain"].as_str().unwrap_or("-")
                            );
                        }
                    }
                }
                acc.accumulate("2026-10-04", p.delta, &p.per_node);
                if i == 4 {
                    println!("[monitor] 第 5 次采样: 本轮下载 {} B", p.delta.down);
                }
            }
            Err(e) => {
                if i == 0 {
                    println!("[monitor] 首次采样失败: {e}");
                }
            }
        }
    }
    core::kill_child(&mut child);

    let today = acc.today("2026-10-04");
    println!("[monitor] 峰值速率   : {peak} B/s");
    println!("[monitor] 今日累计   : 下载 {} B / 上传 {} B", today.down, today.up);
    println!("[monitor] 连接条目数 : {rows_seen}");
    if !sample_row.is_empty() {
        println!("[monitor] 连接样例   : {sample_row}");
    }
    let ok = peak > 0 && today.down > 0 && rows_seen > 0;
    if ok {
        println!("[monitor] 🎉 监控链路正常（速率/累计/连接列表均有数据）");
        0
    } else {
        println!("[monitor] ❌ 监控链路异常：peak={peak} today={} rows={rows_seen}", today.down);
        1
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(90).collect()
}

fn core_version(core: &std::path::Path) -> String {
    core::core_version(core)
}

/// 命令行自检：生成配置并用内核校验（无需 GUI）
fn selftest() -> i32 {
    let rc = selftest_config();
    let rf = selftest_links();
    if rc == 0 && rf == 0 {
        println!("\n[selftest] 🎉 全部自检通过");
        0
    } else {
        1
    }
}

fn selftest_config() -> i32 {
    let tmp = std::env::temp_dir().join("mvpn-selftest");
    let _ = std::fs::create_dir_all(&tmp);
    let paths = store::Paths::new(tmp.clone());

    let mut st = store::default_state();
    st.nodes = vec![
        model::Node {
            id: "t1".into(),
            name: "VLESS-Reality".into(),
            protocol: "vless".into(),
            server: "1.2.3.4".into(),
            port: 443,
            uuid: "bf000d23-0752-40b4-affe-68f7707a9661".into(),
            flow: "xtls-rprx-vision".into(),
            tls: true,
            reality: true,
            sni: "www.microsoft.com".into(),
            reality_public_key: "jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS9".into(),
            reality_short_id: "0123abcd".into(),
            fingerprint: "chrome".into(),
            ..Default::default()
        },
        model::Node {
            id: "t2".into(),
            name: "Hysteria2".into(),
            protocol: "hysteria2".into(),
            server: "5.6.7.8".into(),
            port: 8443,
            password: "pw123".into(),
            tls: true,
            sni: "example.com".into(),
            obfs: "salamander".into(),
            obfs_password: "obfspw".into(),
            allow_insecure: true,
            ..Default::default()
        },
        model::Node {
            id: "t3".into(),
            name: "SS-2022".into(),
            protocol: "shadowsocks".into(),
            server: "9.9.9.9".into(),
            port: 8388,
            method: "2022-blake3-aes-128-gcm".into(),
            password: "cGFzc3dvcmQxMjM0NTY3OA==".into(),
            ..Default::default()
        },
        model::Node {
            id: "t4".into(),
            name: "Trojan-WS".into(),
            protocol: "trojan".into(),
            server: "11.22.33.44".into(),
            port: 443,
            password: "trojanpw".into(),
            tls: true,
            sni: "example.org".into(),
            transport: "ws".into(),
            ws_path: "/ws".into(),
            ws_host: "example.org".into(),
            ..Default::default()
        },
        model::Node {
            id: "t5".into(),
            name: "VMess-gRPC".into(),
            protocol: "vmess".into(),
            server: "55.66.77.88".into(),
            port: 443,
            uuid: "bf000d23-0752-40b4-affe-68f7707a9661".into(),
            alter_id: 0,
            security: "auto".into(),
            tls: true,
            sni: "example.net".into(),
            transport: "grpc".into(),
            grpc_service: "grpcsvc".into(),
            ..Default::default()
        },
        model::Node {
            id: "t6".into(),
            name: "TUIC".into(),
            protocol: "tuic".into(),
            server: "99.88.77.66".into(),
            port: 443,
            uuid: "bf000d23-0752-40b4-affe-68f7707a9661".into(),
            password: "tuicpw".into(),
            tls: true,
            sni: "tuic.example.com".into(),
            congestion_control: "bbr".into(),
            udp_relay_mode: "native".into(),
            ..Default::default()
        },
    ];
    st.setting.tun_enabled = true;
    st.setting.mixed_allow_lan = true;
    st.rules.push(model::RouteRule {
        id: "t-rule".into(),
        enabled: true,
        remark: "自定义规则".into(),
        action: "route".into(),
        outbound: "proxy".into(),
        domain_suffix: "google.com\nyoutube.com".into(),
        process_name: "chrome.exe".into(),
        port: "443,8443".into(),
        ..Default::default()
    });

    let cfg = singbox::build_config(&st, &paths.runtime);
    let config_file = paths.runtime.join("selftest.json");
    let text = serde_json::to_string_pretty(&cfg).unwrap();
    std::fs::write(&config_file, &text).unwrap();
    println!("[selftest] 配置已生成: {} ({} bytes)", config_file.display(), text.len());

    let core = std::env::var("MVP_CORE")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| core::find_core(&paths, &st.setting));
    match core {
        Some(c) => {
            println!("[selftest] 使用内核: {}", c.display());
            match core::check_config(&c, &config_file) {
                Ok(_) => {
                    println!("[selftest] ✅ 配置校验通过");
                    0
                }
                Err(e) => {
                    println!("[selftest] ❌ 配置校验失败:\n{e}");
                    1
                }
            }
        }
        None => {
            println!("[selftest] 未找到内核，仅生成配置（跳过校验）");
            0
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(selftest());
    }
    if args.iter().any(|a| a == "--smoke") {
        std::process::exit(smoke());
    }
    if args.iter().any(|a| a == "--proxtest-on") {
        std::process::exit(proxtest(true));
    }
    if args.iter().any(|a| a == "--proxtest-off") {
        std::process::exit(proxtest(false));
    }
    if args.iter().any(|a| a == "--rtcheck") {
        std::process::exit(rtcheck());
    }
    if args.iter().any(|a| a == "--monitortest") {
        std::process::exit(monitortest());
    }
    let start_minimized = args.iter().any(|a| a == "--minimized");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            let data_dir = handle
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let paths = store::Paths::new(data_dir);
            let state = store::load_state(&paths);
            // 内核未运行却残留着指向本机的系统代理，会让用户直接上不了网
            commands::cleanup_stale_sys_proxy(&state.setting);
            let core_found = core::find_core(&paths, &state.setting);
            let version = core_found
                .as_ref()
                .map(|c| core::core_version(c))
                .unwrap_or_default();
            let admin = core::is_admin();

            let shared: SharedState = Arc::new(Mutex::new(Shared {
                paths: paths.clone(),
                state: state.clone(),
                child: None,
                status: "stopped".into(),
                logs: VecDeque::new(),
                core: core_found,
                core_version: version,
                admin,
                monitor_stop: Arc::new(AtomicBool::new(true)),
                stats: stats::load(&paths.stats_file()),
            }));
            app.manage(shared.clone());

            // 托盘
            let show = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
            let toggle = MenuItem::with_id(app, "toggle", "启动 / 停止代理", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &toggle, &quit])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(icon)
                .tooltip("Mvpn")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event({
                    let shared = shared.clone();
                    let handle = handle.clone();
                    move |app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "toggle" => {
                            let running = shared.lock().unwrap().child.is_some();
                            let h = handle.clone();
                            let sh = shared.clone();
                            std::thread::spawn(move || {
                                if running {
                                    let _ = crate::commands::do_stop_pub(&sh, false);
                                    let _ = h.emit("core-status", "stopped");
                                } else {
                                    let _ = crate::commands::do_start_pub(h.clone(), sh);
                                }
                            });
                        }
                        "quit" => {
                            let _ = crate::commands::do_stop_pub(&shared, false);
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event({
                    let handle = handle.clone();
                    move |_tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            if let Some(w) = handle.get_webview_window("main") {
                                if w.is_visible().unwrap_or(false) {
                                    let _ = w.hide();
                                } else {
                                    let _ = w.show();
                                    let _ = w.set_focus();
                                }
                            }
                        }
                    }
                })
                .build(app)?;
            app.manage(_tray);

            if start_minimized {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }

            // 自启时自动连接
            if state.setting.auto_connect && !state.nodes.is_empty() {
                let h = handle.clone();
                let sh = shared.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    let _ = crate::commands::do_start_pub(h, sh);
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap,
            commands::get_state,
            commands::save_state,
            commands::generate_config,
            commands::get_logs,
            commands::clear_logs,
            commands::is_admin,
            commands::start_core,
            commands::stop_core,
            commands::restart_core,
            commands::core_status,
            commands::get_traffic_stats,
            commands::toggle_sys_proxy,
            commands::set_mode,
            commands::check_core_update,
            commands::download_core,
            commands::test_nodes,
            commands::parse_links,
            commands::update_subscription,
            commands::set_autostart,
            commands::check_app_update,
            commands::update_app,
            commands::open_path,
            commands::outbound_tags,
        ])
        .run(tauri::generate_context!())
        .expect("Mvpn 启动失败");
}
