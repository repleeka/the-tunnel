import { ReactNode, useEffect } from "react";
import { Icon } from "./Icon";

export function Modal({ title, onClose, children, width = 440 }: { title?: string; onClose?: () => void; children: ReactNode; width?: number }) {
  useEffect(() => {
    if (!onClose) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose?.()}>
      <div className="modal" style={{ width }} role="dialog" aria-modal="true">
        {(title || onClose) && (
          <div className="modal-head">
            {title && <h2>{title}</h2>}
            {onClose && (
              <button className="icon-btn" onClick={onClose} aria-label="Close">
                <Icon name="x" />
              </button>
            )}
          </div>
        )}
        {children}
      </div>
    </div>
  );
}

export function Spinner({ size = 16 }: { size?: number }) {
  return <span className="spinner" style={{ width: size, height: size }} />;
}

export function Initials({ name }: { name: string }) {
  const letters =
    name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0]?.toUpperCase())
      .join("") || "?";
  return <span className="avatar">{letters}</span>;
}

export type Toast = { id: number; text: string; tone?: "info" | "error" | "success"; action?: { label: string; run: () => void } };

export function Toasts({ toasts, dismiss }: { toasts: Toast[]; dismiss: (id: number) => void }) {
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.tone ?? "info"}`}>
          <span>{t.text}</span>
          {t.action && (
            <button
              className="link-btn"
              onClick={() => {
                t.action!.run();
                dismiss(t.id);
              }}
            >
              {t.action.label}
            </button>
          )}
          <button className="icon-btn small" onClick={() => dismiss(t.id)} aria-label="Dismiss">
            <Icon name="x" size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
