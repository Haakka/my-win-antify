export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const idx = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  const val = bytes / Math.pow(1024, idx);
  return `${val >= 100 || idx === 0 ? val.toFixed(0) : val.toFixed(1)} ${units[idx]}`;
}

export function formatBps(bps: number): string {
  return `${formatBytes(bps)}/s`;
}

export function formatUptime(secs: number): string {
  if (secs <= 0) return "0秒";
  const days = Math.floor(secs / 86400);
  const hours = Math.floor((secs % 86400) / 3600);
  const mins = Math.floor((secs % 3600) / 60);
  const rem = secs % 60;
  const parts: string[] = [];
  if (days > 0) parts.push(`${days}天`);
  if (hours > 0) parts.push(`${hours}小时`);
  if (mins > 0) parts.push(`${mins}分钟`);
  if (parts.length === 0) parts.push(`${rem}秒`);
  return parts.join(" ");
}

export function formatTimeHms(tsMs: number): string {
  const d = new Date(tsMs);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function formatDateTimeShort(tsMs: number): string {
  const d = new Date(tsMs);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function formatRelativeTime(tsMs: number | undefined): {
  label: string;
  recent: boolean;
} {
  if (!tsMs) return { label: "—", recent: false };
  const diffSec = Math.max(0, Math.floor((Date.now() - tsMs) / 1000));
  if (diffSec < 30) return { label: "刚刚", recent: true };
  if (diffSec < 60) return { label: `${diffSec} 秒前`, recent: true };
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 5) return { label: `${diffMin} 分钟前`, recent: true };
  if (diffMin < 60) return { label: `${diffMin} 分钟前`, recent: false };
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return { label: `${diffHr} 小时前`, recent: false };
  const diffDay = Math.floor(diffHr / 24);
  return { label: `${diffDay} 天前`, recent: false };
}

export function randomId(prefix: string): string {
  return `${prefix}-${Math.random().toString(36).slice(2, 8)}${Date.now().toString(36).slice(-4)}`;
}
