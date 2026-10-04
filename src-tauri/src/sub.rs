//! 订阅解析：支持 vmess / vless / trojan / ss / hysteria2 / tuic 分享链接
use crate::model::Node;
use base64::Engine;
use url::Url;

fn b64_decode(s: &str) -> Option<String> {
    let cleaned: String = s.trim().chars().filter(|c| !c.is_whitespace()).collect();
    let engines = [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ];
    for e in engines {
        if let Ok(v) = e.decode(cleaned.as_bytes()) {
            if let Ok(s) = String::from_utf8(v) {
                return Some(s);
            }
        }
    }
    // 补 padding 再试
    let mut padded = cleaned.clone();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .ok()
        .and_then(|v| String::from_utf8(v).ok())
}

fn q(url: &Url, key: &str) -> String {
    url.query_pairs()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.to_string())
        .unwrap_or_default()
}

fn truthy(s: &str) -> bool {
    matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn name_of(url: &Url, fallback: &str) -> String {
    let frag = url.fragment().unwrap_or("");
    let n = urlencoding::decode(frag)
        .map(|c| c.to_string())
        .unwrap_or_else(|_| frag.to_string());
    if n.trim().is_empty() {
        fallback.to_string()
    } else {
        n.trim().to_string()
    }
}

fn host_of(url: &Url) -> String {
    url.host_str().unwrap_or("").to_string()
}

/// 解析单条分享链接
pub fn parse_link(line: &str) -> Option<Node> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
        return None;
    }
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("vmess://") {
        return parse_vmess(line);
    }
    if lower.starts_with("ss://") {
        return parse_ss(line);
    }
    if lower.starts_with("vless://") {
        return parse_vless(line);
    }
    if lower.starts_with("trojan://") {
        return parse_trojan(line);
    }
    if lower.starts_with("hysteria2://") || lower.starts_with("hy2://") {
        return parse_hy2(line);
    }
    if lower.starts_with("tuic://") {
        return parse_tuic(line);
    }
    None
}

/// 解析订阅文本（自动识别 Base64）
pub fn parse_subscription(text: &str) -> Vec<Node> {
    let body = if text.contains("://") {
        text.to_string()
    } else {
        b64_decode(text).unwrap_or_else(|| text.to_string())
    };
    body.lines().filter_map(parse_link).collect()
}

fn set_tls_common(n: &mut Node, url: &Url) {
    let security = q(url, "security");
    let sni = q(url, "sni");
    let fp = q(url, "fp");
    let alpn = q(url, "alpn");
    let insecure = q(url, "allowInsecure");
    if !sni.is_empty() {
        n.sni = sni;
    }
    if !fp.is_empty() {
        n.fingerprint = fp;
    }
    if !alpn.is_empty() {
        n.alpn = alpn;
    }
    if truthy(&insecure) || q(url, "insecure") == "1" {
        n.allow_insecure = true;
    }
    match security.as_str() {
        "reality" => {
            n.reality = true;
            n.tls = true;
        }
        "tls" => n.tls = true,
        _ => {}
    }
    if n.sni.is_empty() && n.tls {
        n.sni = host_of(url);
    }
}

fn set_transport_common(n: &mut Node, url: &Url) {
    let t = q(url, "type");
    n.transport = match t.as_str() {
        "ws" | "websocket" => "ws".into(),
        "grpc" => "grpc".into(),
        "http" | "h2" => "http".into(),
        "quic" => "quic".into(),
        _ => "tcp".into(),
    };
    n.ws_path = q(url, "path");
    n.ws_host = q(url, "host");
    n.grpc_service = {
        let s = q(url, "serviceName");
        if s.is_empty() {
            q(url, "service")
        } else {
            s
        }
    };
}

