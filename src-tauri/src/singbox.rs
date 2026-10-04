//! sing-box 配置生成：把应用状态翻译成 sing-box 1.14 JSON
use crate::model::*;
use serde_json::{json, Map, Value};
use std::path::Path;

/// 规则集 CDN 源
pub const CDN_JSDELIVR: &str = "https://cdn.jsdelivr.net/gh";
pub const CDN_GITHUB: &str = "https://raw.githubusercontent.com";

/// 返回 (geosite-cn, geoip-cn, geosite-ads) 三个内置规则集地址
pub fn ruleset_urls(source: &str) -> (String, String, String) {
    if source == "github" {
        (
            format!("{CDN_GITHUB}/SagerNet/sing-geosite/rule-set/geosite-geolocation-cn.srs"),
            format!("{CDN_GITHUB}/SagerNet/sing-geoip/rule-set/geoip-cn.srs"),
            format!("{CDN_GITHUB}/SagerNet/sing-geosite/rule-set/geosite-category-ads-all.srs"),
        )
    } else {
        (
            format!("{CDN_JSDELIVR}/SagerNet/sing-geosite@rule-set/geosite-geolocation-cn.srs"),
            format!("{CDN_JSDELIVR}/SagerNet/sing-geoip@rule-set/geoip-cn.srs"),
            format!("{CDN_JSDELIVR}/SagerNet/sing-geosite@rule-set/geosite-category-ads-all.srs"),
        )
    }
}

pub const TAG_PROXY: &str = "proxy";
pub const TAG_AUTO: &str = "auto";
pub const TAG_DIRECT: &str = "direct";
pub const TAG_BLOCK: &str = "block";

/// 按换行 / 逗号切分并清理
pub fn lines(s: &str) -> Vec<String> {
    s.split(['\n', '\r', ','])
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty() && !x.starts_with('#'))
        .collect()
}

/// 生成 clash_api 可用的合法 tag
pub fn sanitize_tag(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "node".into()
    } else {
        s
    }
}

/// 为所有节点分配唯一 tag（按名称，重名追加序号）
pub fn node_tags(nodes: &[Node]) -> Vec<(String, String)> {
    let mut used: Vec<String> = vec![TAG_PROXY.into(), TAG_AUTO.into(), TAG_DIRECT.into(), TAG_BLOCK.into()];
    let mut out = Vec::new();
    for n in nodes {
        let base = sanitize_tag(&n.name);
        let mut tag = base.clone();
        let mut i = 2;
        while used.contains(&tag) {
            tag = format!("{}-{}", base, i);
            i += 1;
        }
        used.push(tag.clone());
        out.push((n.id.clone(), tag));
    }
    out
}

fn insert(m: &mut Map<String, Value>, k: &str, v: Value) {
    m.insert(k.to_string(), v);
}

