# Mvpn

轻量级 Windows 代理客户端 —— 基于 [sing-box](https://github.com/SagerNet/sing-box) 内核，界面参考 [v2rayN](https://github.com/2dust/v2rayN)，支持自定义分流、本地/局域网混合代理与 TUN 模式。

## 特性

- **多协议节点**：VLESS（含 Reality / XTLS-Vision）、VMess、Trojan、Shadowsocks（含 2022）、Hysteria2、TUIC
- **订阅与导入**：支持 `vmess:// vless:// trojan:// ss:// hysteria2:// tuic://` 分享链接与 Base64 订阅
- **入站模式**
  - **Mixed**：HTTP + SOCKS5 同端口，可切 `127.0.0.1`（仅本机）/ `0.0.0.0`（局域网共享），支持用户名密码认证
  - **TUN**：全局接管流量（需管理员权限），可选 `mixed` / `gvisor` / `system` 协议栈
- **自定义分流**：域名 / 域名后缀 / 关键字 / IP CIDR / 端口 / 进程名 / 规则集 → 直连 / 代理 / 拦截；规则可排序、可禁用
- **内置规则集**：中国大陆域名（geosite-cn）+ IP（geoip-cn）直连，可扩展自定义远程 `.srs` 规则集
- **三种模式**：规则 / 全局 / 直连，借助 Clash API 运行时切换，无需重启内核
- **系统代理**：一键开关（写注册表 `HKCU`，无需管理员），退出自动清理
- **节点测速**：临时内核 + Clash API 延迟测试，结果实时回显
- **内核更新**：检测 sing-box 最新版本并一键下载替换
- **流量统计**
  - 右上角实时上下行速率
  - 今日 / 历史累计（保留 90 天）+ 近 7 天柱状图
  - **分节点累计消耗**与占比（按连接差分归因，连接关闭不丢量）
- **实时连接列表**：正在经过代理的目标域名 / IP、端口、进程、链路（走了哪个节点或直连）
- **客户端自身更新**：检查版本清单 → 下载 → 替换运行中的 exe → 自动重启
- **托盘常驻**：最小化到托盘，开机自启，启动时自动连接
- **轻量**：界面为原生 HTML/CSS/JS（无前端框架、无打包步骤），客户端本体约 7.5MB、空载内存约 32MB

## 目录结构

```
Mvpn/
├── ui/                    # 前端（原生 HTML/CSS/JS，由 Tauri 直接加载）
│   ├── index.html
│   ├── style.css
│   └── app.js
├── src-tauri/             # Rust 后端
│   ├── src/
│   │   ├── main.rs        # 入口：托盘、窗口、自检模式
│   │   ├── model.rs       # 数据模型（节点 / 规则 / 设置）
│   │   ├── store.rs       # 路径、持久化、日志
│   │   ├── singbox.rs     # 状态 → sing-box JSON 配置生成
│   │   ├── core.rs        # 内核进程、下载、更新
│   │   ├── sysproxy.rs    # Windows 系统代理
│   │   ├── sub.rs         # 分享链接 / 订阅解析
│   │   └── commands.rs    # Tauri IPC 命令
│   ├── icons/
│   └── tauri.conf.json
├── scripts/
│   ├── setup-toolchain.sh # 一键安装便携构建工具链（免管理员）
│   ├── build.sh           # 构建
│   └── package.sh         # 打包（含内置内核）
└── .toolchain/            # 便携 Rust + MinGW（不污染系统，可随时删除）
```

## 构建

需要 **Rust（GNU 工具链）+ MinGW-w64**。项目自带一键脚本，全部安装在 `.toolchain/` 内，**不需要管理员权限，不需要 Visual Studio**：

```bash
# 国内网络可先设置代理
export MVPN_PROXY=http://127.0.0.1:10808

./scripts/setup-toolchain.sh   # 安装工具链（仅首次）
./scripts/build.sh             # 构建，产物: src-tauri/target/release/mvpn.exe
./scripts/package.sh           # 打包到 dist/Mvpn-0.1.0/（含内核）
```

> 说明：Windows 上 Tauri 默认走 MSVC 工具链，需要体积巨大的 VS Build Tools；
> 本项目改用 **Rust GNU + 便携 MinGW-w64**，无需管理员、无需 Visual Studio，适合受限环境。

### 自检与冒烟测试（无需 GUI）

```bash
./scripts/selftest.sh
```

包含三步：
1. **配置生成校验** —— 造一份含 6 种协议（VLESS-Reality / VMess-gRPC / Trojan-WS / SS-2022 / Hysteria2 / TUIC）+ TUN + 局域网 mixed + 自定义规则的配置，交给内核 `check` 校验
2. **分享链接解析** —— 覆盖 `vless:// vmess:// trojan:// ss://(两种形态) hysteria2:// tuic://` 以及整段 Base64 订阅
3. **全链路冒烟** —— 生成配置 → 拉起内核 → 等混合代理端口就绪 → 用 curl 真实发一次代理请求

其它诊断开关（都是无 GUI 的）：

```bash
MVP_CORE=/path/to/sing-box.exe ./src-tauri/target/release/mvpn.exe --monitortest   # 流量监控链路
./src-tauri/target/release/mvpn.exe --proxtest-on   # 系统代理写入机制（读完即回读校验）
./src-tauri/target/release/mvpn.exe --rtcheck       # 验证 reqwest::blocking 的运行时上下文限制
```

`--smoke` 与 `--monitortest` 会优先读取 `%APPDATA%\com.mvpn.client\state.json`，
也就是**直接拿你自己的真实节点做验证**，改完配置想确认能不能通就跑这两个。

## 客户端自身更新

设置 → 客户端更新 → 填「版本清单地址」。清单是一个 JSON：

```json
{
  "version": "0.2.0",
  "notes": "新增流量统计与实时连接列表",
  "url": "https://github.com/<owner>/<repo>/releases/download/v0.2.0/Mvpn-0.2.0-win-x64.zip"
}
```

仓库里附带 `latest.json` 模板，配合 GitHub Releases 即可用。
本项目已发布 v0.2.0，清单地址为：

```
https://github.com/tmclsamxy/Mvpn/releases/latest/download/latest.json
```

（这是 GitHub 的稳定地址，永远指向最新 Release 的同名资源，把它填进客户端设置即可长期使用。）

更新流程：检查版本 → 提示 → 下载 zip 并解出 exe → 停止内核 →
把运行中的 exe 重命名为 `.old`（Windows 允许）→ 换入新 exe → 清理 → 重启。
若替换被安全软件拦截，会把新版本留在同目录并提示路径，不会破坏现有安装。

### 自己发一个新版本

```bash
MVPN_REPO=tmclsamxy/Mvpn GH_TOKEN=xxx ./scripts/publish.sh 0.3.0
```

一条命令完成：构建 → 打 zip → 生成 `latest.json` → 创建 Release → 上传两个资产。
客户端把清单地址指向 `releases/latest/download/latest.json` 后就会自动发现新版本。

## 使用

1. 首次运行若未内置内核，进入 **设置 → 内核 → 下载 / 更新内核**
2. 在 **节点** 页添加节点，或「从剪贴板导入」分享链接，或在 **订阅** 页添加订阅地址
3. 回到 **概览**，选择节点 → 点右上角 **启动**
4. 需要局域网共享：**设置 → 本地/局域网代理 → 允许局域网连接**，然后重启内核
5. 需要全局代理：**设置 → TUN 模式 → 启用 TUN**（以管理员身份运行 Mvpn）
6. 分流规则在 **分流** 页配置，改完重启内核生效

## 数据目录

`%APPDATA%\com.mvpn.client\`

```
state.json                # 你的配置（节点/规则/设置）
core/sing-box.exe         # 下载的内核
runtime/config.json       # 每次启动生成的 sing-box 配置
runtime/cache.db          # 规则集缓存
logs/mvpn.log             # 运行日志
```

## 已知限制

- **首期仅支持 Windows**（macOS/Linux 需替换 `sysproxy.rs` 与托盘逻辑）
- TUN 模式需以管理员身份运行 Mvpn
- 自定义规则/设置修改后需重启内核生效（**模式切换是热生效的**，走 Clash API）
- 规则集默认走 jsDelivr CDN；如需切 GitHub 官方源，改 `state.json` 的 `setting.rulesetSource` 为 `github`
- **GNU 目标下 `WebView2Loader.dll` 是运行时必需**（MSVC 目标为静态链接）。`scripts/package.sh` 会自动复制；手动构建时若 exe 以退出码 127 秒退，就是缺这个文件
- 应用在 Git Bash 里直接运行会报 `api-ms-win-core-winrt-*.dll` 找不到 —— 那是 MSYS 加载器不认 API set，请从资源管理器或 `cmd` 启动

## 致谢

- 内核：[SagerNet/sing-box](https://github.com/SagerNet/sing-box)
- 规则集：[sing-geosite](https://github.com/SagerNet/sing-geosite) / [sing-geoip](https://github.com/SagerNet/sing-geoip)
- 交互参考：[2dust/v2rayN](https://github.com/2dust/v2rayN)
