//! 内核进程管理、网络请求与更新
use crate::model::Setting;
use crate::store::Paths;
use anyhow::{anyhow, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const GITHUB_API: &str = "https://api.github.com/repos/SagerNet/sing-box/releases/latest";
const WINTUN_URL: &str = "https://www.wintun.net/builds/wintun-0.14.1.zip";

/// 构造 HTTP 客户端（可指定代理，例如本地 mixed 端口）
pub fn client(proxy: Option<&str>) -> Result<reqwest::blocking::Client> {
    let mut b = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .user_agent("Mvpn/0.1 (+https://github.com/2dust/v2rayN)");
    if let Some(p) = proxy {
        if !p.trim().is_empty() {
            b = b.proxy(reqwest::Proxy::all(p.trim())?);
        }
    }
    Ok(b.build()?)
}

/// 若内核在运行，走本地代理拉取（便于访问 GitHub）
pub fn update_proxy_url(setting: &Setting, running: bool) -> Option<String> {
    if running && setting.mixed_enabled {
        Some(format!("http://127.0.0.1:{}", setting.mixed_port))
    } else {
        None
    }
}

pub fn find_core(paths: &Paths, setting: &Setting) -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if !setting.core_path.trim().is_empty() {
        let p = PathBuf::from(setting.core_path.trim());
        cands.push(if p.is_dir() { p.join("sing-box.exe") } else { p });
    }
    cands.push(paths.core.join("sing-box.exe"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("sing-box.exe"));
            cands.push(dir.join("core").join("sing-box.exe"));
        }
    }
    cands.into_iter().find(|p| p.is_file())
}

pub fn core_version(core: &Path) -> String {
    let mut cmd = Command::new(core);
    cmd.arg("version");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

/// 校验配置合法性
pub fn check_config(core: &Path, config: &Path) -> std::result::Result<(), String> {
    let mut cmd = Command::new(core);
    cmd.arg("check").arg("-c").arg(config);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    match cmd.output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stderr).to_string();
            s.push_str(&String::from_utf8_lossy(&o.stdout));
            Err(s.trim().to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn spawn_core(core: &Path, config: &Path, workdir: &Path) -> Result<Child> {
    let mut cmd = Command::new(core);
    cmd.arg("run")
        .arg("-c")
        .arg(config)
        .arg("-D")
        .arg(workdir)
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    Ok(cmd.spawn()?)
}

pub fn kill_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// 是否管理员（TUN 模式需要）
pub fn is_admin() -> bool {
    let mut cmd = Command::new("net");
    cmd.arg("session").stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// 取一个空闲端口
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
        .unwrap_or(20800)
}

/// 查询 sing-box 最新版本与下载地址
pub fn latest_release(proxy: Option<&str>) -> Result<(String, String)> {
    let c = client(proxy)?;
    let v: serde_json::Value = c.get(GITHUB_API).send()?.error_for_status()?.json()?;
    let tag = v["tag_name"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches('v')
        .to_string();
    let asset = v["assets"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|a| {
            a["name"]
                .as_str()
                .map(|n| n.contains("windows-amd64") && n.ends_with(".zip"))
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow!("未找到 windows-amd64 版内核资源"))?;
    let url = asset["browser_download_url"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if url.is_empty() {
        return Err(anyhow!("内核下载地址为空"));
    }
    Ok((tag, url))
}

pub fn download_bytes(url: &str, proxy: Option<&str>) -> Result<Vec<u8>> {
    let c = client(proxy)?;
    let resp = c.get(url).send()?.error_for_status()?;
    let mut buf = Vec::new();
    resp.take(200 * 1024 * 1024).read_to_end(&mut buf)?;
    Ok(buf)
}

/// 从 zip 中提取 sing-box.exe / libcronet.dll / wintun.dll
pub fn install_core_zip(bytes: &[u8], dest_dir: &Path) -> Result<PathBuf> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    std::fs::create_dir_all(dest_dir)?;
    let mut exe: Option<PathBuf> = None;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let name = f.name().to_string();
        let base = name.rsplit('/').next().unwrap_or("").to_string();
        if matches!(base.as_str(), "sing-box.exe" | "libcronet.dll") {
            let out = dest_dir.join(&base);
            let mut w = std::fs::File::create(&out)?;
            std::io::copy(&mut f, &mut w)?;
            if base == "sing-box.exe" {
                exe = Some(out);
            }
        }
    }
    exe.ok_or_else(|| anyhow!("压缩包内未找到 sing-box.exe"))
}

/// 确保 wintun.dll 存在（TUN 模式在 Windows 上需要）
pub fn ensure_wintun(dest_dir: &Path, proxy: Option<&str>) -> Result<PathBuf> {
    let target = dest_dir.join("wintun.dll");
    if target.is_file() {
        return Ok(target);
    }
    let bytes = download_bytes(WINTUN_URL, proxy)
        .map_err(|e| anyhow!("下载 wintun 失败（TUN 模式需要，可手动放置 wintun.dll）: {e}"))?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let name = f.name().to_string().to_ascii_lowercase();
        if name.ends_with("bin/amd64/wintun.dll") || name == "wintun.dll" {
            let mut w = std::fs::File::create(&target)?;
            std::io::copy(&mut f, &mut w)?;
            return Ok(target);
        }
    }
    Err(anyhow!("wintun 压缩包中未找到 amd64 版 wintun.dll"))
}