/// 节点 -> sing-box outbound
pub fn node_outbound(n: &Node, tag: &str, resolver: &str) -> Value {
    let mut o = Map::new();
    insert(&mut o, "tag", json!(tag));
    insert(&mut o, "server", json!(n.server));
    insert(&mut o, "server_port", json!(n.port));
    if !resolver.is_empty() {
        insert(&mut o, "domain_resolver", json!({ "server": resolver }));
    }

    match n.protocol.as_str() {
        "vless" => {
            insert(&mut o, "type", json!("vless"));
            insert(&mut o, "uuid", json!(n.uuid));
            if !n.flow.is_empty() {
                insert(&mut o, "flow", json!(n.flow));
            }
        }
        "vmess" => {
            insert(&mut o, "type", json!("vmess"));
            insert(&mut o, "uuid", json!(n.uuid));
            insert(
                &mut o,
                "security",
                json!(if n.security.is_empty() { "auto" } else { &n.security }),
            );
            insert(&mut o, "alter_id", json!(n.alter_id));
        }
        "trojan" => {
            insert(&mut o, "type", json!("trojan"));
            insert(&mut o, "password", json!(n.password));
        }
        "shadowsocks" => {
            insert(&mut o, "type", json!("shadowsocks"));
            insert(&mut o, "method", json!(n.method));
            insert(&mut o, "password", json!(n.password));
        }
        "hysteria2" => {
            insert(&mut o, "type", json!("hysteria2"));
            insert(&mut o, "password", json!(n.password));
            if !n.obfs.is_empty() {
                let mut ob = Map::new();
                insert(&mut ob, "type", json!(n.obfs));
                insert(&mut ob, "password", json!(n.obfs_password));
                insert(&mut o, "obfs", Value::Object(ob));
            }
            if n.up_mbps > 0 {
                insert(&mut o, "up_mbps", json!(n.up_mbps));
            }
            if n.down_mbps > 0 {
                insert(&mut o, "down_mbps", json!(n.down_mbps));
            }
        }
        "tuic" => {
            insert(&mut o, "type", json!("tuic"));
            insert(&mut o, "uuid", json!(n.uuid));
            insert(&mut o, "password", json!(n.password));
            insert(
                &mut o,
                "congestion_control",
                json!(if n.congestion_control.is_empty() {
                    "bbr"
                } else {
                    &n.congestion_control
                }),
            );
            insert(
                &mut o,
                "udp_relay_mode",
                json!(if n.udp_relay_mode.is_empty() {
                    "native"
                } else {
                    &n.udp_relay_mode
                }),
            );
        }
        _ => {
            insert(&mut o, "type", json!("direct"));
        }
    }

    // TLS（hysteria2 / tuic 必须启用）
    let force_tls = matches!(n.protocol.as_str(), "hysteria2" | "tuic");
    if n.tls || n.reality || force_tls {
        let mut t = Map::new();
        insert(&mut t, "enabled", json!(true));
        if !n.sni.is_empty() {
            insert(&mut t, "server_name", json!(n.sni));
        }
        let alpn = lines(&n.alpn);
        if !alpn.is_empty() {
            insert(&mut t, "alpn", json!(alpn));
        }
        if n.allow_insecure {
            insert(&mut t, "insecure", json!(true));
        }
        if !n.fingerprint.is_empty() {
            insert(
                &mut t,
                "utls",
                json!({ "enabled": true, "fingerprint": n.fingerprint }),
            );
        }
        if n.reality {
            let mut r = Map::new();
            insert(&mut r, "enabled", json!(true));
            insert(&mut r, "public_key", json!(n.reality_public_key));
            if !n.reality_short_id.is_empty() {
                insert(&mut r, "short_id", json!(n.reality_short_id));
            }
            insert(&mut t, "reality", Value::Object(r));
        }
        insert(&mut o, "tls", Value::Object(t));
    }

    // 传输层
    match n.transport.as_str() {
        "ws" => {
            let mut w = Map::new();
            insert(&mut w, "type", json!("ws"));
            insert(
                &mut w,
                "path",
                json!(if n.ws_path.is_empty() { "/" } else { &n.ws_path }),
            );
            if !n.ws_host.is_empty() {
                insert(&mut w, "headers", json!({ "Host": n.ws_host }));
            }
            if n.early_data {
                insert(&mut w, "early_data", json!(true));
            }
            insert(&mut o, "transport", Value::Object(w));
        }
        "grpc" => {
            let mut g = Map::new();
            insert(&mut g, "type", json!("grpc"));
            if !n.grpc_service.is_empty() {
                insert(&mut g, "service_name", json!(n.grpc_service));
            }
            insert(&mut o, "transport", Value::Object(g));
        }
        "http" => {
            let mut h = Map::new();
            insert(&mut h, "type", json!("http"));
            if !n.ws_host.is_empty() {
                insert(&mut h, "host", json!(lines(&n.ws_host)));
            }
            insert(
                &mut h,
                "path",
                json!(if n.ws_path.is_empty() { "/" } else { &n.ws_path }),
            );
            insert(&mut o, "transport", Value::Object(h));
        }
        "quic" => {
            insert(&mut o, "transport", json!({ "type": "quic" }));
        }
        _ => {}
    }

    Value::Object(o)
}

