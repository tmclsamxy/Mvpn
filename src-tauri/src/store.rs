//! 路径、持久化与日志
use crate::model::*;
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub runtime: PathBuf,
    pub core: PathBuf,
    pub state_file: PathBuf,
    pub log_file: PathBuf,
}

impl Paths {
    pub fn new(data: PathBuf) -> Self {
        let runtime = data.join("runtime");
        let core = data.join("core");
        let logs = data.join("logs");
        for d in [&data, &runtime, &core, &logs] {
            let _ = fs::create_dir_all(d);
        }
        Self {
            state_file: data.join("state.json"),
            log_file: logs.join("mvpn.log"),
            data,
            runtime,
            core,
        }
    }
    pub fn config_file(&self) -> PathBuf {
        self.runtime.join("config.json")
    }
    pub fn probe_file(&self) -> PathBuf {
        self.runtime.join("probe.json")
    }
    pub fn stats_file(&self) -> PathBuf {
        self.data.join("stats.json")
    }
}

/// 默认状态（含示例规则与规则集）
pub fn default_state() -> AppState {
    let mut st = AppState {
        setting: Setting::default(),
        ..Default::default()
    };
    st.rules = vec![
        RouteRule {
            id: "r-lan".into(),
            enabled: true,
            remark: "局域网 / 私有地址直连".into(),
            action: "route".into(),
            outbound: "direct".into(),
            ip_cidr: "10.0.0.0/8\n172.16.0.0/12\n192.168.0.0/16\n127.0.0.0/8".into(),
            ..Default::default()
        },
        RouteRule {
            id: "r-ai".into(),
            enabled: false,
            remark: "AI 服务走代理".into(),
            action: "route".into(),
            outbound: "proxy".into(),
            domain_suffix: "openai.com\nchatgpt.com\nanthropic.com\nclaude.ai\ngemini.google.com".into(),
            ..Default::default()
        },
        RouteRule {
            id: "r-ads".into(),
            enabled: false,
            remark: "拦截常见广告域名（geosite-ads 规则集）".into(),
            action: "reject".into(),
            rule_set: "geosite-ads".into(),
            ..Default::default()
        },
    ];
    st.rule_sets = vec![RuleSetDef {
        tag: "geosite-ads".into(),
        kind: "remote".into(),
        format: "binary".into(),
        url: crate::singbox::ruleset_urls("jsdelivr").2,
        path: String::new(),
    }];
    st.setting.clash_api_port = 9900;
    st
}

pub fn load_state(p: &Paths) -> AppState {
    match fs::read_to_string(&p.state_file) {
        Ok(s) if !s.trim().is_empty() => match serde_json::from_str::<AppState>(&s) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[mvpn] state.json 解析失败: {e}, 使用默认配置");
                default_state()
            }
        },
        _ => default_state(),
    }
}

pub fn save_state(p: &Paths, st: &AppState) -> anyhow::Result<()> {
    let s = serde_json::to_string_pretty(st)?;
    let tmp = p.state_file.with_extension("json.tmp");
    fs::write(&tmp, s)?;
    fs::rename(&tmp, &p.state_file)?;
    Ok(())
}

/// 获取当前本地时间字符串 HH:MM:SS
pub fn now_ts() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 东八区
    let local = secs + 8 * 3600;
    let h = (local / 3600) % 24;
    let m = (local / 60) % 60;
    let s = local % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

pub fn now_datetime() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let local = secs + 8 * 3600;
    let days = local / 86400;
    let (y, m, d) = civil_from_days(days as i64);
    let hh = (local / 3600) % 24;
    let mm = (local / 60) % 60;
    let ss = local % 60;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y, m, d, hh, mm, ss
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // Howard Hinnant 算法
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 今天的日期字符串 YYYY-MM-DD
pub fn today() -> String {
    now_datetime()
        .split(' ')
        .next()
        .unwrap_or("")
        .to_string()
}

/// 追加日志到文件
pub fn append_log(p: &Paths, line: &str) {
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p.log_file)
    {
        let _ = writeln!(f, "{}", line);
    }
}

/// 日志环形缓冲
pub type LogBuf = VecDeque<String>;

pub fn push_log(buf: &mut LogBuf, line: String, app: Option<&tauri::AppHandle>) {
    buf.push_back(line.clone());
    while buf.len() > 800 {
        buf.pop_front();
    }
    if let Some(a) = app {
        use tauri::Emitter;
        let _ = a.emit("core-log", line);
    }
}
