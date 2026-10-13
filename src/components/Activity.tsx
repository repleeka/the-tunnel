import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { fileManager, formatBytes, Transfer } from "../api";
import { Icon } from "./Icon";

function statusLine(t: Transfer) {
  const peer = t.direction === "send" ? `to ${t.deviceName}` : `from ${t.deviceName}`;
  switch (t.status) {
    case "waiting":
      return t.direction === "send" ? `Waiting for ${t.deviceName}…` : "Waiting for you to accept";
    case "active":
      return `${formatBytes(t.doneBytes)} of ${formatBytes(t.totalBytes)} · ${peer}`;
    case "done":
      return `${formatBytes(t.totalBytes)} · ${t.direction === "send" ? "Sent" : "Received"} ${peer}`;
    case "declined":
      return `Declined by ${t.deviceName}`;
    case "failed":
      return t.error ?? "Failed";
  }
}

export function Activity({ transfers, onClear }: { transfers: Transfer[]; onClear: () => void }) {
  if (transfers.length === 0) return null;
  const finished = transfers.some((t) => t.status !== "active" && t.status !== "waiting");

  return (
    <section className="activity">
      <div className="activity-head">
        <h3>Activity</h3>
        {finished && (
          <button className="link-btn" onClick={onClear}>
            Clear
          </button>
        )}
      </div>
      <ul>
        {transfers.map((t) => {
          const pct = t.totalBytes > 0 ? Math.min(100, (t.doneBytes / t.totalBytes) * 100) : t.status === "done" ? 100 : 0;
          return (
            <li key={t.id} className={`transfer ${t.status}`}>
              <span className={`transfer-icon ${t.direction}`}>
                <Icon name={t.status === "failed" || t.status === "declined" ? "x" : t.status === "done" ? "check" : t.direction === "send" ? "up" : "down"} size={16} />
              </span>
              <div className="transfer-body">
                <div className="transfer-title">
                  <span className="truncate">{t.label}</span>
                  {t.fileCount > 1 && <span className="muted small">{t.fileCount} files</span>}
                </div>
                <div className="transfer-status muted small truncate" title={t.error ?? undefined}>
                  {statusLine(t)}
                </div>
                {(t.status === "active" || t.status === "waiting") && (
                  <div className="progress">
                    <div style={{ width: `${pct}%` }} className={t.status === "waiting" ? "indeterminate" : ""} />
                  </div>
                )}
              </div>
              {t.status === "done" && t.location && (
                <button className="btn small" onClick={() => revealItemInDir(t.location!)}>
                  <Icon name="reveal" size={14} /> Show in {fileManager}
                </button>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
