# MyWinAntify

**English** | [简体中文](README.zh-CN.md)

An unofficial, lightweight Windows per-app proxy tool inspired by **[Antify](https://antifyapp.com)** for macOS — built for **personal use and technical learning**.

![Platform](https://img.shields.io/badge/Platform-Windows%2010%2F11%20x64-0A7AFF)
![Stack](https://img.shields.io/badge/Stack-Rust%20%2B%20WinDivert%20%2B%20Tauri%202%20%2B%20Vue%203-blue)
![License](https://img.shields.io/badge/License-MIT-green)

> [!IMPORTANT]
> **Personal Learning Project & Support Official Antify**:
> - This project was created purely for **personal use** and **technical learning / research**.
> - It is **incomplete and rough around the edges** — please do **not** treat it as a production-ready alternative.
> - If you enjoy this workflow or use macOS, **please support the official [Antify](https://antifyapp.com)** (Official feedback repo: [cyberlesterr/antify-feedback](https://github.com/cyberlesterr/antify-feedback))!

---

## Why This Project Exists

I genuinely love **[Antify](https://antifyapp.com)** on macOS — its per-application transparent proxy model, clean UI, and effortless rule workflow make routing individual apps a joy without setting a global system proxy or installing heavy virtual network adapters.

Unfortunately, Antify does not currently have a Windows version. Since I work on Windows every day and wanted a similar per-app routing experience (and also wanted to learn Windows packet-level traffic interception with WinDivert, Tokio, and Tauri 2), I built this **unofficial personal imitation** for my own use.

Again: **MyWinAntify is far from complete** and exists **for learning and personal experimentation only**. All credit for the original UX design and product vision goes to the official Antify team — **please support the genuine official Antify** and look forward to an official Windows release!

---

## How to Use (Portable Release)

> [!NOTE]
> **Clean Release Guarantee (Zero Personal Data)**:
> Every GitHub Release archive is built completely clean and contains **zero preset proxy nodes, zero credentials, and zero local user data**.
> On first launch, the app automatically initializes an empty default configuration at `%ProgramData%\MyWinAntify\config.json` on your own machine.

### Quick Start
1. Download `MyWinAntify-v0.1.0-win-x64.zip` from the [Releases](../../releases) page.
2. **Extract the entire folder** to a local directory (do not run directly from inside the ZIP preview).
3. Double-click `MyWinAntify.exe`.
   - On launch, Windows will show a **UAC elevation prompt** so the desktop UI can silently start the background traffic engine (`core-service.exe`) and load the signed `WinDivert64.sys` driver (no console window).
4. **Add an Upstream Proxy**: Go to **Proxies** (`代理`) -> click `+` -> add your local or self-hosted SOCKS5 endpoint (for example, `127.0.0.1:7890` exposed by Clash / FlClash / v2rayN).
5. **Add App Rules**: Go to **Rules** (`规则`) -> click `+` -> either browse for a `.exe` executable or pick directly from currently running processes, and assign `Proxy`, `Direct`, or `Block`.
6. **Start Routing**: Go back to **Dashboard** (`仪表盘`) and click **Start** (`启动`). Matched apps will route through your chosen upstream proxy while other apps stay direct.
7. **Clean Exit**: Clicking the window close button (`X`) minimizes MyWinAntify to the system tray so proxying continues in the background. To completely stop the engine and restore direct networking, **right-click the system tray icon and choose Exit (`退出 MyWinAntify`)**.

### System Requirements & Notes
- **OS**: Windows 10 / Windows 11 (x64)
- **Runtime**: Microsoft Edge WebView2 Runtime (pre-installed on Windows 11)
- **Antivirus False Positives**: Because `WinDivert64.sys` is a kernel-level packet/flow driver, Windows Defender or third-party antivirus software may occasionally flag it. This repo bundles the unmodified, officially signed WinDivert 2.2.2-A binaries; if blocked, add the extracted folder to your exclusions.

---

## Current Features (v0.1.0)

- **No Virtual NIC (Non-TUN)**: Uses official signed **WinDivert 2.2.2-A** (`FLOW` + `NETWORK` reflection) rather than Wintun/TAP adapters, coexisting cleanly with Clash, FlClash, v2rayN, or VPN tools.
- **Per-App & Path Rule Matching**: Match by executable path, process name, or directory, with optional target host/IP/port filters and `Proxy` / `Direct` / `Block` (fast TCP RST fail) actions.
- **Child Process Inheritance (`includeChildren`)**: Traces parent process chains so child processes spawned by IDEs or terminals (e.g., language servers, `node.exe`, `curl.exe`) automatically inherit parent rules.
- **Network Lock (`networkLock`)**: Prevents traffic leaks by immediately closing connections instead of falling back to direct when an upstream proxy is unreachable.
- **Domain Sniffing (`TLS SNI` / `HTTP Host`)**: Non-destructively peeks the initial packet in the local Tokio relay to extract SNI / `Host` headers, display domains in the Connections view, and send domain addresses (`ATYP = 0x03`) during SOCKS5 `CONNECT`.
- **Privilege Separation**: `core-service.exe` runs elevated and enforces a single-writer config model (`%ProgramData%\MyWinAntify\config.json`), while the Tauri 2 desktop UI runs unprivileged over a DACL-protected Named Pipe (`\\.\pipe\my-win-antify-core-v1`).
- **UI & Preview Mode**: Light/Dark/System themes, multiple accent skins, live connection stream, and an offline **Mock Preview** toggle in the sidebar for UI exploration without running the privileged service.

---

## Current Limitations (Why It Is Incomplete)

This is an **early v0.1.0 learning prototype** with notable gaps:
1. **IPv4 TCP + SOCKS5 Only**: UDP (`SOCKS5 UDP ASSOCIATE`), IPv6 traffic interception, and upstream HTTP CONNECT proxies are not implemented yet.
2. **Placeholder Views**: Historical traffic charts, Wi-Fi automation switching, config import/export, and Windows SCM service registration are incomplete or UI placeholders.
3. **Edge Cases**: Complex multi-NIC setups, QUIC/UDP fallback apps, or certain anti-cheat/protected processes may hit unhandled edge cases.

---

## Build from Source

### Prerequisites
- Windows 10 / 11 (x64)
- [Rust](https://www.rust-lang.org/) toolchain (Cargo + MSVC or GNU target; `vendor/windivert/x64/` and `.cargo/config.toml` are already configured)
- [Node.js](https://nodejs.org/) 20+ & npm

### Commands

```powershell
# 1. Install frontend dependencies
npm install

# 2. Frontend typecheck & production bundle
npm run typecheck
npm run build

# 3. Run Rust workspace tests
cargo test --workspace

# 4. Build portable Windows x64 ZIP (outputs to release/MyWinAntify-v0.1.0-win-x64.zip)
npm run package
```

---

## Project Structure

```text
my-win-antify/
├── crates/
│   ├── shared/          # Shared domain models, errors, and Named Pipe IPC schema
│   └── core-service/    # Privileged traffic engine (WinDivert, process lookup, SNI sniffer, SOCKS5 relay, IPC server)
├── src-tauri/           # Unprivileged Tauri 2 desktop shell (Named Pipe client, launcher, tray, Mica window)
├── src/                 # Vue 3 + TypeScript + Tailwind CSS v4 frontend
├── vendor/windivert/    # Official precompiled signed WinDivert 2.2.2-A binaries & header
├── scripts/             # Portable release packaging script
├── LICENSE              # MIT License
└── THIRD_PARTY_NOTICES.md
```

---

## License, Third-Party Notices & Disclaimer

- Source code is licensed under the **[MIT License](LICENSE)**.
- Packet/flow interception uses **WinDivert 2.2.2-A** (Copyright basil00, dual-licensed under LGPL-3.0-or-later or GPL-2.0-or-later). See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
- **Non-Affiliation**: MyWinAntify is an independent personal learning project and is not affiliated with official **[Antify](https://antifyapp.com)**. Please support the official version!
- **Disclaimer**: Provided strictly for personal learning, protocol research, and lawful local network debugging. Use at your own risk.
