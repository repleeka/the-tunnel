import { useCallback, useEffect, useState } from "react";
import { api, Device, formatBytes, Listing } from "../api";
import { Icon } from "./Icon";
import { Spinner } from "./ui";

export function RemoteBrowser({ device, saveTo, onPulled, onError }: { device: Device; saveTo: string; onPulled: () => void; onError: (m: string) => void }) {
  const saveName = saveTo.split(/[\\/]/).filter(Boolean).pop() ?? "this device";
  const [path, setPath] = useState("");
  const [listing, setListing] = useState<Listing | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [pulling, setPulling] = useState(false);

  const load = useCallback(
    async (p: string) => {
      setLoading(true);
      setError(null);
      try {
        const l = await api.list(device.id, p);
        setListing(l);
        setPath(p);
        setSelected(new Set());
      } catch (e) {
        setError(String(e));
      } finally {
        setLoading(false);
      }
    },
    [device.id],
  );

  useEffect(() => {
    load("");
  }, [load]);

  const crumbs = path ? path.split("/") : [];

  function toggle(p: string) {
    setSelected((s) => {
      const next = new Set(s);
      next.has(p) ? next.delete(p) : next.add(p);
      return next;
    });
  }

  async function pull(paths: string[]) {
    setPulling(true);
    try {
      await api.pull(device.id, paths);
      setSelected(new Set());
      onPulled();
    } catch (e) {
      onError(String(e));
    } finally {
      setPulling(false);
    }
  }

  return (
    <div className="browser">
      <div className="browser-bar">
        <div className="crumbs">
          <button className="crumb" onClick={() => load("")}>
            {listing?.root || device.deviceName}
          </button>
          {crumbs.map((c, i) => (
            <span key={i} className="crumb-wrap">
              <Icon name="chevron" size={14} />
              <button className="crumb" onClick={() => load(crumbs.slice(0, i + 1).join("/"))}>
                {c}
              </button>
            </span>
          ))}
        </div>
        <div className="browser-actions">
          <button className="icon-btn" onClick={() => load(path)} title="Refresh" aria-label="Refresh">
            <Icon name="refresh" size={16} />
          </button>
          <button className="btn primary small" disabled={selected.size === 0 || pulling} onClick={() => pull([...selected])}>
            {pulling ? <Spinner size={14} /> : <Icon name="down" size={16} />}
            {selected.size > 1 ? `Copy ${selected.size} items to ${saveName}` : `Copy to ${saveName}`}
          </button>
        </div>
      </div>

      <div className="browser-list">
        {loading && (
          <div className="browser-state">
            <Spinner />
          </div>
        )}
        {!loading && error && (
          <div className="browser-state">
            <p className="error-text">{error}</p>
            <button className="btn" onClick={() => load(path)}>
              Try again
            </button>
          </div>
        )}
        {!loading && !error && listing && listing.entries.length === 0 && (
          <div className="browser-state muted">
            {path ? "This folder is empty." : `Nothing to show on ${device.deviceName}.`}
          </div>
        )}
        {!loading &&
          !error &&
          listing?.entries.map((e) => (
            <div key={e.path} className={`entry ${selected.has(e.path) ? "selected" : ""}`} onClick={() => toggle(e.path)}>
              <input type="checkbox" checked={selected.has(e.path)} onChange={() => toggle(e.path)} onClick={(ev) => ev.stopPropagation()} />
              <span className={`entry-icon ${e.dir ? "dir" : ""} ${e.kind ? "place" : ""}`}>
                <Icon name={e.kind === "home" ? "home" : e.kind === "drive" ? "drive" : e.dir ? "folder" : "file"} />
              </span>
              {e.dir ? (
                <button
                  className="entry-name link"
                  onClick={(ev) => {
                    ev.stopPropagation();
                    load(e.path);
                  }}
                >
                  {e.name}
                </button>
              ) : (
                <span className="entry-name">{e.name}</span>
              )}
              <span className="entry-meta">{e.kind === "drive" ? "Drive" : e.kind === "home" ? "" : e.dir ? "Folder" : formatBytes(e.size)}</span>
              <span className="entry-meta date">{e.modified ? new Date(e.modified * 1000).toLocaleDateString() : ""}</span>
            </div>
          ))}
      </div>
    </div>
  );
}
