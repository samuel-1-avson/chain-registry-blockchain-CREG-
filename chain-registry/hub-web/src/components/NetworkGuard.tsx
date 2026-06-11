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
    <div className="hub-network-guard" data-tone="warn" role="alert">
      <div>
        <strong>Wrong network</strong>
        <p style={{ margin: "0.25rem 0 0", fontSize: "0.875rem", opacity: 0.9 }}>
          CREG testnet uses Sepolia (chain id {SEPOLIA_CHAIN_ID}
          {chainId != null ? `; your wallet reports ${chainId}` : ""}). Switch
          your wallet to continue with on-chain steps.
        </p>
        {isWalletConnect && (
          <p style={{ margin: "0.5rem 0 0", fontSize: "0.8125rem", opacity: 0.85 }}>
            Switch network in your mobile wallet app.
          </p>
        )}
      </div>
      {!isWalletConnect && (
        <button
          type="button"
          disabled={isSwitching}
          onClick={() => void switchToSepolia()}
        >
          {isSwitching ? "Switching…" : "Switch to Sepolia"}
        </button>
      )}
    </div>
  );
}
