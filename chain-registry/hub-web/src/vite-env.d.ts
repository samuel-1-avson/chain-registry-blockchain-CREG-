/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_WALLETCONNECT_PROJECT_ID?: string;
  readonly VITE_EXPLORER_URL?: string;
  readonly VITE_FAUCET_URL?: string;
  readonly VITE_DOCS_URL?: string;
  readonly VITE_API_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

declare module "*.md?raw" {
  const content: string;
  export default content;
}

declare module "@walletconnect/ethereum-provider" {
  export const EthereumProvider: {
    init: (config: Record<string, unknown>) => Promise<{
      enable: () => Promise<void>;
      accounts: string[];
      chainId: number | string;
      disconnect?: () => Promise<void>;
      request: (args: {
        method: string;
        params?: unknown[];
      }) => Promise<unknown>;
    }>;
  };
}
