import type {
  ActivityLogEntry,
  AppConfig,
  CoexistenceConflict,
  ConfigBackupExport,
  ConnectionSnapshot,
  CurrentWifiInfo,
  DiagnosticBundleSummary,
  NetworkInterfaceInfo,
  PipeConnectionState,
  PongInfo,
  ProxyTestResult,
  Rule,
  RulesImportPreview,
  RunningProcessInfo,
  ServiceStatus,
  TrafficHistoryBucket,
  TrafficHistoryQuery,
  TrafficHistoryResponse,
  TrafficStats,
} from "@/types";
import type { CoreClient, CoreEventCallbacks } from "./coreClient";

function clone<T>(val: T): T {
  return JSON.parse(JSON.stringify(val)) as T;
}

const now = Date.now();

const INITIAL_CONFIG: AppConfig = {
  schemaVersion: 2,
  activeProfileId: "default",
  profiles: [
    {
      id: "default",
      name: "General",
      defaultAction: "direct",
      defaultProxyId: "proxy-flclash",
    },
  ],
  proxies: [
    {
      id: "proxy-flclash",
      name: "FlClash",
      protocol: "socks5",
      host: "127.0.0.1",
      port: 7890,
      auth: null,
      interfaceName: null,
      allowLan: false,
      udpRelay: true,
      fallbackDirect: false,
      skipTlsVerify: false,
      enabled: true,
    },
  ],
  rules: [
    {
      id: "rule-antigravity",
      profileId: "default",
      enabled: true,
      name: "Antigravity",
      matcher: {
        kind: "executablePath",
        path: "C:\\Program Files\\Antigravity\\antigravity.exe",
        includeChildren: true,
      },
      action: "proxy",
      proxyId: "proxy-flclash",
      networkLock: false,
      priority: 10,
      targetHosts: [],
      targetPorts: [],
    },
    {
      id: "rule-antigravity-ide",
      profileId: "default",
      enabled: true,
      name: "Antigravity IDE",
      matcher: {
        kind: "executablePath",
        path: "C:\\Program Files\\Antigravity IDE\\antigravity-ide.exe",
        includeChildren: true,
      },
      action: "proxy",
      proxyId: "proxy-flclash",
      networkLock: false,
      priority: 20,
      targetHosts: [],
      targetPorts: [],
    },
    {
      id: "rule-chatgpt",
      profileId: "default",
      enabled: true,
      name: "ChatGPT",
      matcher: {
        kind: "processName",
        name: "ChatGPT.exe",
        includeChildren: true,
      },
      action: "proxy",
      proxyId: "proxy-flclash",
      networkLock: false,
      priority: 30,
      targetHosts: [],
      targetPorts: [],
    },
    {
      id: "rule-gemini",
      profileId: "default",
      enabled: true,
      name: "Gemini",
      matcher: {
        kind: "processName",
        name: "Gemini.exe",
        includeChildren: true,
      },
      action: "proxy",
      proxyId: "proxy-flclash",
      networkLock: false,
      priority: 40,
      targetHosts: [],
      targetPorts: [],
    },
    {
      id: "rule-terminal",
      profileId: "default",
      enabled: true,
      name: "Windows Terminal",
      matcher: {
        kind: "processName",
        name: "WindowsTerminal.exe",
        includeChildren: true,
      },
      action: "proxy",
      proxyId: "proxy-flclash",
      networkLock: false,
      priority: 50,
      targetHosts: [],
      targetPorts: [],
    },
  ],
  settings: {
    theme: "light",
    language: "zh-CN",
    launchAtLogin: false,
    autoStartProxy: false,
    bypassLan: true,
    bypassHosts: ["*.corp.example.com", "intranet.example.com"],
    recordConnections: true,
    discoverUnmatched: false,
  },
  automation: {
    enabled: false,
    wifiRules: [
      {
        id: "wifi-office",
        ssid: "Studio-5G",
        profileId: "default",
        enabled: true,
      },
    ],
  },
};

