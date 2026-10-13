import { FormEvent, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, BrowseScope, Device, formatBytes, Snapshot } from "../api";
import { DeviceIcon, Icon } from "./Icon";
import { Modal, Spinner } from "./ui";

export type PairingState = { sessionId: string; device: Device; code: string; incoming: boolean; confirmed: boolean };

export function PairingDialog({ pairing, onRespond }: { pairing: PairingState; onRespond: (accept: boolean) => void }) {
  const { device, code, incoming, confirmed } = pairing;
  return (
    <Modal width={400}>
      <div className="pair-dialog">
        <div className="big-icon accent">
          <DeviceIcon os={device.os} size={28} />
        </div>
        <h2>{incoming ? `${device.deviceName} wants to pair` : `Pairing with ${device.deviceName}`}</h2>
        <p className="muted">Make sure this code matches the one shown on {device.deviceName}.</p>
        <div className="pair-code" aria-label={`Pairing code ${code}`}>
          {code.split("").map((c, i) => (c === " " ? <span key={i} className="gap" /> : <span key={i}>{c}</span>))}
        </div>
        {confirmed ? (
          <div className="waiting">
            <Spinner /> Waiting for {device.deviceName} to confirm…
            <button className="link-btn" onClick={() => onRespond(false)}>
              Cancel
            </button>
          </div>
        ) : (
          <div className="row gap stretch">
            <button className="btn" onClick={() => onRespond(false)}>
              Doesn’t match
            </button>
            <button className="btn primary" onClick={() => onRespond(true)}>
              Codes match
            </button>
          </div>
        )}
      </div>
    </Modal>
  );
}

export type Offer = { transferId: string; deviceName: string; label: string; fileCount: number; totalBytes: number };

export function OfferDialog({ offer, onRespond }: { offer: Offer; onRespond: (accept: boolean) => void }) {
  return (
    <Modal width={400}>
      <div className="pair-dialog">
        <div className="big-icon accent">
          <Icon name="down" size={28} />
        </div>
        <h2>{offer.deviceName} wants to send you files</h2>
        <p>
          <strong>{offer.label}</strong>
        </p>
        <p className="muted">
          {offer.fileCount} {offer.fileCount === 1 ? "file" : "files"} · {formatBytes(offer.totalBytes)}
        </p>
        <div className="row gap stretch">
          <button className="btn" onClick={() => onRespond(false)}>
            Decline
          </button>
          <button className="btn primary" onClick={() => onRespond(true)}>
            Accept
          </button>
        </div>
      </div>
    </Modal>
  );
}