fn parse_vless(line: &str) -> Option<Node> {
    let url = Url::parse(line).ok()?;
    let mut n = Node {
        protocol: "vless".into(),
        id: String::new(),
        name: name_of(&url, &format!("{}:{}", host_of(&url), url.port().unwrap_or(443))),
        server: host_of(&url),
        port: url.port().unwrap_or(443),
        uuid: url.username().to_string(),
        flow: q(&url, "flow"),
        ..Default::default()
    };
    set_tls_common(&mut n, &url);
    set_transport_common(&mut n, &url);
    n.reality_public_key = q(&url, "pbk");
    n.reality_short_id = q(&url, "sid");
    if n.server.is_empty() || n.uuid.is_empty() {
        return None;
    }
    Some(n)
}

fn parse_trojan(line: &str) -> Option<Node> {
    let url = Url::parse(line).ok()?;
    let mut n = Node {
        protocol: "trojan".into(),
        name: name_of(&url, &format!("{}:{}", host_of(&url), url.port().unwrap_or(443))),
        server: host_of(&url),
        port: url.port().unwrap_or(443),
        password: urlencoding::decode(url.username())
            .map(|c| c.to_string())
            .unwrap_or_else(|_| url.username().to_string()),
        tls: true,
        ..Default::default()
    };
    set_tls_common(&mut n, &url);
    set_transport_common(&mut n, &url);
    n.tls = true;
    if n.server.is_empty() || n.password.is_empty() {
        return None;
    }
    Some(n)
}

fn parse_hy2(line: &str) -> Option<Node> {
    let url = Url::parse(line).ok()?;
    let auth = urlencoding::decode(url.username())
        .map(|c| c.to_string())
        .unwrap_or_else(|_| url.username().to_string());
    // 支持 user:pass 形式，取最后一段为密码
    let password = match url.password() {
        Some(p) => p.to_string(),
        None => auth,
    };
    let mut n = Node {
        protocol: "hysteria2".into(),
        name: name_of(&url, &format!("{}:{}", host_of(&url), url.port().unwrap_or(443))),
        server: host_of(&url),
        port: url.port().unwrap_or(443),
        password,
        tls: true,
        sni: q(&url, "sni"),
        obfs: q(&url, "obfs"),
        obfs_password: {
            let v = q(&url, "obfs-password");
            if v.is_empty() {
                q(&url, "obfs_password")
            } else {
                v
            }
        },
        ..Default::default()
    };
    if n.sni.is_empty() {
        n.sni = host_of(&url);
    }
    if q(&url, "insecure") == "1" || truthy(&q(&url, "allowInsecure")) {
        n.allow_insecure = true;
    }
    if n.server.is_empty() {
        return None;
    }
    Some(n)
}

fn parse_tuic(line: &str) -> Option<Node> {
    let url = Url::parse(line).ok()?;
    let mut n = Node {
        protocol: "tuic".into(),
        name: name_of(&url, &format!("{}:{}", host_of(&url), url.port().unwrap_or(443))),
        server: host_of(&url),
        port: url.port().unwrap_or(443),
        uuid: url.username().to_string(),
        password: url.password().unwrap_or("").to_string(),
        tls: true,
        sni: q(&url, "sni"),
        congestion_control: q(&url, "congestion_control"),
        udp_relay_mode: q(&url, "udp_relay_mode"),
        ..Default::default()
    };
    if n.sni.is_empty() {
        n.sni = q(&url, "peer");
    }
    if n.sni.is_empty() {
        n.sni = host_of(&url);
    }
    if q(&url, "allow_insecure") == "1" || truthy(&q(&url, "insecure")) {
        n.allow_insecure = true;
    }
    if n.server.is_empty() {
        return None;
    }
    Some(n)
}