const SAMPLE_PROCESSES: RunningProcessInfo[] = [
  {
    pid: 12480,
    parentPid: 1024,
    name: "antigravity.exe",
    executablePath: "C:\\Program Files\\Antigravity\\antigravity.exe",
    appName: "Antigravity",
    hasActiveConnections: true,
  },
  {
    pid: 14832,
    parentPid: 12480,
    name: "language_server_windows_x64.exe",
    executablePath:
      "C:\\Program Files\\Antigravity\\resources\\app\\language_server_windows_x64.exe",
    appName: "Antigravity Language Server",
    hasActiveConnections: true,
  },
  {
    pid: 8820,
    parentPid: 1024,
    name: "WindowsTerminal.exe",
    executablePath:
      "C:\\Program Files\\WindowsApps\\Microsoft.WindowsTerminal\\WindowsTerminal.exe",
    appName: "Windows Terminal",
    hasActiveConnections: true,
  },
  {
    pid: 19244,
    parentPid: 8820,
    name: "curl.exe",
    executablePath: "C:\\Windows\\System32\\curl.exe",
    appName: "curl",
    hasActiveConnections: false,
  },
  {
    pid: 6620,
    parentPid: 1024,
    name: "msedge.exe",
    executablePath:
      "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    appName: "Microsoft Edge",
    hasActiveConnections: true,
  },
  {
    pid: 7412,
    parentPid: 1024,
    name: "Code.exe",
    executablePath: "C:\\Users\\User\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe",
    appName: "Visual Studio Code",
    hasActiveConnections: false,
  },
];

export class MockCoreClient implements CoreClient {
  private config: AppConfig = clone(INITIAL_CONFIG);
  private running = true;
  private startedAtMs: number | null = now - 3675 * 1000;
  private nextConnId = 100;
  private nextLogId = 20;
  private timer: number | null = null;
  private callbacks: CoreEventCallbacks | null = null;
  private mockWifi: CurrentWifiInfo = {
    supported: true,
    connected: true,
    ssid: "Studio-5G",
    interfaceName: "WLAN",
  };
  private mockConflicts: CoexistenceConflict[] = [
    {
      id: "sys-proxy",
      kind: "systemProxy",
      severity: "warn",
      title: "检测到 Windows 系统代理已开启",
      detail: "127.0.0.1:7890 当前已设为系统代理，通常可与本工具按应用分流共存，如遇重复代理请检查规则。",
      dismissed: false,
    },
  ];
  private historyCleared = false;

  private connections: ConnectionSnapshot[] = [
    {
      id: 95,
      timestampMs: now - 2000,
      protocol: "tcp",
      pid: 14832,
      processName: "language_server_windows_x64.exe",
      executablePath:
        "C:\\Program Files\\Antigravity\\resources\\app\\language_server_windows_x64.exe",
      appName: "Antigravity",
      srcAddr: "192.168.1.24:54312",
      dstAddr: "142.250.190.106:443",
      domain: "play.googleapis.com",
      action: "proxy",
      ruleId: "rule-antigravity",
      ruleName: "Antigravity",
      proxyId: "proxy-flclash",
      proxyName: "FlClash",
      state: "active",
      bytesUp: 48_210,
      bytesDown: 192_440,
      error: null,
      matchReason: "命中规则「Antigravity」",
      failureCode: null,
    },
    {
      id: 94,
      timestampMs: now - 9000,
      protocol: "udp",
      pid: 8820,
      processName: "curl.exe",
      executablePath: "C:\\Windows\\System32\\curl.exe",
      appName: "Windows Terminal",
      srcAddr: "192.168.1.24:54288",
      dstAddr: "8.8.8.8:53",
      domain: "api.github.com",
      action: "proxy",
      ruleId: "rule-terminal",
      ruleName: "Windows Terminal",
      proxyId: "proxy-flclash",
      proxyName: "FlClash",
      state: "closed",
      bytesUp: 96,
      bytesDown: 184,
      error: null,
      matchReason: "DNS-over-TCP 经代理转发",
      failureCode: null,
    },
    {
      id: 93,
      timestampMs: now - 28000,
      protocol: "tcp",
      pid: 6620,
      processName: "msedge.exe",
      executablePath:
        "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
      appName: "Microsoft Edge",
      srcAddr: "192.168.1.24:54101",
      dstAddr: "20.190.159.2:443",
      domain: "login.microsoftonline.com",
      action: "direct",
      ruleId: null,
      ruleName: null,
      proxyId: null,
      proxyName: null,
      state: "active",
      bytesUp: 4_120,
      bytesDown: 12_880,
      error: null,
      matchReason: "未命中规则，按默认策略直连",
      failureCode: null,
    },
  ];