/// 解析 DNS 字符串为 sing-box dns server 对象
fn dns_server(tag: &str, raw: &str, detour: &str) -> Value {
    let mut m = Map::new();
    insert(&mut m, "tag", json!(tag));
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix("https://") {
        let host = rest.split('/').next().unwrap_or(rest);
        insert(&mut m, "type", json!("https"));
        insert(&mut m, "server", json!(host));
        insert(&mut m, "path", json!("/dns-query"));
    } else if let Some(rest) = s.strip_prefix("tls://") {
        insert(&mut m, "type", json!("tls"));
        insert(&mut m, "server", json!(rest));
    } else if let Some(rest) = s.strip_prefix("quic://") {
        insert(&mut m, "type", json!("quic"));
        insert(&mut m, "server", json!(rest));
    } else if let Some(rest) = s.strip_prefix("h3://") {
        insert(&mut m, "type", json!("h3"));
        insert(&mut m, "server", json!(rest));
    } else {
        let host = s.strip_prefix("udp://").unwrap_or(s);
        insert(&mut m, "type", json!("udp"));
        insert(
            &mut m,
            "server",
            json!(if host.is_empty() { "223.5.5.5" } else { host }),
        );
    }
    if !detour.is_empty() {
        insert(&mut m, "detour", json!(detour));
    }
    Value::Object(m)
}

/// 收集生效的规则集定义
pub fn effective_rule_sets(st: &AppState) -> Vec<RuleSetDef> {
    let mut out: Vec<RuleSetDef> = Vec::new();
    let push = |tag: &str, url: &str, out: &mut Vec<RuleSetDef>| {
        if out.iter().any(|r| r.tag == tag) {
            return;
        }
        out.push(RuleSetDef {
            tag: tag.into(),
            kind: "remote".into(),
            format: "binary".into(),
            url: url.into(),
            path: String::new(),
        });
    };
    let (geosite_cn, geoip_cn, _ads) = ruleset_urls(&st.setting.ruleset_source);
    if st.setting.cn_direct {
        push("geosite-cn", &geosite_cn, &mut out);
        push("geoip-cn", &geoip_cn, &mut out);
    }
    for r in &st.rule_sets {
        if r.tag.trim().is_empty() {
            continue;
        }
        if out.iter().any(|x| x.tag == r.tag) {
            continue;
        }
        out.push(r.clone());
    }
    out
}

fn rule_set_value(r: &RuleSetDef) -> Value {
    let mut m = Map::new();
    insert(&mut m, "tag", json!(r.tag));
    if r.kind == "local" {
        insert(&mut m, "type", json!("local"));
        insert(
            &mut m,
            "format",
            json!(if r.format.is_empty() { "binary" } else { &r.format }),
        );
        insert(&mut m, "path", json!(r.path));
    } else {
        insert(&mut m, "type", json!("remote"));
        insert(
            &mut m,
            "format",
            json!(if r.format.is_empty() { "binary" } else { &r.format }),
        );
        insert(&mut m, "url", json!(r.url));
        insert(&mut m, "update_interval", json!("1d"));
    }
    Value::Object(m)
}

/// 把一条自定义规则转换为 sing-box route rule
fn user_rule_value(r: &RouteRule) -> Option<Value> {
    let mut m = Map::new();
    let action = if r.action.is_empty() { "route" } else { r.action.as_str() };

    if action == "route" {
        if !r.outbound.is_empty() {
            insert(&mut m, "outbound", json!(r.outbound));
        }
    }
    insert(&mut m, "action", json!(action));

    macro_rules! arr {
        ($field:ident, $key:literal) => {
            let v = lines(&r.$field);
            if !v.is_empty() {
                insert(&mut m, $key, json!(v));
            }
        };
    }
    arr!(domain, "domain");
    arr!(domain_suffix, "domain_suffix");
    arr!(domain_keyword, "domain_keyword");
    arr!(ip_cidr, "ip_cidr");
    arr!(source_ip_cidr, "source_ip_cidr");
    arr!(process_name, "process_name");
    arr!(rule_set, "rule_set");
    arr!(protocol, "protocol");
    arr!(inbound, "inbound");

    if !r.port.is_empty() {
        let ports: Vec<Value> = lines(&r.port)
            .iter()
            .filter_map(|p| {
                p.parse::<u16>()
                    .ok()
                    .map(|n| json!(n))
                    .or_else(|| Some(json!(p)))
            })
            .collect();
        if !ports.is_empty() {
            insert(&mut m, "port", Value::Array(ports));
        }
    }
    if !r.clash_mode.is_empty() {
        insert(&mut m, "clash_mode", json!(r.clash_mode));
    }
    if r.invert {
        insert(&mut m, "invert", json!(true));
    }

    // 没有任何匹配条件且不是 sniff/reject 动作 -> 丢弃
    let has_matcher = m.keys().any(|k| {
        !matches!(
            k.as_str(),
            "action" | "outbound" | "invert" | "clash_mode"
        )
    });
    if !has_matcher && action != "reject" && action != "sniff" {
        return None;
    }
    Some(Value::Object(m))
}

