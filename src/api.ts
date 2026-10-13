import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export type Me = {
  deviceId: string;
  deviceName: string;
  profileName: string;
  os: string;
  port: number;
  addresses: string[];
};

export type Settings = {
  onboarded: boolean;
  deviceName: string;
  profileName: string;
  receiveDir: string;
  syncDir: string;
  askBeforeReceiving: boolean;
  syncEnabled: boolean;
  browseScope: BrowseScope;
  launchAtLogin: boolean;
  contextMenu: boolean;
  notifications: boolean;
};

export type BrowseScope = "computer" | "home" | "syncFolder" | "nobody";

export type Device = {
  id: string;
  deviceName: string;
  profileName: string;
  os: string;
  paired: boolean;
  online: boolean;
};

export type TransferStatus = "waiting" | "active" | "done" | "declined" | "failed" | "cancelled";

export type Transfer = {
  id: string;
  deviceId: string;
  deviceName: string;
  direction: "send" | "receive";
  label: string;
  fileCount: number;
  totalBytes: number;
  doneBytes: number;
  status: TransferStatus;
  error: string | null;
  location: string | null;
  startedAt: number;
  canRetry: boolean;
};

export type PeerSync = {
  deviceId: string;
  state: "waiting" | "syncing" | "idle" | "error";
  filesLeft: number;
  bytesLeft: number;
  lastSynced: number | null;
  error: string | null;
};

export type PendingDeletion = { deviceId: string; deviceName: string; path: string; size: number };

export type SyncChange = {
  at: number;
  deviceName: string;
  path: string;
  from: string | null;
  kind: "added" | "updated" | "renamed" | "deleted" | "kept" | "conflict";
};

export type SyncStatus = {
  enabled: boolean;
  folder: string;
  scanning: boolean;
  peers: PeerSync[];
  pendingDeletions: PendingDeletion[];
  issues: string[];
  recent: SyncChange[];
};

export type Snapshot = { me: Me; settings: Settings; devices: Device[]; transfers: Transfer[]; sync: SyncStatus };

export type RemoteEntry = { name: string; path: string; dir: boolean; size: number; modified: number | null; kind: "" | "home" | "drive" };
export type Listing = { path: string; entries: RemoteEntry[]; root: string };

export type TunnelEvent =
  | { kind: "devicesChanged" }
  | { kind: "settingsChanged" }
  | { kind: "pairingCode"; sessionId: string; device: Device; code: string; incoming: boolean }
  | { kind: "pairingFinished"; sessionId: string; deviceId: string; success: boolean; message: string | null }
  | { kind: "incomingOffer"; transferId: string; deviceName: string; label: string; fileCount: number; totalBytes: number }
  | { kind: "transferChanged"; transfer: Transfer }
  | { kind: "syncChanged" };

export const api = {
  state: () => invoke<Snapshot>("get_state"),
  updateSettings: (patch: Partial<Settings>) => invoke<Settings>("update_settings", { patch }),
  pair: (deviceId: string) => invoke<string>("pair_device", { deviceId }),
  respondPairing: (sessionId: string, accept: boolean) => invoke<void>("respond_pairing", { sessionId, accept }),
  unpair: (deviceId: string) => invoke<void>("unpair_device", { deviceId }),
  connectAddress: (address: string) => invoke<Device>("connect_address", { address }),
  send: (deviceId: string, paths: string[]) => invoke<string>("send_paths", { deviceId, paths }),
  respondOffer: (transferId: string, accept: boolean) => invoke<void>("respond_offer", { transferId, accept }),
  list: (deviceId: string, path: string) => invoke<Listing>("list_remote", { deviceId, path }),
  pull: (deviceId: string, paths: string[]) => invoke<string>("pull_remote", { deviceId, paths }),
  clearTransfers: () => invoke<void>("clear_transfers"),
  resolveDeletions: (deviceId: string, paths: string[], del: boolean) => invoke<void>("resolve_deletions", { deviceId, paths, delete: del }),
  syncNow: () => invoke<void>("sync_now"),
  cancelTransfer: (transferId: string) => invoke<void>("cancel_transfer", { transferId }),
  retryTransfer: (transferId: string) => invoke<void>("retry_transfer", { transferId }),
  takeSendRequest: () => invoke<string[]>("take_send_request"),
};

export const onTunnelEvent = (cb: (e: TunnelEvent) => void): Promise<UnlistenFn> =>
  listen<TunnelEvent>("tunnel", (e) => cb(e.payload));

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

export function osLabel(os: string): string {
  return { macos: "Mac", windows: "PC", linux: "Linux", ios: "iPhone", android: "Android" }[os] ?? "Device";
}

export function timeAgo(secs: number): string {
  const d = Math.max(0, Date.now() / 1000 - secs);
  if (d < 45) return "just now";
  if (d < 3600) return `${Math.round(d / 60)} min ago`;
  if (d < 86400) return `${Math.round(d / 3600)} h ago`;
  return new Date(secs * 1000).toLocaleDateString();
}

export const isMac = navigator.userAgent.includes("Mac");
export const platform: "mac" | "windows" | "linux" = isMac ? "mac" : navigator.userAgent.includes("Windows") ? "windows" : "linux";
export const fileManager = isMac ? "Finder" : "File Explorer";
export const trashName = isMac ? "Trash" : "Recycle Bin";