  private logs: ActivityLogEntry[] = [
    {
      id: 4,
      timestampMs: now - 3675 * 1000,
      level: "info",
      category: "service",
      title: "Engine Started (Mock Preview)",
      detail: "WinDivert FLOW + NETWORK reflection active in offline preview",
    },
    {
      id: 3,
      timestampMs: now - 3680 * 1000,
      level: "info",
      category: "rule",
      title: 'Rule "Antigravity" Created',
      detail: "ExecutablePath + includeChildren=true -> FlClash",
    },
    {
      id: 2,
      timestampMs: now - 3700 * 1000,
      level: "info",
      category: "proxy",
      title: 'Proxy "FlClash" Created',
      detail: "SOCKS5 127.0.0.1:7890",
    },
  ];

  async subscribe(callbacks: CoreEventCallbacks): Promise<() => void> {
    this.callbacks = callbacks;
    callbacks.onPipeState("connected");
    callbacks.onStatusChanged(await this.getStatus());
    callbacks.onStatsTick(this.buildStats());
    callbacks.onWifiChanged(clone(this.mockWifi));

    this.timer = window.setInterval(() => {
      if (!this.callbacks) return;
      if (this.running) {
        for (const c of this.connections) {
          if (c.state === "active") {
            c.bytesUp += Math.floor(Math.random() * 640);
            c.bytesDown += Math.floor(Math.random() * 2400);
          }
        }
        if (Math.random() < 0.25 && this.config.settings.recordConnections) {
          this.nextConnId += 1;
          const sample: ConnectionSnapshot = {
            id: this.nextConnId,
            timestampMs: Date.now(),
            protocol: "tcp",
            pid: 14832,
            processName: "language_server_windows_x64.exe",
            executablePath:
              "C:\\Program Files\\Antigravity\\resources\\app\\language_server_windows_x64.exe",
            appName: "Antigravity",
            srcAddr: `192.168.1.24:${54500 + (this.nextConnId % 400)}`,
            dstAddr: "142.250.190.106:443",
            domain: "generativelanguage.googleapis.com",
            action: "proxy",
            ruleId: "rule-antigravity",
            ruleName: "Antigravity",
            proxyId: "proxy-flclash",
            proxyName: "FlClash",
            state: "active",
            bytesUp: 1_240,
            bytesDown: 4_820,
            error: null,
            matchReason: "命中规则「Antigravity」",
            failureCode: null,
          };
          this.connections.unshift(sample);
          if (this.connections.length > 80) {
            this.connections.pop();
          }
          this.callbacks.onConnectionEvent(clone(sample));
        }
      }
      this.callbacks.onStatsTick(this.buildStats());
    }, 1000);

    return () => {
      if (this.timer !== null) {
        window.clearInterval(this.timer);
        this.timer = null;
      }
      this.callbacks = null;
    };
  }

  private buildStats(): TrafficStats {
    const activeConns = this.running
      ? this.connections.filter((c) => c.state === "active").length
      : 0;
    const activeApps = this.running
      ? new Set(
          this.connections
            .filter((c) => c.state === "active")
            .map((c) => c.appName),
        ).size
      : 0;
    const uptimeSecs =
      this.running && this.startedAtMs
        ? Math.floor((Date.now() - this.startedAtMs) / 1000)
        : 0;
    const ts = Date.now();
    return {
      timestampMs: ts,
      running: this.running,
      uptimeSecs,
      activeConnections: activeConns,
      activeApps,
      uploadBps: this.running ? 420 + Math.floor(Math.random() * 900) : 0,
      downloadBps: this.running ? 1600 + Math.floor(Math.random() * 4800) : 0,
      totalProxiedUp: 946_100_000,
      totalProxiedDown: 898_800_000,
      totalDirectUp: 2_048,
      totalDirectDown: 4_096,
      ruleLastMatched: this.running
        ? [
            { ruleId: "rule-antigravity", lastMatchedAtMs: ts - 2_000 },
            { ruleId: "rule-gemini", lastMatchedAtMs: ts - 60_000 },
            { ruleId: "rule-terminal", lastMatchedAtMs: ts - 120_000 },
            { ruleId: "rule-chatgpt", lastMatchedAtMs: ts - 660_000 },
          ]
        : [],
      proxyHealth: this.config.proxies.map((p) => ({
        proxyId: p.id,
        score: p.enabled ? 100 : 0,
        status: p.enabled ? ("healthy" as const) : ("unknown" as const),
        lastErrorCode: null,
        lastError: null,
      })),
    };
  }

