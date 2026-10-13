import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, Device, fileManager, formatBytes, PendingDeletion, SyncChange, SyncStatus, timeAgo, trashName } from "../api";
import { Icon, IconName } from "./Icon";
import { Spinner } from "./ui";

type Props = { device: Device; sync: SyncStatus; onError: (m: string) => void };

export function SyncPane({ device, sync, onError }: Props) {
  const peer = sync.peers.find((p) => p.deviceId === device.id);
  const pending = sync.pendingDeletions.filter((p) => p.deviceId === device.id);
  const [syncing, setSyncing] = useState(false);

  async function changeFolder() {
    const picked = await open({ directory: true, defaultPath: sync.folder, title: "Choose the folder to keep in sync" });
    if (typeof picked !== "string" || picked === sync.folder) return;
    try {
      await api.updateSettings({ syncDir: picked });
    } catch (e) {
      onError(String(e));
    }
  }

  async function syncNow() {
    setSyncing(true);
    try {
      await api.syncNow();
    } catch (e) {
      onError(String(e));
    } finally {
      setSyncing(false);
    }
  }

  return (
    <div className="sync">
      <div className="sync-card">
        <div className="sync-folder">
          <div className="sync-folder-icon">
            <Icon name="folder" size={22} />
          </div>
          <div className="sync-folder-text">
            <div className="sync-folder-name">Sync folder</div>
            <div className="muted small truncate" title={sync.folder}>
              {sync.folder}
            </div>
          </div>
          <div className="row gap-sm">
            <button className="btn small" onClick={changeFolder}>
              Change…
            </button>
            <button className="btn small" onClick={() => revealItemInDir(sync.folder)}>
              <Icon name="reveal" size={14} /> Show in {fileManager}
            </button>
          </div>
        </div>
        <div className="sync-status">
          <StatusLine sync={sync} device={device} />
          {sync.enabled && device.online && (
            <button className="link-btn" onClick={syncNow} disabled={syncing}>
              {syncing ? "Syncing…" : "Sync now"}
            </button>
          )}
        </div>
      </div>

      {!sync.enabled && (
        <div className="notice">
          <span>Sync is off. Files in your sync folder won’t be kept up to date on {device.deviceName}.</span>
          <button className="btn small primary" onClick={() => api.updateSettings({ syncEnabled: true }).catch((e) => onError(String(e)))}>
            Turn on
          </button>
        </div>
      )}

      {pending.length > 0 && <DeletionReview device={device} pending={pending} onError={onError} />}

      {sync.issues.length > 0 && (
        <div className="sync-section">
          <h3>
            <Icon name="alert" size={15} /> Needs attention
          </h3>
          <ul className="issue-list">
            {sync.issues.slice(0, 8).map((issue, i) => (
              <li key={i}>{issue}</li>
            ))}
            {sync.issues.length > 8 && <li className="muted">…and {sync.issues.length - 8} more</li>}
          </ul>
        </div>
      )}

      <div className="sync-section">
        <h3>Recent changes</h3>
        {sync.recent.length === 0 ? (
          <p className="muted small empty-recent">
            Anything you add to your sync folder appears on {device.deviceName} automatically, and the other way round.{" "}
            {peer?.state === "syncing" ? "" : "Changes from either device will show up here."}
          </p>
        ) : (
          <ul className="change-list">
            {sync.recent.map((c, i) => (
              <ChangeRow key={`${c.at}-${c.path}-${i}`} change={c} />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function StatusLine({ sync, device }: { sync: SyncStatus; device: Device }) {
  const peer = sync.peers.find((p) => p.deviceId === device.id);
  if (!sync.enabled) return <span className="status muted">Sync is off</span>;
  if (!device.online)
    return (
      <span className="status muted">
        <span className="dot off" />
        Waiting for {device.deviceName}. Changes will sync when it’s back on this network.
      </span>
    );
  if (sync.scanning || peer?.state === "syncing") {
    return (
      <span className="status">
        <Spinner size={13} />
        {peer && peer.filesLeft > 0 ? `Syncing · ${peer.filesLeft} ${peer.filesLeft === 1 ? "file" : "files"} left (${formatBytes(peer.bytesLeft)})` : "Checking for changes…"}
      </span>
    );
  }
  if (peer?.state === "error")
    return (
      <span className="status error-text">
        <Icon name="alert" size={14} /> {peer.error}
      </span>
    );
  return (
    <span className="status">
      <span className="dot on" />
      Up to date{peer?.lastSynced ? ` · checked ${timeAgo(peer.lastSynced)}` : ""}
    </span>
  );
}

function DeletionReview({ device, pending, onError }: { device: Device; pending: PendingDeletion[]; onError: (m: string) => void }) {
  const [busy, setBusy] = useState(false);

  async function resolve(paths: string[], del: boolean) {
    setBusy(true);
    try {
      await api.resolveDeletions(device.id, paths, del);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const all = pending.map((p) => p.path);
  return (
    <div className="sync-section review">
      <div className="review-head">
        <div>
          <h3>
            <Icon name="trash" size={15} /> Deleted on {device.deviceName}
          </h3>
          <p className="muted small">
            {pending.length === 1 ? "This file was" : `These ${pending.length} files were`} deleted on {device.deviceName}. Nothing has been deleted here yet.
          </p>
        </div>
        {pending.length > 1 && (
          <div className="row gap-sm">
            <button className="btn small" disabled={busy} onClick={() => resolve(all, false)}>
              Keep all
            </button>
            <button className="btn small danger" disabled={busy} onClick={() => resolve(all, true)}>
              Delete all
            </button>
          </div>
        )}
      </div>
      <ul className="review-list">
        {pending.map((p) => (
          <li key={p.path}>
            <span className="entry-icon">
              <Icon name="file" size={16} />
            </span>
            <span className="truncate review-path" title={p.path}>
              {p.path}
            </span>
            <span className="muted small">{formatBytes(p.size)}</span>
            <button className="btn small" disabled={busy} onClick={() => resolve([p.path], false)} title={`Keep it and restore it on ${device.deviceName}`}>
              Keep
            </button>
            <button className="btn small danger" disabled={busy} onClick={() => resolve([p.path], true)} title={`Move it to the ${trashName}`}>
              Delete
            </button>
          </li>
        ))}
      </ul>
      <p className="muted small">
        <strong>Delete</strong> moves the file to the {trashName}. <strong>Keep</strong> keeps it here and puts it back on {device.deviceName}.
      </p>
    </div>
  );
}

const changeInfo: Record<SyncChange["kind"], { icon: IconName; text: (c: SyncChange) => string }> = {
  added: { icon: "plus", text: (c) => `Added from ${c.deviceName}` },
  updated: { icon: "refresh", text: (c) => `Updated from ${c.deviceName}` },
  renamed: { icon: "edit", text: (c) => `Renamed on ${c.deviceName}${c.from ? ` (was ${c.from})` : ""}` },
  deleted: { icon: "trash", text: (c) => `Deleted (as on ${c.deviceName})` },
  kept: { icon: "check", text: (c) => `Kept, and restored on ${c.deviceName}` },
  conflict: { icon: "copy", text: (c) => `Edited on both devices. ${c.deviceName}’s version kept the name, and yours was saved as a conflict copy` },
};

function ChangeRow({ change }: { change: SyncChange }) {
  const info = changeInfo[change.kind];
  return (
    <li className={`change ${change.kind}`}>
      <span className="change-icon">
        <Icon name={info.icon} size={14} />
      </span>
      <div className="change-body">
        <div className="truncate" title={change.path}>
          {change.path}
        </div>
        <div className="muted small">
          {info.text(change)} · {timeAgo(change.at)}
        </div>
      </div>
    </li>
  );
}
