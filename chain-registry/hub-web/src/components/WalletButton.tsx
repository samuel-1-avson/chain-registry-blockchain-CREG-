import { useState } from "react";
import { useWallet, walletConnectEnabled } from "../wallet/WalletProvider";

export function WalletButton() {
  const {
    address,
    connected,
    label,
    disconnect,
    connectInjected,
    connectWalletConnect,
    isPending,
    error,
  } = useWallet();
  const [open, setOpen] = useState(false);

  if (connected && address) {
    return (
      <div style={styles.group}>
        <span style={styles.address} title={address}>
          {shortAddress(address)}
          {label ? ` · ${label}` : ""}
        </span>
        <button type="button" style={styles.secondary} onClick={() => void disconnect()}>
          Disconnect
        </button>
      </div>
    );
  }

  return (
    <div style={styles.wrapper}>
      <button
        type="button"
        style={styles.primary}
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        Connect wallet
      </button>
      {open && (
        <div style={styles.menu} role="menu">
          <button
            type="button"
            style={styles.menuItem}
            disabled={isPending}
            onClick={() => {
              void connectInjected();
              setOpen(false);
            }}
          >
            Browser wallet
          </button>
          {walletConnectEnabled && (
            <button
              type="button"
              style={styles.menuItem}
              disabled={isPending}
              onClick={() => {
                void connectWalletConnect();
                setOpen(false);
              }}
            >
              WalletConnect
            </button>
          )}
          {!walletConnectEnabled && (
            <p style={styles.hint}>
              WalletConnect disabled — set VITE_WALLETCONNECT_PROJECT_ID for QR /
              mobile.
            </p>
          )}
          {error && <p style={styles.error}>{error}</p>}
        </div>
      )}
    </div>
  );
}

function shortAddress(address: string): string {
  return `${address.slice(0, 6)}…${address.slice(-4)}`;
}

const styles = {
  wrapper: {
    position: "relative" as const,
  },
  group: {
    display: "flex",
    alignItems: "center",
    gap: "var(--space-2)",
    flexWrap: "wrap" as const,
  },
  address: {
    fontFamily: "var(--font-mono)",
    fontSize: "0.85rem",
    color: "var(--text-secondary)",
  },
  primary: {
    border: "none",
    borderRadius: "var(--radius-sm)",
    padding: "0.45rem 0.85rem",
    background: "var(--accent-primary)",
    color: "#fff",
    fontWeight: 600,
    cursor: "pointer",
  },
  secondary: {
    border: "1px solid var(--border)",
    borderRadius: "var(--radius-sm)",
    padding: "0.45rem 0.75rem",
    background: "transparent",
    color: "var(--text-secondary)",
    cursor: "pointer",
  },
  menu: {
    position: "absolute" as const,
    right: 0,
    top: "calc(100% + 6px)",
    minWidth: "12rem",
    padding: "0.5rem",
    borderRadius: "var(--radius-md)",
    border: "1px solid var(--border)",
    background: "var(--bg-elevated)",
    boxShadow: "0 8px 24px rgba(0,0,0,0.45)",
    zIndex: 200,
  },
  menuItem: {
    display: "block",
    width: "100%",
    textAlign: "left" as const,
    border: "none",
    borderRadius: "var(--radius-sm)",
    padding: "0.5rem 0.65rem",
    background: "transparent",
    color: "var(--text-primary)",
    cursor: "pointer",
  },
  hint: {
    margin: "0.35rem 0.25rem",
    fontSize: "0.8rem",
    color: "var(--text-tertiary)",
  },
  error: {
    margin: "0.35rem 0.25rem",
    fontSize: "0.8rem",
    color: "var(--accent-error)",
  },
};