export function ConnectDialog({ addresses, onClose, onConnected }: { addresses: string[]; onClose: () => void; onConnected: (d: Device) => void }) {
  const [address, setAddress] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onConnected(await api.connectAddress(address));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title="Connect by address" onClose={onClose}>
      <form onSubmit={submit} className="stack">
        <p className="muted">
          Use this if your devices don’t find each other automatically. Open The Tunnel on the other device, go to Settings, and type the address it shows.
        </p>
        <label className="field">
          <span>Other device’s address</span>
          <input value={address} onChange={(e) => setAddress(e.target.value)} placeholder="e.g. 172.20.10.3" autoFocus />
        </label>
        {error && <p className="error-text">{error}</p>}
        <div className="row spread">
          <span className="muted small">This device: {addresses.length ? addresses.join(", ") : "no network"}</span>
          <button className="btn primary" disabled={busy || !address.trim()}>
            {busy && <Spinner size={14} />} Connect
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function SettingsDialog({ snap, onClose, onUnpair, onError }: { snap: Snapshot; onClose: () => void; onUnpair: (d: Device) => void; onError: (m: string) => void }) {
  const { settings, me } = snap;
  const [deviceName, setDeviceName] = useState(settings.deviceName);
  const [profileName, setProfileName] = useState(settings.profileName);
  const paired = snap.devices.filter((d) => d.paired);

  async function save(patch: Parameters<typeof api.updateSettings>[0]) {
    try {
      await api.updateSettings(patch);
    } catch (e) {
      onError(String(e));
    }
  }

  async function chooseFolder(key: "receiveDir" | "syncDir") {
    const picked = await open({ directory: true, defaultPath: settings[key] });
    if (typeof picked === "string") save({ [key]: picked });
  }

  return (
    <Modal title="Settings" onClose={onClose} width={520}>
      <div className="settings">
        <div className="settings-group">
          <h4>Profile</h4>
          <label className="field">
            <span>Your name</span>
            <input value={profileName} onChange={(e) => setProfileName(e.target.value)} onBlur={() => profileName !== settings.profileName && save({ profileName })} />
          </label>
          <label className="field">
            <span>This device’s name</span>
            <input
              value={deviceName}
              onChange={(e) => setDeviceName(e.target.value)}
              onBlur={() => deviceName.trim() && deviceName !== settings.deviceName && save({ deviceName })}
            />
          </label>
        </div>

        <div className="settings-group">
          <h4>Folders</h4>
          <div className="setting-row">
            <div>
              <div>Save received files to</div>
              <div className="muted small truncate">{settings.receiveDir}</div>
            </div>
            <button className="btn small" onClick={() => chooseFolder("receiveDir")}>
              Change
            </button>
          </div>
          <div className="setting-row">
            <div>
              <div>Sync folder</div>
              <div className="muted small truncate">{settings.syncDir}</div>
              <div className="muted small">Kept identical on your paired devices.</div>
            </div>
            <button className="btn small" onClick={() => chooseFolder("syncDir")}>
              Change
            </button>
          </div>
        </div>

        <div className="settings-group">
          <h4>Sync</h4>
          <label className="setting-row toggle">
            <div>
              <div>Keep The Tunnel folder in sync</div>
              <div className="muted small">New and changed files are copied to your other devices automatically. Deletions always ask first.</div>
            </div>
            <input type="checkbox" className="switch" checked={settings.syncEnabled} onChange={(e) => save({ syncEnabled: e.target.checked })} />
          </label>
        </div>

        <div className="settings-group">
          <h4>Browsing</h4>
          <div className="setting-stack">
            <div>Your paired devices can browse and copy</div>
            <div className="segmented" role="radiogroup">
              {(
                [
                  ["computer", "Whole computer"],
                  ["home", "Home folder"],
                  ["syncFolder", "Sync folder"],
                  ["nobody", "Nothing"],
                ] as [BrowseScope, string][]
              ).map(([value, label]) => (
                <button key={value} role="radio" aria-checked={settings.browseScope === value} className={settings.browseScope === value ? "active" : ""} onClick={() => save({ browseScope: value })}>
                  {label}
                </button>
              ))}
            </div>
            <div className="muted small">Copying is read-only: the other device can never change or delete anything here. Hidden and system folders (such as Library and AppData) are never shown.</div>
          </div>
        </div>

        <div className="settings-group">
          <h4>Receiving</h4>
          <label className="setting-row toggle">
            <div>
              <div>Ask before receiving files</div>
              <div className="muted small">If this is off, files from your paired devices arrive straight away.</div>
            </div>
            <input type="checkbox" className="switch" checked={settings.askBeforeReceiving} onChange={(e) => save({ askBeforeReceiving: e.target.checked })} />
          </label>
        </div>

        <div className="settings-group">
          <h4>Paired devices</h4>
          {paired.length === 0 && <p className="muted small">No paired devices yet.</p>}
          {paired.map((d) => (
            <div className="setting-row" key={d.id}>
              <div className="row gap-sm">
                <DeviceIcon os={d.os} size={16} />
                <span>{d.deviceName}</span>
              </div>
              <button className="btn small danger" onClick={() => onUnpair(d)}>
                Unpair
              </button>
            </div>
          ))}
        </div>

        <div className="settings-foot muted small">
          <Icon name="shield" size={14} />
          <span>
            Local network only · Encrypted · Address {me.addresses.join(", ") || "—"} (port {me.port}) · Device ID {me.deviceId.slice(0, 8).toUpperCase()}
          </span>
        </div>
      </div>
    </Modal>
  );
}
