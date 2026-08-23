interface Props {
  paused: boolean;
  onToggle: () => void;
}

export default function PauseButton({ paused, onToggle }: Props) {
  return (
    <button
      onClick={onToggle}
      style={{
        alignSelf: "center",
        padding: "12px 28px",
        fontSize: 16,
        fontWeight: 700,
        background: paused ? "var(--ok)" : "var(--danger)",
        color: "#fff",
        border: "none",
        borderRadius: 8,
        transition: "background 120ms ease",
      }}
    >
      {paused ? "▶ Resume access" : "⏸ Pause all access"}
    </button>
  );
}
