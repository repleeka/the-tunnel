import { FormEvent, useState } from "react";
import { api, Settings } from "../api";
import { Icon } from "./Icon";

export function Onboarding({ settings, onDone }: { settings: Settings; onDone: () => void }) {
  const [profileName, setProfileName] = useState(settings.profileName);
  const [deviceName, setDeviceName] = useState(settings.deviceName);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      await api.updateSettings({ profileName, deviceName, onboarded: true });
      onDone();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  return (
    <div className="onboarding">
      <div className="drag-strip" data-tauri-drag-region />
      <form className="onboarding-card" onSubmit={submit}>
        <img src="/logo.png" alt="" className="onboarding-logo" />
        <h1>Welcome to The Tunnel</h1>
        <p className="muted">Move files between your Mac and PC over the same Wi-Fi or phone hotspot.</p>
        <div className="privacy-note">
          <Icon name="shield" />
          <span>Files go straight from one device to the other, encrypted. Nothing is sent to the internet, so it uses none of your mobile data.</span>
        </div>
        <label className="field">
          <span>Your name</span>
          <input value={profileName} onChange={(e) => setProfileName(e.target.value)} placeholder="e.g. Arun" autoFocus />
        </label>
        <label className="field">
          <span>Name for this device</span>
          <input value={deviceName} onChange={(e) => setDeviceName(e.target.value)} placeholder="e.g. Arun's MacBook" required />
          <small className="muted">Your other device will see this name.</small>
        </label>
        {error && <p className="error-text">{error}</p>}
        <button className="btn primary wide" disabled={busy || !deviceName.trim()}>
          Get started
        </button>
      </form>
    </div>
  );
}
