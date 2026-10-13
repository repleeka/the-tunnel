import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, Device, onTunnelEvent, Snapshot, Transfer } from "./api";
import { Activity } from "./components/Activity";
import { DevicePanel, Searching, Tab } from "./components/DevicePanel";
import { ConnectDialog, Offer, OfferDialog, PairingDialog, PairingState, SettingsDialog } from "./components/Dialogs";
import { Onboarding } from "./components/Onboarding";
import { Sidebar } from "./components/Sidebar";
import { Toast, Toasts } from "./components/ui";

export default function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pairing, setPairing] = useState<PairingState | null>(null);
  const [offers, setOffers] = useState<Offer[]>([]);
  const [dialog, setDialog] = useState<"settings" | "connect" | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [dragging, setDragging] = useState(false);
  const [dismissed, setDismissed] = useState<Set<string>>(new Set());
  const [tab, setTab] = useState<Tab>("sync");

  const snapRef = useRef(snap);
  snapRef.current = snap;
  const selectedRef = useRef(selectedId);
  selectedRef.current = selectedId;
  const toastId = useRef(0);
  const announcedOnline = useRef<Set<string> | null>(null);

  const toast = useCallback((t: Omit<Toast, "id">) => {
    const id = ++toastId.current;
    setToasts((ts) => [...ts.slice(-3), { ...t, id }]);
    setTimeout(() => setToasts((ts) => ts.filter((x) => x.id !== id)), t.action ? 9000 : 5000);
  }, []);
  const dismissToast = (id: number) => setToasts((ts) => ts.filter((t) => t.id !== id));
  const showError = useCallback((text: string) => toast({ text, tone: "error" }), [toast]);

  const refresh = useCallback(async () => setSnap(await api.state()), []);

  // Engine events.
  useEffect(() => {
    refresh();
    const unlisten = onTunnelEvent((e) => {
      switch (e.kind) {
        case "devicesChanged":
        case "settingsChanged":
        case "syncChanged":
          refresh();
          break;
        case "pairingCode":
          setPairing({ sessionId: e.sessionId, device: e.device, code: e.code, incoming: e.incoming, confirmed: false });
          setSelectedId(e.device.id);
          break;
        case "pairingFinished":
          setPairing((p) => (p && p.sessionId === e.sessionId ? null : p));
          if (e.success) {
            toast({ text: "Paired. You can now send files both ways.", tone: "success" });
            setSelectedId(e.deviceId);
          } else if (e.message) {
            toast({ text: e.message, tone: "error" });
          }
          refresh();
          break;
        case "incomingOffer":
          setOffers((o) => [...o, e]);
          break;
        case "transferChanged":
          setSnap((s) => (s ? { ...s, transfers: upsert(s.transfers, e.transfer) } : s));
          notifyTransfer(e.transfer, toast);
          break;
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [refresh, toast]);

  // Drag files from Finder / File Explorer onto the window.
  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === "enter" || p.type === "over") setDragging(true);
      else if (p.type === "leave") setDragging(false);
      else if (p.type === "drop") {
        setDragging(false);
        const device = snapRef.current?.devices.find((d) => d.id === selectedRef.current);
        if (!device?.paired || !device.online) {
          showError("Choose a connected device first, then drop your files.");
          return;
        }
        send(device, p.paths);
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [showError]);

  const devices = snap?.devices ?? [];

  // Keep a sensible device selected.
  useEffect(() => {
    if (!snap) return;
    if (!selectedId || !devices.some((d) => d.id === selectedId)) {
      setSelectedId(devices[0]?.id ?? null);
    }
  }, [snap, selectedId, devices]);

  // Tell the user when a paired device comes onto the network.
  useEffect(() => {
    if (!snap) return;
    const online = new Set(devices.filter((d) => d.paired && d.online).map((d) => d.id));
    if (announcedOnline.current) {
      for (const d of devices) {
        if (online.has(d.id) && !announcedOnline.current.has(d.id)) toast({ text: `${d.deviceName} is connected.`, tone: "success" });
      }
    }
    announcedOnline.current = online;
  }, [snap, devices, toast]);

  async function send(device: Device, paths: string[]) {
    if (paths.length === 0) return;
    try {
      await api.send(device.id, paths);
    } catch (e) {
      showError(String(e));
    }
  }

  async function startPair(device: Device) {
    try {
      await api.pair(device.id);
    } catch (e) {
      showError(String(e));
    }
  }

  async function respondPairing(accept: boolean) {
    if (!pairing) return;
    await api.respondPairing(pairing.sessionId, accept);
    setPairing(accept ? { ...pairing, confirmed: true } : null);
  }

  async function respondOffer(accept: boolean) {
    const [offer, ...rest] = offers;
    setOffers(rest);
    await api.respondOffer(offer.transferId, accept);
  }

  async function unpair(device: Device) {
    if (!window.confirm(`Unpair ${device.deviceName}? You'll need to pair again before sending files.`)) return;
    await api.unpair(device.id);
  }

  if (!snap) return <div className="app loading" />;
  if (!snap.settings.onboarded) return <Onboarding settings={snap.settings} onDone={refresh} />;

  const selected = devices.find((d) => d.id === selectedId) ?? null;
  const nearbyNew = devices.find((d) => !d.paired && d.online && !dismissed.has(d.id) && d.id !== selectedId);
  const review = snap.sync.pendingDeletions;
  const reviewDevice = review.length > 0 ? devices.find((d) => d.id === review[0].deviceId) : undefined;
  const showReviewBanner = reviewDevice && !(selectedId === reviewDevice.id && tab === "sync");

  return (
    <div className="app">
      <Sidebar
        me={snap.me}
        devices={devices}
        sync={snap.sync}
        selectedId={selectedId}
        onSelect={setSelectedId}
        onConnect={() => setDialog("connect")}
        onSettings={() => setDialog("settings")}
      />
      <main className="main">
        <div className="drag-strip" data-tauri-drag-region />
        {nearbyNew && (
          <div className="banner">
            <span>
              <strong>{nearbyNew.deviceName}</strong> is on this network. Connect to it?
            </span>
            <div className="row gap-sm">
              <button className="btn small" onClick={() => setDismissed((s) => new Set(s).add(nearbyNew.id))}>
                Not now
              </button>
              <button
                className="btn small primary"
                onClick={() => {
                  setSelectedId(nearbyNew.id);
                  startPair(nearbyNew);
                }}
              >
                Pair
              </button>
            </div>
          </div>
        )}
        {showReviewBanner && (
          <div className="banner warn">
            <span>
              {review.length === 1 ? "A file was" : `${review.length} files were`} deleted on <strong>{reviewDevice.deviceName}</strong>. Review before anything is deleted here.
            </span>
            <button
              className="btn small primary"
              onClick={() => {
                setSelectedId(reviewDevice.id);
                setTab("sync");
              }}
            >
              Review
            </button>
          </div>
        )}
        <div className="main-scroll">
          {selected ? (
            <DevicePanel
              key={selected.id}
              device={selected}
              me={snap.me}
              sync={snap.sync}
              saveTo={snap.settings.receiveDir}
              tab={tab}
              onTab={setTab}
              dragging={dragging}
              onSend={(paths) => send(selected, paths)}
              onPair={() => startPair(selected)}
              onUnpair={() => unpair(selected)}
              onConnect={() => setDialog("connect")}
              onError={showError}
              onPulled={() => toast({ text: `Copying from ${selected.deviceName}…` })}
            />
          ) : (
            <Searching me={snap.me} onConnect={() => setDialog("connect")} />
          )}
          <Activity
            transfers={snap.transfers}
            onClear={async () => {
              await api.clearTransfers();
              refresh();
            }}
          />
        </div>
      </main>

      {pairing && <PairingDialog pairing={pairing} onRespond={respondPairing} />}
      {!pairing && offers[0] && <OfferDialog offer={offers[0]} onRespond={respondOffer} />}
      {dialog === "connect" && (
        <ConnectDialog
          addresses={snap.me.addresses}
          onClose={() => setDialog(null)}
          onConnected={(d) => {
            setDialog(null);
            setSelectedId(d.id);
            refresh();
            toast({ text: `Found ${d.deviceName}.`, tone: "success" });
          }}
        />
      )}
      {dialog === "settings" && <SettingsDialog snap={snap} onClose={() => setDialog(null)} onUnpair={unpair} onError={showError} />}
      <Toasts toasts={toasts} dismiss={dismissToast} />
    </div>
  );
}

function upsert(list: Transfer[], t: Transfer): Transfer[] {
  const i = list.findIndex((x) => x.id === t.id);
  if (i === -1) return [t, ...list];
  const next = list.slice();
  next[i] = t;
  return next;
}

function notifyTransfer(t: Transfer, toast: (t: Omit<Toast, "id">) => void) {
  if (t.status === "done" && t.direction === "receive") toast({ text: `Received ${t.label} from ${t.deviceName}.`, tone: "success" });
  if (t.status === "failed") toast({ text: `${t.label}: ${t.error ?? "transfer failed"}`, tone: "error" });
}
