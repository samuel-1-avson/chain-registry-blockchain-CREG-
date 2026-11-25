type TrustedTypePolicyFactoryLike = {
  createPolicy: (
    name: string,
    rules: {
      createHTML?: (input: string) => string;
      createScript?: (input: string) => string;
      createScriptURL?: (input: string) => string;
    },
  ) => unknown;
};

const win = typeof window !== "undefined" ? (window as unknown as { trustedTypes?: TrustedTypePolicyFactoryLike }) : null;

if (win?.trustedTypes?.createPolicy) {
  try {
    win.trustedTypes.createPolicy("creg-default", {
      createScriptURL(url: string) {
        const parsed = new URL(url, window.location.href);
        if (parsed.origin !== window.location.origin) {
          throw new TypeError(`Blocked cross-origin script URL: ${parsed.origin}`);
        }
        return parsed.toString();
      },
      createHTML() {
        throw new TypeError("HTML injection is forbidden under creg-default policy");
      },
      createScript() {
        throw new TypeError("Dynamic script evaluation is forbidden under creg-default policy");
      },
    });
  } catch {
    // Policy already registered in HMR/strict-mode contexts.
  }
}

export {};
