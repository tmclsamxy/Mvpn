//! Windows 系统代理控制（HKCU + InternetSetOption，无需管理员）
use anyhow::Result;
use std::ffi::c_void;
use winreg::enums::*;
use winreg::RegKey;

const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
const OPT_REFRESH: u32 = 37;
const OPT_SETTINGS_CHANGED: u32 = 39;

#[link(name = "wininet")]
unsafe extern "system" {
    fn InternetSetOptionW(
        hinternet: *mut c_void,
        dwoption: u32,
        lpbuffer: *mut c_void,
        dwbufferlength: u32,
    ) -> i32;
}

fn notify() {
    unsafe {
        InternetSetOptionW(std::ptr::null_mut(), OPT_SETTINGS_CHANGED, std::ptr::null_mut(), 0);
        InternetSetOptionW(std::ptr::null_mut(), OPT_REFRESH, std::ptr::null_mut(), 0);
    }
}

fn key() -> Result<RegKey> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (k, _) = hkcu.create_subkey(INTERNET_SETTINGS)?;
    Ok(k)
}

/// 开启系统代理
pub fn enable(server: &str, bypass: &str) -> Result<()> {
    let k = key()?;
    k.set_value("ProxyEnable", &1u32)?;
    k.set_value("ProxyServer", &server.to_string())?;
    if !bypass.is_empty() {
        k.set_value("ProxyOverride", &bypass.to_string())?;
    }
    k.set_value("ProxyHttp1.1", &0u32).ok();
    notify();
    Ok(())
}

/// 关闭系统代理
pub fn disable() -> Result<()> {
    let k = key()?;
    k.set_value("ProxyEnable", &0u32)?;
    notify();
    Ok(())
}

/// 读取当前系统代理，返回 Some("host:port") 表示已启用
pub fn current() -> Option<String> {
    let k = key().ok()?;
    let enable: u32 = k.get_value("ProxyEnable").unwrap_or(0);
    if enable == 0 {
        return None;
    }
    let server: String = k.get_value("ProxyServer").unwrap_or_default();
    if server.is_empty() {
        None
    } else {
        Some(server)
    }
}
