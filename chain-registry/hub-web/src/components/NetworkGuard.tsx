import { SEPOLIA_CHAIN_ID } from "../config/links";
import { useWallet } from "../wallet/WalletProvider";

export function NetworkGuard() {
  const { connected, onSepolia, chainId, switchToSepolia, isSwitching, session } =
    useWallet();

  if (!connected || onSepolia) {
    return null;
  }

  const isWalletConnect = session?.source === "walletconnect";

  return (
    <div style={styles.banner} role="alert">
      <div>
        <strong>Wrong network</strong>
        <p style={styles.text}>
          CREG testnet uses Sepolia (chain id {SEPOLIA_CHAIN_ID}
          {chainId != null ? `; your wallet reports ${chainId}` : ""}). Switch
          your wallet to continue with on-chain steps.
        </p>
      </div>
      {!isWalletConnect && (
        <button
          type="button"
          style={styles.button}
          disabled={isSwitching}
          onClick={() => void switchToSepolia()}
        >
          {isSwitching ? "Switching…" : "Switch to Sepolia"}
        </button>
      )}
      {isWalletConnect && (
        <p style={styles.hint}>Switch network in your mobile wallet app.</p>
      )}
    </div>
  );
}

const styles = {
  banner: {
    display: "flex",
    flexWrap: "wrap" as const,
    alignItems: "center",
    justifyContent: "space-between",
    gap: "var(--space-4)",
    padding: "0.85rem 1rem",
    marginBottom: "var(--space-6)",
    borderRadius: "var(--radius-md)",
    border: "1px solid rgba(245, 158, 11, 0.35)",
    background: "rgba(245, 158, 11, 0.08)",
  },
  text: {
    margin: "0.25rem 0 0",
    color: "var(--text-secondary)",
    fontSize: "0.9rem",
  },
  hint: {
    margin: 0,
    color: "var(--text-secondary)",
    fontSize: "0.85rem",
  },
  button: {
    border: "none",
    borderRadius: "var(--radius-sm)",
    padding: "0.5rem 0.9rem",
    background: "var(--accent-warning)",
    color: "#1a1200",
    fontWeight: 600,
    cursor: "pointer",
  },
};
