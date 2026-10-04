//! 数据模型 —— 与前端 JS 一一对应（统一 camelCase）
use serde::{Deserialize, Serialize};

fn t() -> bool {
    true
}

/// 代理节点
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Node {
    pub id: String,
    pub name: String,
    /// vless | vmess | trojan | shadowsocks | hysteria2 | tuic
    pub protocol: String,
    pub server: String,
    pub port: u16,

    // 凭据
    pub uuid: String,
    pub password: String,
    /// shadowsocks 加密方式
    pub method: String,
    pub flow: String,
    pub alter_id: u32,
    pub security: String,

    // TLS
    pub tls: bool,
    pub sni: String,
    pub alpn: String,
    pub fingerprint: String,
    pub allow_insecure: bool,
    pub reality: bool,
    pub reality_public_key: String,
    pub reality_short_id: String,

    // 传输层
    /// tcp | ws | grpc | http | quic（空 = 默认 tcp）
    pub transport: String,
    pub ws_path: String,
    pub ws_host: String,
    pub grpc_service: String,
    pub early_data: bool,

    // hysteria2 / tuic
    pub obfs: String,
    pub obfs_password: String,
    pub up_mbps: u32,
    pub down_mbps: u32,
    pub congestion_control: String,
    pub udp_relay_mode: String,

    // 分组 / 备注
    pub group: String,
    #[serde(default = "t")]
    pub enabled: bool,
}

/// 注意：必须手写 Default —— 派生版会把 enabled 置为 false，
/// 导致用 `..Default::default()` 构造的节点在生成配置时被静默过滤掉。
impl Default for Node {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            protocol: String::new(),
            server: String::new(),
            port: 0,
            uuid: String::new(),
            password: String::new(),
            method: String::new(),
            flow: String::new(),
            alter_id: 0,
            security: String::new(),
            tls: false,
            sni: String::new(),
            alpn: String::new(),
            fingerprint: String::new(),
            allow_insecure: false,
            reality: false,
            reality_public_key: String::new(),
            reality_short_id: String::new(),
            transport: String::new(),
            ws_path: String::new(),
            ws_host: String::new(),
            grpc_service: String::new(),
            early_data: false,
            obfs: String::new(),
            obfs_password: String::new(),
            up_mbps: 0,
            down_mbps: 0,
            congestion_control: String::new(),
            udp_relay_mode: String::new(),
            group: String::new(),
            enabled: true,
        }
    }
}

/// 自定义分流规则
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RouteRule {
    pub id: String,
    pub enabled: bool,
    pub remark: String,
    /// route | reject | hijack-dns | sniff
    pub action: String,
    /// proxy | direct | block | <节点名>
    pub outbound: String,
    /// 多行：精确域名
    pub domain: String,
    /// 多行：域名后缀
    pub domain_suffix: String,
    /// 多行：域名关键字
    pub domain_keyword: String,
    /// 多行：IP CIDR
    pub ip_cidr: String,
    /// 多行：端口
    pub port: String,
    /// 多行：源 IP CIDR
    pub source_ip_cidr: String,
    /// 多行：进程名
    pub process_name: String,
    /// 逗号分隔：规则集标签
    pub rule_set: String,
    /// 逗号分隔：嗅探协议 tls/http/quic
    pub protocol: String,
    /// 入站标签
    pub inbound: String,
    /// rule / global / direct
    pub clash_mode: String,
    pub invert: bool,
}

impl Default for RouteRule {
    fn default() -> Self {
        Self {
            id: String::new(),
            enabled: true,
            remark: String::new(),
            action: "route".into(),
            outbound: "proxy".into(),
            domain: String::new(),
            domain_suffix: String::new(),
            domain_keyword: String::new(),
            ip_cidr: String::new(),
            port: String::new(),
            source_ip_cidr: String::new(),
            process_name: String::new(),
            rule_set: String::new(),
            protocol: String::new(),
            inbound: String::new(),
            clash_mode: String::new(),
            invert: false,
        }
    }
}

