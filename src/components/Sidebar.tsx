import { Device, Me, osLabel, SyncStatus } from "../api";
import { DeviceIcon, Icon } from "./Icon";
import { Initials } from "./ui";

function statusText(d: Device, sync: SyncStatus) {
  if (!d.paired) return "Nearby · not paired";
  if (!d.online) return "Not on this network";
  const review = sync.pendingDeletions.filter((p) => p.deviceId === d.id).length;
  if (review > 0) return `${review} to review`;
  const peer = sync.peers.find((p) => p.deviceId === d.id);
  if (sync.enabled && peer?.state === "syncing" && peer.filesLeft > 0) return "Syncing…";
  return sync.enabled ? "Connected · in sync" : "Connected";
}

export function Sidebar({
  me,
  devices,
  sync,
  selectedId,
  onSelect,
  onConnect,
  onSettings,
}: {
  me: Me;
  devices: Device[];
  sync: SyncStatus;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onConnect: () => void;
  onSettings: () => void;
}) {
  return (
    <aside className="sidebar">
      <div className="drag-strip" data-tauri-drag-region />
      <div className="brand" data-tauri-drag-region>
        <img src="/logo.png" alt="" />
        <span>The Tunnel</span>
      </div>

      <div className="section-label">Devices</div>
      <nav className="device-list">
        {devices.length === 0 && (
          <div className="device-empty">
            <span className="pulse-dot" /> Searching…
          </div>
        )}
        {devices.map((d) => (
          <button key={d.id} className={`device-row ${selectedId === d.id ? "selected" : ""}`} onClick={() => onSelect(d.id)}>
            <span className="device-row-icon">
              <DeviceIcon os={d.os} />
            </span>
            <span className="device-row-text">
              <span className="device-row-name">{d.deviceName}</span>
              <span className="device-row-status">
                <span className={`dot ${d.paired ? (d.online ? "on" : "off") : "new"}`} />
                {statusText(d, sync)}
              </span>
            </span>
          </button>
        ))}
      </nav>

      <div className="sidebar-foot">
        <button className="ghost-btn" onClick={onConnect}>
          <Icon name="link" size={16} /> Connect by address
        </button>
        <button className="me-card" onClick={onSettings} title="Settings">
          <Initials name={me.profileName || me.deviceName} />
          <span className="me-text">
            <span className="me-name">{me.deviceName}</span>
            <span className="muted small">
              This {osLabel(me.os)}
              {me.profileName ? ` · ${me.profileName}` : ""}
            </span>
          </span>
          <Icon name="settings" size={16} />
        </button>
      </div>
    </aside>
  );
}
