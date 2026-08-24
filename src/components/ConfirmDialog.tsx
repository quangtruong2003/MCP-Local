import { useEffect, useRef } from "react";
import { AlertTriangle, XOctagon } from "lucide-react";

interface Props {
  open: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: "danger" | "warning";
  onConfirm: () => void;
  onCancel: () => void;
}

export default function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  tone = "danger",
  onConfirm,
  onCancel,
}: Props) {
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    // Focus the cancel button (the safe action) when the dialog opens.
    cancelRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onCancel]);

  if (!open) return null;

  const ToneIcon = tone === "warning" ? AlertTriangle : XOctagon;

  return (
    <div
      className="dialog-backdrop"
      role="presentation"
      onClick={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <div className="dialog-card" role="alertdialog" aria-modal="true" aria-labelledby="confirm-dialog-title">
        <div className="dialog-title-row">
          <span className={`dialog-icon dialog-icon-${tone}`} aria-hidden="true">
            <ToneIcon />
          </span>
          <h2 id="confirm-dialog-title">{title}</h2>
        </div>
        <p className="dialog-message">{message}</p>
        <div className="dialog-actions">
          <button ref={cancelRef} className="btn btn-secondary" onClick={onCancel}>
            {cancelLabel}
          </button>
          <button className={`btn ${tone === "warning" ? "btn-primary" : "btn-danger"}`} onClick={onConfirm}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
