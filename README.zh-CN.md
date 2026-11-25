# MyWinAntify

[English](README.md) | **简体中文**

一个仿照 macOS 端 **[Antify](https://antifyapp.com)** 编写的 Windows 轻量级应用级代理接管工具（**个人自用 & 技术学习项目**）。

![Platform](https://img.shields.io/badge/Platform-Windows%2010%2F11%20x64-0A7AFF)
![Stack](https://img.shields.io/badge/Stack-Rust%20%2B%20WinDivert%20%2B%20Tauri%202%20%2B%20Vue%203-blue)
![License](https://img.shields.io/badge/License-MIT-green)

> [!IMPORTANT]
> **声明与号召支持正版**：
> - 本项目最初纯粹是为了**个人自用与技术学习**而写的实验性作品。
> - **当前版本尚不完善**，很多功能仍在占位阶段、边界场景也有待打磨，**仅供学习与技术交流使用**。
> - **强烈号召大家支持官方正版 [Antify](https://antifyapp.com)**（官方反馈仓库：[cyberlesterr/antify-feedback](https://github.com/cyberlesterr/antify-feedback)）！

---

## 这个项目为什么存在？

我非常喜欢 macOS 端的 **[Antify](https://antifyapp.com)** —— 无需接管全局系统代理，也不用创建虚拟网卡，就能极其优雅地让指定应用各自走不同的代理规则，界面和交互体验都很出色。

但遗憾的是，Antify 目前只有 macOS 版本，**没有 Windows 端**。因为平时主力环境也在使用 Windows，很想在 Windows 上体验类似的按应用分流方式，同时也想借此深入学习 Windows 底层流量拦截（WinDivert）、Rust 异步透明中继（Tokio）以及 Tauri 2 跨权限架构，于是自己动手仿照 Antify 的界面与思路做了这个小项目。

需要再次坦诚说明：**这是一个不完善的个人仿制学习项目，仅供个人学习与技术交流使用，无法替代成熟的正式软件。** 原版的产品设计与交互创意全部归属于 Antify 官方团队，**如果您使用 macOS，请务必支持官方正版 [Antify](https://antifyapp.com)**，也一起期待官方未来推出 Windows 版本！

---

## 怎么使用？（便携发行版）

> [!NOTE]
> **纯净发行版保证（绝无节点隐私泄漏）**：
> GitHub Releases 中发布的压缩包**完全干净**，**不包含任何预设节点信息、代理账号或作者个人本地数据**。
> 首次运行后，程序只会在你自己的电脑上自动生成一份空白默认配置（`%ProgramData%\MyWinAntify\config.json`）。

### 使用步骤
1. 前往本仓库 [Releases](../../releases) 页面下载 `MyWinAntify-v0.1.0-win-x64.zip`。
2. 将压缩包**完整解压到本地文件夹**（请勿直接在压缩包预览窗口中双击运行）。
3. 双击运行 `MyWinAntify.exe`：
   - 启动时会弹出一次 Windows **管理员权限（UAC）授权提示**，点击「是」，桌面端会在后台静默拉起核心引擎（`core-service.exe`）并加载 WinDivert 驱动，无黑框弹窗。
4. **添加上游代理**：进入左侧「代理」页面，点击右上角 `+`，添加你本地或自建的 SOCKS5 上游代理（例如 Clash / FlClash / v2rayN 开放的 `127.0.0.1:7890`）。
5. **配置应用规则**：进入「规则」页面，点击 `+`，可以通过浏览选择 `.exe` 文件，或直接从「当前运行进程列表」中一键勾选需要代理的应用，设置动作为「代理 / 直连 / 阻断」。
6. **开始使用**：回到「仪表盘」，点击「启动」。命中规则的应用流量会经由指定上游代理转发，其余应用保持直连。
7. **彻底退出**：点击窗口右上角 `×` 会最小化到系统托盘继续代理；如需彻底关闭并立即恢复系统直连，请**右键点击系统托盘图标 -> 选择「退出 MyWinAntify」**。

### 运行环境与注意事项
- **系统要求**：Windows 10 / Windows 11 (x64)
- **依赖组件**：Microsoft Edge WebView2 运行时（Windows 11 默认自带）
- **杀毒软件误报提示**：由于底层依赖内核级抓包/改包驱动 `WinDivert64.sys`，Windows Defender 或部分安全软件可能会产生误报。本项目使用的是 WinDivert 2.2.2-A 官方原版签名二进制；若被拦截，请将解压目录添加到安全软件排除项。

---

## 已实现特性（v0.1.0）

- **无虚拟网卡（非 TUN 模式）**：采用 WinDivert 2.2.2-A 官方签名驱动实现 `FLOW` + `NETWORK` 双层反射接管，不安装任何虚拟网卡，可与 Clash / FlClash / v2rayN 等工具和平共存。
- **按应用精细分流**：支持按可执行文件路径、进程名、目录匹配规则，支持 `Proxy`（代理）、`Direct`（直连）、`Block`（注入 TCP RST 快速失败阻断）三种动作，以及按目标域名/IP/端口过滤。
- **子进程继承（`includeChildren`）**：自动追溯父进程链，IDE、终端或启动器拉起的子进程（如语言服务、`node.exe`、`curl.exe` 等）可自动继承父级规则。
- **网络锁（`networkLock`）**：上游代理不可达或拒绝连接时直接断开本地连接，严禁静默回退直连，避免流量泄漏。
- **轻量域名嗅探（TLS SNI / HTTP Host）**：本地 Tokio 中继对首包执行非破坏性 `peek()`，解析 TLS SNI 或 HTTP `Host`，在连接监控页展示域名并在 SOCKS5 `CONNECT` 时优先传递域名（`ATYP = 0x03`）。
- **特权服务与桌面壳分离**：`core-service.exe` 负责底层引擎与单一配置文件读写；`MyWinAntify.exe` 以普通权限运行，通过带 DACL 安全控制的 Named Pipe（`\\.\pipe\my-win-antify-core-v1`）通信。
- **主题与离线 Mock 预览**：支持浅色 / 深色 / 跟随系统外观、多套强调色皮肤、实时连接监控；侧边栏提供「Mock 预览」开关，无需开启核心服务即可体验界面交互。

---

## 当前不完善之处 / 已知局限

当前仅为 **v0.1.0 个人学习原型版本**，存在以下已知不足：

1. **仅覆盖 IPv4 TCP 与 SOCKS5 上游**：UDP（`SOCKS5 UDP ASSOCIATE`）、IPv6 流量接管以及上游 HTTP CONNECT 代理类型尚未实现。
2. **部分页面为框架占位**：历史流量柱状图、Wi-Fi 自动化切换配置、配置导入导出、Windows Service (SCM) 服务注册等功能尚未完成。
3. **复杂环境兼容性**：在多网卡、QUIC/UDP 回退或受保护进程等场景下仍可能存在未覆盖到的边缘问题。

---

## 从源码构建

### 前置环境
- Windows 10 / 11 (x64)
- [Rust](https://www.rust-lang.org/) 工具链（仓库已内置 `vendor/windivert/x64/` 官方预编译二进制与 `.cargo/config.toml`）
- [Node.js](https://nodejs.org/) 20+ 与 npm

### 常用命令

```powershell
# 1. 安装前端依赖
npm install

# 2. 前端类型检查与构建
npm run typecheck
npm run build

# 3. 运行 Rust 工作区单元测试
cargo test --workspace

# 4. 打包 Windows x64 便携绿色版（生成 release/MyWinAntify-v0.1.0-win-x64.zip）
npm run package
```

---

## 项目结构

```text
my-win-antify/
├── crates/
│   ├── shared/          # 跨进程共享领域模型、错误码与 Named Pipe IPC 协议
│   └── core-service/    # Windows 特权核心引擎（WinDivert、进程追踪、SNI 嗅探、SOCKS5 中继、IPC 服务）
├── src-tauri/           # 普通权限 Tauri 2 桌面壳（Named Pipe 客户端、服务拉起、托盘、Mica 窗口）
├── src/                 # Vue 3 + TypeScript + Tailwind CSS v4 前端界面
├── vendor/windivert/    # WinDivert 2.2.2-A 官方预编译签名二进制与头文件
├── scripts/             # 便携版打包脚本
├── LICENSE              # MIT License
└── THIRD_PARTY_NOTICES.md
```

---

## 开源协议、第三方致谢与免责声明

- 本项目源码基于 **[MIT License](LICENSE)** 开源。
- 底层驱动使用了 **WinDivert 2.2.2-A**（Copyright basil00，以 LGPL-3.0-or-later / GPL-2.0-or-later 双许可发布），详情请参阅 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
- **非官方声明**：本项目为个人独立学习与仿制作品，与官方 **[Antify](https://antifyapp.com)** 无任何关联；再次号召大家支持官方正版！
- **免责声明**：本项目仅供个人技术学习、网络协议研究与合法合规的本地调试使用，因使用本项目产生的一切后果由使用者自行承担。
