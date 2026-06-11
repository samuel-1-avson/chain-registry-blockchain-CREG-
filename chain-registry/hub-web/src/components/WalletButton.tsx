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
      <div className="hub-wallet">
        <span className="hub-wallet-address" title={address}>
          {shortAddress(address)}
          {label ? ` · ${label}` : ""}
        </span>
        <button
          type="button"
          className="hub-wallet-disconnect"
          onClick={() => void disconnect()}
          disabled={isPending}
        >
          Disconnect
        </button>
      </div>
    );
  }

  return (
    <div className="hub-wallet-menu-wrap">
      <button
        type="button"
        className={`hub-wallet-connect${isPending ? " is-loading" : ""}`}
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-haspopup="menu"
        disabled={isPending}
      >
        {isPending ? "Connecting…" : "Connect wallet"}
      </button>
      {open && (
        <div className="hub-wallet-menu" role="menu">
          <button
            type="button"
            className="hub-wallet-menu-item"
            role="menuitem"
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
              className="hub-wallet-menu-item"
              role="menuitem"
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
            <p className="hub-wallet-hint">
              WalletConnect is disabled. Set VITE_WALLETCONNECT_PROJECT_ID for
              QR / mobile wallets.
            </p>
          )}
          {error && <p className="hub-wallet-error">{error}</p>}
        </div>
      )}
    </div>
  );
}

function shortAddress(address: string): string {
  return `${address.slice(0, 6)}…${address.slice(-4)}`;
}