fn parse_ss(line: &str) -> Option<Node> {
    // 形态1: ss://base64(method:password@host:port)#name
    let after = line.trim_start_matches("ss://");
    let (body, frag) = match after.split_once('#') {
        Some((b, f)) => (b, urlencoding::decode(f).map(|c| c.to_string()).unwrap_or_default()),
        None => (after, String::new()),
    };

    if !body.contains('@') {
        // 整体 base64
        let decoded = b64_decode(body.split('?').next().unwrap_or(body))?;
        let (method, rest) = decoded.split_once(':')?;
        let (cred, hostport) = match rest.rsplit_once('@') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => return None,
        };
        let (host, port) = hostport.rsplit_once(':')?;
        let mut n = Node {
            protocol: "shadowsocks".into(),
            name: if frag.is_empty() { host.to_string() } else { frag },
            server: host.to_string(),
            port: port.parse().ok()?,
            method: method.to_string(),
            password: cred,
            ..Default::default()
        };
        // SIP003 插件
        apply_ss_plugin(&mut n, body);
        return Some(n);
    }

    // 形态2/3: ss://userinfo@host:port?plugin=...
    let url = Url::parse(&format!("ss://{}", body)).ok()?;
    let mut method = urlencoding::decode(url.username())
        .map(|c| c.to_string())
        .unwrap_or_else(|_| url.username().to_string());
    let mut password = urlencoding::decode(url.password().unwrap_or(""))
        .map(|c| c.to_string())
        .unwrap_or_else(|_| url.password().unwrap_or("").to_string());

    // userinfo 是 base64(method:password)
    if password.is_empty() {
        if let Some(dec) = b64_decode(&method) {
            if let Some((m, p)) = dec.split_once(':') {
                method = m.to_string();
                password = p.to_string();
            }
        }
    }

    let mut n = Node {
        protocol: "shadowsocks".into(),
        name: if frag.is_empty() {
            format!("{}:{}", host_of(&url), url.port().unwrap_or(0))
        } else {
            frag
        },
        server: host_of(&url),
        port: url.port().unwrap_or(0),
        method,
        password,
        ..Default::default()
    };
    apply_ss_plugin(&mut n, body);
    if n.server.is_empty() || n.port == 0 {
        return None;
    }
    Some(n)
}

fn apply_ss_plugin(n: &mut Node, body: &str) {
    // 解析 ?plugin=obfs-local%3Bobfs%3Dhttp... —— 目前仅记录到 transport 备注用途
    if let Some(idx) = body.find("plugin=") {
        let p = &body[idx + 7..];
        let p = p.split('&').next().unwrap_or("");
        if let Ok(dec) = urlencoding::decode(p) {
            let d = dec.to_string();
            if d.starts_with("v2ray-plugin") {
                n.transport = "ws".into();
            }
        }
    }
}

fn parse_vmess(line: &str) -> Option<Node> {
    let raw = line.trim_start_matches("vmess://").trim();
    let json_str = b64_decode(raw)?;
    let v: serde_json::Value = serde_json::from_str(&json_str).ok()?;
    let s = |k: &str| -> String {
        v.get(k)
            .map(|x| match x {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default()
    };
    let net = s("net");
    let tls = s("tls");
    let host = s("host");
    let n = Node {
        protocol: "vmess".into(),
        name: {
            let ps = s("ps");
            if ps.is_empty() { s("add") } else { ps }
        },
        server: s("add"),
        port: s("port").parse().unwrap_or(0),
        uuid: s("id"),
        alter_id: s("aid").parse().unwrap_or(0),
        security: {
            let sc = s("scy");
            if sc.is_empty() { "auto".into() } else { sc }
        },
        transport: match net.as_str() {
            "ws" => "ws".into(),
            "grpc" => "grpc".into(),
            "h2" | "http" => "http".into(),
            "quic" => "quic".into(),
            _ => "tcp".into(),
        },
        ws_path: s("path"),
        ws_host: host.clone(),
        grpc_service: s("path"),
        tls: tls == "tls",
        sni: {
            let sni = s("sni");
            if sni.is_empty() { host } else { sni }
        },
        alpn: s("alpn"),
        fingerprint: s("fp"),
        ..Default::default()
    };
    if n.server.is_empty() || n.port == 0 || n.uuid.is_empty() {
        return None;
    }
    Some(n)
}