/// 构建完整 sing-box 配置
pub fn build_config(st: &AppState, runtime_dir: &Path) -> Value {
    let s = &st.setting;

    // 可用节点
    let nodes: Vec<Node> = st
        .nodes
        .iter()
        .filter(|n| n.enabled && !n.server.is_empty() && n.port > 0)
        .cloned()
        .collect();
    let tags = node_tags(&nodes);
    let has_nodes = !nodes.is_empty();

    // ---------- outbounds ----------
    let mut outbounds: Vec<Value> = Vec::new();

    if has_nodes {
        let mut members: Vec<String> = Vec::new();
        if nodes.len() > 1 {
            members.push(TAG_AUTO.into());
        }
        for (_, t) in &tags {
            members.push(t.clone());
        }
        members.push(TAG_DIRECT.into());

        let default = if !st.active_node.is_empty() {
            tags.iter()
                .find(|(id, _)| *id == st.active_node)
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| members[0].clone())
        } else {
            members[0].clone()
        };

        outbounds.push(json!({
            "type": "selector",
            "tag": TAG_PROXY,
            "outbounds": members,
            "default": default,
            "interrupt_exist_connections": false
        }));

        if nodes.len() > 1 {
            let node_tags_only: Vec<String> = tags.iter().map(|(_, t)| t.clone()).collect();
            outbounds.push(json!({
                "type": "urltest",
                "tag": TAG_AUTO,
                "outbounds": node_tags_only,
                "url": "http://www.gstatic.com/generate_204",
                "interval": "3m",
                "tolerance": 50,
                "idle_timeout": "30m"
            }));
        }

        for (i, n) in nodes.iter().enumerate() {
            outbounds.push(node_outbound(n, &tags[i].1, "dns-local"));
        }
    } else {
        // 无节点：保留 proxy 标签指向 direct，保证规则不报错
        outbounds.push(json!({
            "type": "selector",
            "tag": TAG_PROXY,
            "outbounds": [TAG_DIRECT],
            "default": TAG_DIRECT
        }));
    }

    outbounds.push(json!({ "type": "direct", "tag": TAG_DIRECT, "domain_resolver": { "server": "dns-local" } }));
    outbounds.push(json!({ "type": "block", "tag": TAG_BLOCK }));

    // ---------- inbounds ----------
    let mut inbounds: Vec<Value> = Vec::new();
    if s.mixed_enabled {
        let mut m = Map::new();
        insert(&mut m, "type", json!("mixed"));
        insert(&mut m, "tag", json!("mixed-in"));
        insert(
            &mut m,
            "listen",
            json!(if s.mixed_allow_lan { "0.0.0.0" } else { "127.0.0.1" }),
        );
        insert(&mut m, "listen_port", json!(s.mixed_port));
        if s.mixed_auth && !s.mixed_username.is_empty() {
            insert(
                &mut m,
                "users",
                json!([{ "username": s.mixed_username, "password": s.mixed_password }]),
            );
        }
        inbounds.push(Value::Object(m));
    }
    if s.tun_enabled {
        let mut t = Map::new();
        insert(&mut t, "type", json!("tun"));
        insert(&mut t, "tag", json!("tun-in"));
        insert(
            &mut t,
            "address",
            json!(["172.19.0.1/30", "fd00::1/126"]),
        );
        insert(&mut t, "mtu", json!(s.tun_mtu));
        insert(&mut t, "auto_route", json!(s.tun_auto_route));
        insert(&mut t, "strict_route", json!(s.tun_strict_route));
        insert(
            &mut t,
            "stack",
            json!(if s.tun_stack.is_empty() { "mixed" } else { &s.tun_stack }),
        );
        insert(&mut t, "endpoint_independent_nat", json!(false));
        // 注意：入站的 sniff 字段在 sing-box 1.13 已被移除，改为在 route.rules 里用 {"action":"sniff"}
        // tun.platform.http_proxy 仅适用于 Android/Apple；Windows 由应用自身设置系统代理
        inbounds.push(Value::Object(t));
    }

    // ---------- route ----------
    // 规则集下载走哪个出站（有节点则走代理，否则直连）
    let detour = if has_nodes { TAG_PROXY } else { TAG_DIRECT };
    const HTTP_CLIENT_TAG: &str = "rule-set-client";

    let mut rules: Vec<Value> = Vec::new();
    rules.push(json!({ "action": "sniff" }));
    rules.push(json!({ "protocol": "dns", "action": "hijack-dns" }));
    rules.push(json!({ "clash_mode": "direct", "action": "route", "outbound": TAG_DIRECT }));
    rules.push(json!({ "clash_mode": "global", "action": "route", "outbound": TAG_PROXY }));
    rules.push(json!({ "ip_is_private": true, "action": "route", "outbound": TAG_DIRECT }));

    for r in &st.rules {
        if !r.enabled {
            continue;
        }
        if let Some(v) = user_rule_value(r) {
            rules.push(v);
        }
    }

    if s.cn_direct && has_nodes {
        rules.push(json!({ "rule_set": "geosite-cn", "action": "route", "outbound": TAG_DIRECT }));
        rules.push(json!({ "rule_set": "geoip-cn", "action": "route", "outbound": TAG_DIRECT }));
    }

    let rule_sets: Vec<Value> = effective_rule_sets(st)
        .iter()
        .map(rule_set_value)
        .collect();

    let final_out = if !has_nodes {
        TAG_DIRECT
    } else if s.final_outbound.is_empty() {
        TAG_PROXY
    } else {
        s.final_outbound.as_str()
    };

    let route = json!({
        "rules": rules,
        "rule_set": rule_sets,
        "final": final_out,
        "auto_detect_interface": s.auto_detect_interface,
        // 开启后连接信息里会带 processPath，便于 UI 显示"哪个程序在走代理"
        "find_process": true,
        "default_http_client": HTTP_CLIENT_TAG,
        "default_domain_resolver": { "server": "dns-local" }
    });

    // ---------- dns ----------
    let dns = json!({
        "servers": [
            dns_server("dns-local", &s.dns_local, TAG_DIRECT),
            dns_server("dns-remote", &s.dns_remote, detour)
        ],
        "rules": if s.dns_cn_direct && has_nodes {
            json!([{ "rule_set": "geosite-cn", "server": "dns-local" }])
        } else {
            json!([])
        },
        "final": "dns-remote",
        "strategy": s.dns_strategy
    });

    // ---------- experimental ----------
    let cache = runtime_dir.join("cache.db");
    let experimental = json!({
        "cache_file": { "enabled": true, "path": cache.to_string_lossy() },
        "clash_api": {
            "external_controller": format!("127.0.0.1:{}", s.clash_api_port),
            "default_mode": "rule"
        }
    });

    json!({
        "log": {
            "level": s.log_level,
            "timestamp": true
        },
        "http_clients": [
            { "tag": HTTP_CLIENT_TAG, "detour": detour }
        ],
        "dns": dns,
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": route,
        "experimental": experimental
    })
}

