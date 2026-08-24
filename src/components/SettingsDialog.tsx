interface Props {
  onClose: () => void;
  onError: (message: string | null) => void;
}

/** @deprecated Settings are now a full page in the control-center shell. */
export default function SettingsDialog({ onClose }: Props) {
  return (
    <div className="legacy-dialog-backdrop" onClick={onClose}>
      <div className="legacy-dialog" role="dialog" onClick={(event) => event.stopPropagation()}>
        <strong>Settings moved</strong>
        <p>Connection settings are now managed from the Settings page.</p>
        <button className="btn btn-primary" onClick={onClose}>Close</button>
      </div>
    </div>
  );
}
