/** Fallback when @walletconnect/ethereum-provider is not installed. */
export const EthereumProvider = {
  init: async (): Promise<never> => {
    throw new Error(
      "WalletConnect is not installed. Add @walletconnect/ethereum-provider to hub-web dependencies.",
    );
  },
};
