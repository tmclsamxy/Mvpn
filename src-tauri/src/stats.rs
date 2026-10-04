//! 流量统计：今日/历史/分节点，用 Clash API 的累计值做差分累计
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Bytes {
    pub up: u64,
    pub down: u64,
}

impl Bytes {
    pub fn add(&mut self, o: &Bytes) {
        self.up = self.up.saturating_add(o.up);
        self.down = self.down.saturating_add(o.down);
    }
    pub fn total(&self) -> u64 {
        self.up.saturating_add(self.down)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DayStat {
    pub date: String,
    pub up: u64,
    pub down: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrafficStats {
    /// 按日期升序，最多保留 90 天
    pub days: Vec<DayStat>,
    /// node_id -> 累计流量（历史总用量）
    pub nodes: HashMap<String, Bytes>,
    /// 保留若干天的秒级采样，供前端画曲线（当天）
    pub series: Vec<Sample>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Sample {
    pub t: u64,
    pub up: u64,
    pub down: u64,
}

pub const MAX_DAYS: usize = 90;
pub const MAX_SERIES: usize = 180;

impl TrafficStats {
    /// 累计一轮采样。`delta` 是本轮新增流量（由监控线程做差分得到）。
    pub fn accumulate(&mut self, date: &str, delta: Bytes, per_node: &HashMap<String, Bytes>) {
        if delta.up > 0 || delta.down > 0 {
            if self.days.last().map(|d| d.date.as_str()) != Some(date) {
                self.days.push(DayStat {
                    date: date.to_string(),
                    up: 0,
                    down: 0,
                });
                if self.days.len() > MAX_DAYS {
                    self.days.drain(0..self.days.len() - MAX_DAYS);
                }
            }
            if let Some(d) = self.days.last_mut() {
                d.up = d.up.saturating_add(delta.up);
                d.down = d.down.saturating_add(delta.down);
            }
        }

        for (id, b) in per_node {
            if b.up > 0 || b.down > 0 {
                let e = self.nodes.entry(id.clone()).or_default();
                e.add(b);
            }
        }
    }

    /// 记录一个速率采样点（当天曲线）
    pub fn push_sample(&mut self, now: u64, up: u64, down: u64) {
        self.series.push(Sample { t: now, up, down });
        if self.series.len() > MAX_SERIES {
            let drop = self.series.len() - MAX_SERIES;
            self.series.drain(0..drop);
        }
    }

    /// 跨天时清空当天曲线
    pub fn rollover_series(&mut self, date: &str) {
        if let Some(d) = self.days.last() {
            if d.date != date {
                self.series.clear();
            }
        }
    }

    pub fn today(&self, date: &str) -> Bytes {
        self.days
            .iter()
            .find(|d| d.date == date)
            .map(|d| Bytes {
                up: d.up,
                down: d.down,
            })
            .unwrap_or_default()
    }

    pub fn lifetime(&self) -> Bytes {
        let mut t = Bytes::default();
        for d in &self.days {
            t.up = t.up.saturating_add(d.up);
            t.down = t.down.saturating_add(d.down);
        }
        t
    }
}

pub fn load(path: &Path) -> TrafficStats {
    match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => serde_json::from_str(&s).unwrap_or_default(),
        _ => TrafficStats::default(),
    }
}

pub fn save(path: &Path, st: &TrafficStats) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(st).unwrap_or_default())?;
    std::fs::rename(&tmp, path)
}