  async ping(): Promise<PongInfo> {
    return {
      coreVersion: "0.1.0-mock",
      protocolVersion: 1,
      servicePid: 9999,
    };
  }

  async getPipeState(): Promise<PipeConnectionState> {
    return "connected";
  }

  async getStatus(): Promise<ServiceStatus> {
    return {
      running: this.running,
      startedAtMs: this.startedAtMs,
      uptimeSecs:
        this.running && this.startedAtMs
          ? Math.floor((Date.now() - this.startedAtMs) / 1000)
          : 0,
      servicePid: 9999,
      coreVersion: "0.2.0-mock",
      driverReady: true,
      activeProfileId: this.config.activeProfileId,
      lastError: null,
      configError: null,
      invalidRuleIds: [],
    };
  }

  async startEngine(): Promise<ServiceStatus> {
    this.running = true;
    this.startedAtMs = Date.now();
    const status = await this.getStatus();
    this.emitLog("info", "service", "Engine Started (Mock)", "Traffic interception active");
    this.callbacks?.onStatusChanged(status);
    return status;
  }

  async stopEngine(): Promise<ServiceStatus> {
    this.running = false;
    this.startedAtMs = null;
    const status = await this.getStatus();
    this.emitLog("info", "service", "Engine Stopped (Mock)", "Direct routing restored");
    this.callbacks?.onStatusChanged(status);
    return status;
  }

  async getConfig(): Promise<AppConfig> {
    return clone(this.config);
  }

  async saveConfig(config: AppConfig): Promise<AppConfig> {
    this.config = clone(config);
    this.callbacks?.onConfigChanged(clone(this.config));
    this.emitLog("info", "app", "Configuration Updated", `Profile "${this.config.activeProfileId}"`);
    return clone(this.config);
  }

  async restoreConfigBackup(): Promise<AppConfig> {
    this.callbacks?.onConfigChanged(clone(this.config));
    this.callbacks?.onStatusChanged(await this.getStatus());
    this.emitLog("info", "app", "Configuration Restored From Backup", null);
    return clone(this.config);
  }

  async setActiveProfile(profileId: string): Promise<AppConfig> {
    this.config.activeProfileId = profileId;
    this.callbacks?.onConfigChanged(clone(this.config));
    this.callbacks?.onStatusChanged(await this.getStatus());
    return clone(this.config);
  }

  async getConnections(
    includeClosed: boolean,
    limit: number | null,
  ): Promise<ConnectionSnapshot[]> {
    let list = this.connections.filter((c) =>
      includeClosed ? true : c.state === "active" || c.state === "connecting",
    );
    if (limit !== null) {
      list = list.slice(0, limit);
    }
    return clone(list);
  }

  async clearConnections(): Promise<void> {
    this.connections = [];
  }

  async listRunningProcesses(): Promise<RunningProcessInfo[]> {
    return clone(SAMPLE_PROCESSES);
  }

  async listNetworkInterfaces(): Promise<NetworkInterfaceInfo[]> {
    return [
      {
        name: "WLAN",
        description: "Intel(R) Wi-Fi 6E AX211 160MHz",
        ipv4: "192.168.1.24",
        isUp: true,
      },
      {
        name: "以太网",
        description: "Realtek PCIe 2.5GbE Family Controller",
        ipv4: "192.168.10.12",
        isUp: false,
      },
    ];
  }