/// 构建用于「节点延迟测试」的临时配置
pub fn build_probe_config(st: &AppState, clash_port: u16, mixed_port: u16) -> Value {
    let nodes: Vec<Node> = st
        .nodes
        .iter()
        .filter(|n| n.enabled && !n.server.is_empty() && n.port > 0)
        .cloned()
        .collect();
    let tags = node_tags(&nodes);

    let mut outbounds: Vec<Value> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        outbounds.push(node_outbound(n, &tags[i].1, "dns-local"));
    }
    outbounds.push(json!({ "type": "direct", "tag": TAG_DIRECT, "domain_resolver": { "server": "dns-local" } }));

    json!({
        "log": { "level": "error", "timestamp": false },
        "dns": {
            "servers": [{ "type": "udp", "server": "223.5.5.5", "tag": "dns-local" }],
            "final": "dns-local"
        },
        "inbounds": [{
            "type": "mixed",
            "tag": "probe-in",
            "listen": "127.0.0.1",
            "listen_port": mixed_port
        }],
        "outbounds": outbounds,
        "route": {
            "final": TAG_DIRECT,
            "default_domain_resolver": { "server": "dns-local" }
        },
        "experimental": {
            "clash_api": {
                "external_controller": format!("127.0.0.1:{}", clash_port),
                "default_mode": "rule"
            }
        }
    })
}