/// 远程 / 本地规则集
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RuleSetDef {
    pub tag: String,
    /// remote | local
    pub kind: String,
    /// binary | source
    pub format: String,
    pub url: String,
    pub path: String,
}

impl Default for RuleSetDef {
    fn default() -> Self {
        Self {
            tag: String::new(),
            kind: "remote".into(),
            format: "binary".into(),
            url: String::new(),
            path: String::new(),
        }
    }
}

/// 订阅
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub url: String,
    /// 该订阅导入的节点 id 列表
    pub node_ids: Vec<String>,
    pub last_update: String,
}

impl Default for Subscription {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            url: String::new(),
            node_ids: Vec::new(),
            last_update: String::new(),
        }
    }
}

/// 全局设置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Setting {
    // ---- 混合入站（本地 / 局域网）----
    pub mixed_enabled: bool,
    /// 127.0.0.1 = 仅本机；0.0.0.0 = 局域网
    pub mixed_allow_lan: bool,
    pub mixed_port: u16,
    pub mixed_auth: bool,
    pub mixed_username: String,
    pub mixed_password: String,

    // ---- TUN ----
    pub tun_enabled: bool,
    /// mixed | gvisor | system
    pub tun_stack: String,
    pub tun_mtu: u32,
    pub tun_auto_route: bool,
    pub tun_strict_route: bool,

    // ---- 代理模式 ----
    /// rule | global | direct
    pub mode: String,

    // ---- 系统代理 ----
    pub sys_proxy: bool,
    pub sys_proxy_bypass: String,

    // ---- DNS ----
    pub dns_local: String,
    pub dns_remote: String,
    pub dns_strategy: String,
    pub dns_cn_direct: bool,

    // ---- 分流 ----
    /// 内置规则集：中国大陆直连
    pub cn_direct: bool,
    pub final_outbound: String,
    /// 规则集下载源：jsdelivr | github
    pub ruleset_source: String,

    // ---- 内核 ----
    pub core_path: String,
    pub clash_api_port: u16,
    pub log_level: String,
    pub auto_detect_interface: bool,
    pub allow_lan_check: bool,

    // ---- 启动行为 ----
    pub autostart: bool,
    pub start_minimized: bool,
    pub auto_connect: bool,

    // ---- 其它 ----
    pub language: String,
    pub theme: String,
    pub update_channel: String,
    /// 客户端自身更新：版本清单地址（JSON），留空表示不检查
    pub update_manifest_url: String,
    /// 启动时自动检查客户端更新
    pub auto_check_update: bool,
}

impl Default for Setting {
    fn default() -> Self {
        Self {
            mixed_enabled: true,
            mixed_allow_lan: false,
            mixed_port: 2080,
            mixed_auth: false,
            mixed_username: "mvpn".into(),
            mixed_password: "mvpn".into(),

            tun_enabled: false,
            tun_stack: "mixed".into(),
            tun_mtu: 9000,
            tun_auto_route: true,
            tun_strict_route: true,

            mode: "rule".into(),

            sys_proxy: false,
            sys_proxy_bypass: "localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*;<local>".into(),

            dns_local: "223.5.5.5".into(),
            dns_remote: "https://1.1.1.1/dns-query".into(),
            dns_strategy: "prefer_ipv4".into(),
            dns_cn_direct: true,

            cn_direct: true,
            final_outbound: "proxy".into(),
            ruleset_source: "jsdelivr".into(),

            core_path: String::new(),
            clash_api_port: 9900,
            log_level: "info".into(),
            auto_detect_interface: true,
            allow_lan_check: false,

            autostart: false,
            start_minimized: true,
            auto_connect: true,

            language: "zh-CN".into(),
            theme: "light".into(),
            update_channel: "stable".into(),
            update_manifest_url: String::new(),
            auto_check_update: false,
        }
    }
}

/// 完整应用状态（持久化）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppState {
    pub setting: Setting,
    pub nodes: Vec<Node>,
    pub rules: Vec<RouteRule>,
    pub rule_sets: Vec<RuleSetDef>,
    pub subscriptions: Vec<Subscription>,
    /// 当前选中节点 id（空 = 自动选择）
    pub active_node: String,
}
