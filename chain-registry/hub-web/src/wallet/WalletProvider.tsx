import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { type EIP1193Provider, type Hex } from "viem";
import { sepolia } from "../config/chains";
import { SEPOLIA_CHAIN_ID } from "../config/links";

export const walletConnectProjectId =
  import.meta.env.VITE_WALLETCONNECT_PROJECT_ID?.trim() ?? "";

export const walletConnectEnabled = Boolean(walletConnectProjectId);

type WalletSource = "injected" | "walletconnect";

type WalletSession = {
  address: Hex;
  chainId: number;
  provider: EIP1193Provider;
  source: WalletSource;
  label: string;
};

type WalletContextValue = {
  session: WalletSession | null;
  connected: boolean;
  address: Hex | undefined;
  chainId: number | undefined;
  onSepolia: boolean;
  label: string | undefined;
  isPending: boolean;
  error: string | null;
  connectInjected: () => Promise<void>;
  connectWalletConnect: () => Promise<void>;
  disconnect: () => Promise<void>;
  switchToSepolia: () => Promise<void>;
  isSwitching: boolean;
};

const WalletContext = createContext<WalletContextValue | null>(null);

const SEPOLIA_HEX = `0x${SEPOLIA_CHAIN_ID.toString(16)}` as const;

async function readChainId(provider: EIP1193Provider): Promise<number> {
  const hex = (await provider.request({ method: "eth_chainId" })) as string;
  return Number.parseInt(hex, 16);
}

export function WalletProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<WalletSession | null>(null);
  const [isPending, setPending] = useState(false);
  const [isSwitching, setSwitching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const bindProvider = useCallback(
    async (
      provider: EIP1193Provider,
      source: WalletSource,
      label: string,
    ) => {
      const accounts = (await provider.request({
        method: "eth_requestAccounts",
      })) as string[];
      const address = accounts[0] as Hex | undefined;
      if (!address) {
        throw new Error("No accounts returned from wallet.");
      }
      const chainId = await readChainId(provider);
      setSession({ address, chainId, provider, source, label });
      setError(null);
    },
    [],
  );

  const connectInjected = useCallback(async () => {
    const ethereum = (
      window as Window & { ethereum?: EIP1193Provider }
    ).ethereum;
    if (!ethereum) {
      setError("No browser wallet extension detected.");
      return;
    }
    setPending(true);
    setError(null);
    try {
      await bindProvider(ethereum, "injected", "Browser wallet");
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Wallet connection failed.";
      if (!message.toLowerCase().includes("user rejected")) {
        setError(message);
      }
    } finally {
      setPending(false);
    }
  }, [bindProvider]);

  const connectWalletConnect = useCallback(async () => {
    if (!walletConnectEnabled) {
      setError("Set VITE_WALLETCONNECT_PROJECT_ID to enable WalletConnect.");
      return;
    }
    setPending(true);
    setError(null);
    try {
      const { EthereumProvider } = await import(
        "@walletconnect/ethereum-provider"
      );
      const provider = await EthereumProvider.init({
        projectId: walletConnectProjectId,
        chains: [sepolia.id],
        rpcMap: { [sepolia.id]: sepolia.rpcUrls.default.http[0] },
        showQrModal: true,
        metadata: {
          name: "CREG Testnet Hub",
          description: "Join the CREG Sepolia testnet",
          url: window.location.origin,
          icons: [],
        },
      });
      await provider.enable();
      const accounts = provider.accounts;
      const address = accounts?.[0] as Hex | undefined;
      if (!address) {
        throw new Error("No accounts returned from WalletConnect.");
      }
      const chainId = Number(provider.chainId) || sepolia.id;
      setSession({
        address,
        chainId,
        provider: provider as unknown as EIP1193Provider,
        source: "walletconnect",
        label: "WalletConnect",
      });
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "WalletConnect connection failed.";
      if (!message.toLowerCase().includes("user rejected")) {
        setError(message);
      }
    } finally {
      setPending(false);
    }
  }, []);

  const disconnect = useCallback(async () => {
    const provider = session?.provider as EIP1193Provider & {
      disconnect?: () => Promise<void>;
    };
    if (provider?.disconnect) {
      await provider.disconnect().catch(() => undefined);
    }
    setSession(null);
    setError(null);
  }, [session?.provider]);

  const switchToSepolia = useCallback(async () => {
    if (!session?.provider) {
      return;
    }
    setSwitching(true);
    try {
      await session.provider.request({
        method: "wallet_switchEthereumChain",
        params: [{ chainId: SEPOLIA_HEX }],
      });
      const chainId = await readChainId(session.provider);
      setSession((current) =>
        current ? { ...current, chainId } : current,
      );
    } catch (err) {
      const e = err as { code?: number };
      if (e.code === 4902) {
        await session.provider.request({
          method: "wallet_addEthereumChain",
          params: [
            {
              chainId: SEPOLIA_HEX,
              chainName: sepolia.name,
              nativeCurrency: sepolia.nativeCurrency,
              rpcUrls: sepolia.rpcUrls.default.http,
              blockExplorerUrls: [
                sepolia.blockExplorers?.default?.url,
              ].filter(Boolean),
            },
          ],
        });
        const chainId = await readChainId(session.provider);
        setSession((current) =>
          current ? { ...current, chainId } : current,
        );
      } else {
        throw err;
      }
    } finally {
      setSwitching(false);
    }
  }, [session?.provider]);

  useEffect(() => {
    if (!session?.provider) {
      return;
    }
    const provider = session.provider;
    const onAccounts = (accounts: unknown) => {
      const list = accounts as string[];
      if (!list?.length) {
        void disconnect();
        return;
      }
      setSession((current) =>
        current
          ? { ...current, address: list[0] as Hex }
          : current,
      );
    };
    const onChain = (hex: unknown) => {
      const chainId = Number.parseInt(String(hex), 16);
      if (Number.isFinite(chainId)) {
        setSession((current) =>
          current ? { ...current, chainId } : current,
        );
      }
    };
    const events = provider as EIP1193Provider & {
      on?: (event: string, listener: (...args: unknown[]) => void) => void;
      removeListener?: (
        event: string,
        listener: (...args: unknown[]) => void,
      ) => void;
    };
    events.on?.("accountsChanged", onAccounts);
    events.on?.("chainChanged", onChain);
    return () => {
      events.removeListener?.("accountsChanged", onAccounts);
      events.removeListener?.("chainChanged", onChain);
    };
  }, [session?.provider, disconnect]);

  const value = useMemo<WalletContextValue>(() => {
    const connected = Boolean(session);
    const chainId = session?.chainId;
    return {
      session,
      connected,
      address: session?.address,
      chainId,
      onSepolia: !connected || chainId === SEPOLIA_CHAIN_ID,
      label: session?.label,
      isPending,
      error,
      connectInjected,
      connectWalletConnect,
      disconnect,
      switchToSepolia,
      isSwitching,
    };
  }, [
    session,
    isPending,
    error,
    connectInjected,
    connectWalletConnect,
    disconnect,
    switchToSepolia,
    isSwitching,
  ]);

  return (
    <WalletContext.Provider value={value}>{children}</WalletContext.Provider>
  );
}

export function useWallet() {
  const ctx = useContext(WalletContext);
  if (!ctx) {
    throw new Error("useWallet must be used within WalletProvider");
  }
  return ctx;
}
