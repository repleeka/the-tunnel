import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Device, Me, osLabel, SyncStatus } from "../api";
import { DeviceIcon, Icon } from "./Icon";
import { RemoteBrowser } from "./RemoteBrowser";
import { SyncPane } from "./SyncPane";

export type Tab = "sync" | "send" | "browse";

type Props = {
  device: Device;
  me: Me;
  sync: SyncStatus;
  saveTo: string;
  tab: Tab;
  onTab: (t: Tab) => void;
  dragging: boolean;
  onSend: (paths: string[]) => void;
  onPair: () => void;
  onUnpair: () => void;
  onConnect: () => void;
  onError: (m: string) => void;
  onPulled: () => void;
};

export function DevicePanel(p: Props) {
  const { device } = p;
  const { tab, onTab: setTab } = p;
  const [menu, setMenu] = useState(false);
  const reviewCount = p.sync.pendingDeletions.filter((d) => d.deviceId === device.id).length;

  const status = !device.paired ? "Nearby · not paired" : device.online ? "Connected" : "Not on this network";

  return (
    <section className="panel">
      <header className="device-header">
        <div className="device-hero-icon">
          <DeviceIcon os={device.os} size={26} />
        </div>
        <div className="device-header-text">
          <h1>{device.deviceName}</h1>
          <p className="muted">
            <span className={`dot ${device.paired ? (device.online ? "on" : "off") : "new"}`} />
            {status}
            {device.profileName ? ` · ${device.profileName}'s ${osLabel(device.os)}` : ""}
          </p>
        </div>
        {device.paired && (
          <div className="menu-wrap">
            <button className="icon-btn" onClick={() => setMenu((m) => !m)} aria-label="More">
              <Icon name="more" size={20} strokeWidth={3} />
            </button>
            {menu && (
              <div className="menu" onMouseLeave={() => setMenu(false)}>
                <button
                  className="menu-item danger"
                  onClick={() => {
                    setMenu(false);
                    p.onUnpair();
                  }}
                >
                  Unpair this device
                </button>
              </div>
            )}
          </div>
        )}
      </header>

      {!device.paired && <PairPrompt device={device} onPair={p.onPair} />}
      {device.paired && !device.online && tab !== "sync" && <OfflineHint device={device} me={p.me} onConnect={p.onConnect} />}
      {device.paired && (
        <>
          <div className="tabs" role="tablist">
            <button role="tab" className={tab === "sync" ? "active" : ""} onClick={() => setTab("sync")}>
              <Icon name="refresh" size={15} /> Sync
              {reviewCount > 0 && <span className="badge">{reviewCount}</span>}
            </button>
            <button role="tab" className={tab === "send" ? "active" : ""} onClick={() => setTab("send")}>
              <Icon name="up" size={15} /> Send
            </button>
            <button role="tab" className={tab === "browse" ? "active" : ""} onClick={() => setTab("browse")}>
              <Icon name="down" size={15} /> Browse {device.deviceName}
            </button>
          </div>
          {tab === "sync" && <SyncPane device={device} sync={p.sync} onError={p.onError} />}
          {device.online && tab === "send" && <DropZone device={device} dragging={p.dragging} onSend={p.onSend} />}
          {device.online && tab === "browse" && <RemoteBrowser key={device.id} device={device} saveTo={p.saveTo} onError={p.onError} onPulled={p.onPulled} />}
        </>
      )}
    </section>
  );
}

function DropZone({ device, dragging, onSend }: { device: Device; dragging: boolean; onSend: (paths: string[]) => void }) {
  async function choose(directory: boolean) {
    const picked = await open({ multiple: true, directory, title: `Send to ${device.deviceName}` });
    if (!picked) return;
    onSend(Array.isArray(picked) ? picked : [picked]);
  }

  return (
    <div className={`dropzone ${dragging ? "over" : ""}`}>
      <div className="dropzone-icon">
        <Icon name="up" size={26} />
      </div>
      <h2>{dragging ? `Release to send to ${device.deviceName}` : "Drop files or folders here"}</h2>
      <p className="muted">They’ll be sent straight to {device.deviceName}, encrypted.</p>
      <div className="row gap">
        <button className="btn" onClick={() => choose(false)}>
          <Icon name="file" size={16} /> Choose files
        </button>
        <button className="btn" onClick={() => choose(true)}>
          <Icon name="folder" size={16} /> Choose folder
        </button>
      </div>
    </div>
  );
}

function PairPrompt({ device, onPair }: { device: Device; onPair: () => void }) {
  return (
    <div className="center-card">
      <div className="big-icon accent">
        <Icon name="shield" size={30} />
      </div>
      <h2>Pair with {device.deviceName}?</h2>
      <p className="muted">
        You’ll both see a 6-digit code. If the codes match, the two devices will trust each other. Nobody else on the network can send you files or see
        your shared folder.
      </p>
      <button className="btn primary" onClick={onPair}>
        Pair
      </button>
    </div>
  );
}

function OfflineHint({ device, me, onConnect }: { device: Device; me: Me; onConnect: () => void }) {
  return (
    <div className="center-card">
      <div className="big-icon">
        <Icon name="wifi" size={30} />
      </div>
      <h2>Waiting for {device.deviceName}</h2>
      <ul className="tips">
        <li>Connect both devices to the same hotspot or Wi-Fi.</li>
        <li>Open The Tunnel on {device.deviceName}.</li>
        <li>On Windows, allow The Tunnel through the firewall when asked.</li>
      </ul>
      <p className="muted small">
        Still not showing? Some phone hotspots hide devices from each other. Try{" "}
        <button className="link-btn" onClick={onConnect}>
          connecting by address
        </button>
        {me.addresses.length > 0 && <> — this device is at {me.addresses.join(", ")}</>}.
      </p>
    </div>
  );
}

export function Searching({ me, onConnect }: { me: Me; onConnect: () => void }) {
  return (
    <section className="panel">
      <div className="center-card tall">
        <div className="radar">
          <span />
          <span />
          <span />
          <img src="/logo.png" alt="" />
        </div>
        <h2>Looking for your other device…</h2>
        <ul className="tips">
          <li>Connect both devices to the same hotspot or Wi-Fi.</li>
          <li>Open The Tunnel on the other device.</li>
        </ul>
        <p className="muted small">
          Can’t find it?{" "}
          <button className="link-btn" onClick={onConnect}>
            Connect by address
          </button>
          {me.addresses.length > 0 && <> · This device is at {me.addresses.join(", ")}</>}
        </p>
      </div>
    </section>
  );
}