  async testProxy(proxyId: string): Promise<ProxyTestResult> {
    const found = this.config.proxies.find((p) => p.id === proxyId);
    const result: ProxyTestResult = found
      ? {
          proxyId,
          reachable: true,
          latencyMs: 18 + Math.floor(Math.random() * 35),
          error: null,
          checkedAtMs: Date.now(),
          stale: false,
        }
      : {
          proxyId,
          reachable: false,
          latencyMs: null,
          error: "Proxy not found",
          checkedAtMs: Date.now(),
          stale: false,
        };
    return result;
  }

  async getActivityLogs(limit: number | null): Promise<ActivityLogEntry[]> {
    const items = limit !== null ? this.logs.slice(0, limit) : this.logs;
    return clone(items);
  }

  async clearActivityLogs(): Promise<void> {
    this.logs = [];
  }

  async launchService(): Promise<boolean> {
    return true;
  }

  async exportConfigBackup(includePasswords: boolean): Promise<ConfigBackupExport> {
    const exported = clone(this.config);
    if (!includePasswords) {
      for (const p of exported.proxies) {
        if (p.auth) {
          p.auth = { username: p.auth.username, password: "" };
        }
      }
    }
    return {
      json: JSON.stringify(exported, null, 2),
      includesPasswords: includePasswords,
      warning: includePasswords
        ? "此备份包含明文代理密码，请妥善加密或离线保管。"
        : null,
    };
  }

  async importConfigBackup(json: string): Promise<AppConfig> {
    const parsed = JSON.parse(json) as AppConfig;
    if (!parsed || !Array.isArray(parsed.profiles) || parsed.profiles.length === 0) {
      throw new Error("备份文件缺少有效的配置列表");
    }
    if (!parsed.profiles.some((p) => p.id === parsed.activeProfileId)) {
      parsed.activeProfileId = parsed.profiles[0]!.id;
    }
    if (!parsed.automation) {
      parsed.automation = { enabled: false, wifiRules: [] };
    }
    parsed.schemaVersion = 2;
    this.config = clone(parsed);
    this.callbacks?.onConfigChanged(clone(this.config));
    this.emitLog("info", "app", "已从备份导入配置", null);
    return clone(this.config);
  }

  async exportRulesJson(profileId: string | null): Promise<string> {
    const pid = profileId ?? this.config.activeProfileId;
    const rules = this.config.rules
      .filter((r) => r.profileId === pid)
      .map((r) => {
        const next = clone(r);
        if (next.matcher.kind === "executablePath") {
          next.matcher.path = next.matcher.path.replace(/^C:\\Users\\[^\\]+/i, "%USERPROFILE%");
        } else if (next.matcher.kind === "directory") {
          next.matcher.dir = next.matcher.dir.replace(/^C:\\Users\\[^\\]+/i, "%USERPROFILE%");
        }
        return next;
      });
    return JSON.stringify(rules, null, 2);
  }

  async importRulesJson(
    json: string,
    profileId: string,
    dryRun: boolean,
  ): Promise<RulesImportPreview> {
    const incoming = JSON.parse(json) as Rule[];
    if (!Array.isArray(incoming)) {
      throw new Error("规则 JSON 格式无效，应为数组");
    }
    const existingKeys = new Set(
      this.config.rules
        .filter((r) => r.profileId === profileId)
        .map((r) => `${r.matcher.kind}:${JSON.stringify(r.matcher).toLowerCase()}`),
    );
    const proxyIds = new Set(this.config.proxies.map((p) => p.id));
    const added: string[] = [];
    const skippedDuplicates: string[] = [];
    const missingProxyRules: string[] = [];

    for (const raw of incoming) {
      const rule = clone(raw);
      rule.profileId = profileId;
      const key = `${rule.matcher.kind}:${JSON.stringify(rule.matcher).toLowerCase()}`;
      if (existingKeys.has(key)) {
        skippedDuplicates.push(rule.name);
        continue;
      }
      if (rule.action === "proxy" && rule.proxyId && !proxyIds.has(rule.proxyId)) {
        missingProxyRules.push(rule.name);
      }
      added.push(rule.name);
      if (!dryRun) {
        this.config.rules.push(rule);
      }
    }
    if (!dryRun && added.length > 0) {
      this.callbacks?.onConfigChanged(clone(this.config));
    }
    return {
      committed: !dryRun,
      totalInFile: incoming.length,
      added,
      skippedDuplicates,
      missingProxyRules,
    };
  }

  async getTrafficHistory(query: TrafficHistoryQuery): Promise<TrafficHistoryResponse> {
    if (this.historyCleared) {
      return {
        range: query.range,
        profileId: query.profileId,
        totalProxiedUp: 0,
        totalProxiedDown: 0,
        totalDirectUp: 0,
        totalDirectDown: 0,
        buckets: [],
        topRules: [],
        topProxies: [],
        topApps: [],
      };
    }
    const count = query.range === "24h" ? 24 : 30;
    const stepMs = query.range === "24h" ? 3600_000 : 86400_000;
    const nowMs = Date.now();
    const buckets: TrafficHistoryBucket[] = [];
    let totalProxiedUp = 0;
    let totalProxiedDown = 0;
    let totalDirectUp = 0;
    let totalDirectDown = 0;
    for (let i = count - 1; i >= 0; i--) {
      const ts = nowMs - i * stepMs;
      const d = new Date(ts);
      const label =
        query.range === "24h"
          ? `${String(d.getHours()).padStart(2, "0")}:00`
          : `${d.getMonth() + 1}/${d.getDate()}`;
      const pu = 120_000 + ((i * 73_121) % 900_000);
      const pd = 640_000 + ((i * 213_519) % 4_800_000);
      const du = 24_000 + ((i * 19_001) % 140_000);
      const dd = 85_000 + ((i * 51_007) % 420_000);
      totalProxiedUp += pu;
      totalProxiedDown += pd;
      totalDirectUp += du;
      totalDirectDown += dd;
      buckets.push({
        bucketStartMs: ts,
        label,
        proxiedUp: pu,
        proxiedDown: pd,
        directUp: du,
        directDown: dd,
      });
    }
    return {
      range: query.range,
      profileId: query.profileId,
      totalProxiedUp,
      totalProxiedDown,
      totalDirectUp,
      totalDirectDown,
      buckets,
      topRules: [
        { key: "rule-antigravity", label: "Antigravity", upBytes: 12_400_000, downBytes: 84_200_000 },
        { key: "rule-terminal", label: "Windows Terminal", upBytes: 3_100_000, downBytes: 14_800_000 },
      ],
      topProxies: [
        { key: "proxy-flclash", label: "FlClash", upBytes: totalProxiedUp, downBytes: totalProxiedDown },
      ],
      topApps: [
        { key: "antigravity.exe", label: "Antigravity", upBytes: 12_400_000, downBytes: 84_200_000 },
        { key: "curl.exe", label: "curl", upBytes: 3_100_000, downBytes: 14_800_000 },
      ],
    };
  }

  async clearTrafficHistory(): Promise<void> {
    this.historyCleared = true;
  }

  async getCurrentWifi(): Promise<CurrentWifiInfo> {
    return clone(this.mockWifi);
  }

  async getCoexistenceConflicts(): Promise<CoexistenceConflict[]> {
    return clone(this.mockConflicts.filter((c) => !c.dismissed));
  }

  async dismissCoexistenceConflict(conflictId: string): Promise<void> {
    for (const c of this.mockConflicts) {
      if (c.id === conflictId) c.dismissed = true;
    }
  }

  async collectDiagnosticBundle(outputPath: string | null): Promise<DiagnosticBundleSummary> {
    return {
      path: outputPath ?? "C:\\ProgramData\\MyWinAntify\\diagnostics-mock.json",
      generatedAtMs: Date.now(),
      logCount: this.logs.length,
      interfaceCount: 2,
      sanitizedFields: ["proxyHost", "proxyCredentials", "wifiSsid", "userProfilePath"],
    };
  }

  private emitLog(
    level: ActivityLogEntry["level"],
    category: ActivityLogEntry["category"],
    title: string,
    detail: string | null,
  ) {
    this.nextLogId += 1;
    const entry: ActivityLogEntry = {
      id: this.nextLogId,
      timestampMs: Date.now(),
      level,
      category,
      title,
      detail,
    };
    this.logs.unshift(entry);
    this.callbacks?.onActivityLog(clone(entry));
  }
}
