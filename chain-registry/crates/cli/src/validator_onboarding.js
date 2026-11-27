const byId = (id) => document.getElementById(id);
const session = location.hash.slice(1) || sessionStorage.getItem("creg-validator-session");
if (location.hash && session) {
  sessionStorage.setItem("creg-validator-session", session);
  history.replaceState(null, "", location.pathname);
}

const connect = byId("connect");
const switchAccount = byId("switch-account");
const rescanWallets = byId("rescan-wallets");
const walletList = byId("wallet-list");
const walletHint = byId("wallet-hint");
const stake = byId("stake");
const register = byId("register");
const refreshButton = byId("refresh");
const start = byId("start");
let config;
let provider;
let wallet;
let connectedWalletName = null;
let busy = false;
let refreshing = false;
let walletState;
let enrollment;
let pendingHash;
const detectedWallets = [];
let selectedWalletId = null;
const boundProviders = new WeakSet();

function notice(text, error = false) {
  const element = byId("status");
  element.textContent = text;
  element.dataset.error = String(error);
}

async function api(path, body = {}) {
  const response = await fetch("/api/" + path, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Creg-Session": session || "" },
    body: JSON.stringify(body),
  });
  if (!response.ok) {
    const data = await response.json().catch(() => ({}));
    throw new Error(data.error || "CREG returned HTTP " + response.status);
  }
  return response.json();
}

function isSafeIconDataUri(value) {
  return typeof value === "string"
    && /^data:image\/(?:png|svg\+xml|webp|jpeg|gif)(?:;[a-z0-9=+-]+)*,/i.test(value.trim());
}

function inferLegacyWalletName(candidate) {
  if (!candidate || typeof candidate !== "object") return null;
  if (candidate.isBraveWallet) return "Brave Wallet";
  if (candidate.isOkxWallet || candidate.isOKExWallet) return "OKX Wallet";
  if (candidate.isRabby) return "Rabby Wallet";
  if (candidate.isCoinbaseWallet) return "Coinbase Wallet";
  if (candidate.isPhantom) return "Phantom";
  if (candidate.isTrust || candidate.isTrustWallet) return "Trust Wallet";
  if (candidate.isMetaMask) return "MetaMask";
  return null;
}

function normalizeWalletName(name) {
  return String(name || "").trim().toLowerCase().replace(/\s+wallet$/, "").trim();
}

function getSelectedWallet() {
  return detectedWallets.find((entry) => entry.id === selectedWalletId) || null;
}

function registerWalletCandidate(entry) {
  if (!entry?.provider || typeof entry.provider.request !== "function") return false;
  const inferredName = inferLegacyWalletName(entry.provider);
  const name = (entry.name || "").trim() || inferredName || "Browser Wallet";
  const normalizedName = normalizeWalletName(name);
  const rdns = typeof entry.rdns === "string" ? entry.rdns.trim() : "";
  const icon = isSafeIconDataUri(entry.icon) ? entry.icon.trim() : "";
  const source = entry.source === "eip6963" ? "eip6963" : "legacy";
  const entryId = entry.id || (rdns ? "eip6963:" + rdns
    : "wallet:" + name.toLowerCase().replace(/[^a-z0-9]+/g, "-") + ":" + (detectedWallets.length + 1));

  const byProvider = detectedWallets.find((item) => item.provider === entry.provider);
  if (byProvider) {
    if (source === "eip6963" && (byProvider.source !== "eip6963" || name !== byProvider.name || icon !== byProvider.icon || rdns !== byProvider.rdns)) {
      byProvider.name = name;
      byProvider.rdns = rdns;
      byProvider.icon = icon;
      byProvider.source = "eip6963";
      return true;
    }
    return false;
  }

  if (source === "eip6963") {
    const existingEip6963 = detectedWallets.find((item) =>
      item.source === "eip6963" && ((rdns && item.rdns === rdns) || item.id === entryId));
    if (existingEip6963) {
      existingEip6963.provider = entry.provider;
      existingEip6963.name = name;
      if (icon) existingEip6963.icon = icon;
      return true;
    }
    const existingLegacy = normalizedName
      ? detectedWallets.find((item) => item.source === "legacy" && normalizeWalletName(item.name) === normalizedName)
      : null;
    if (existingLegacy) {
      if (selectedWalletId === existingLegacy.id) {
        selectedWalletId = entryId;
      }
      existingLegacy.id = entryId;
      existingLegacy.name = name;
      existingLegacy.rdns = rdns;
      existingLegacy.icon = icon;
      existingLegacy.provider = entry.provider;
      existingLegacy.source = "eip6963";
      return true;
    }
  } else {
    if (detectedWallets.some((item) => normalizedName && normalizeWalletName(item.name) === normalizedName)) return false;
    if (!inferredName && !entry.name && detectedWallets.length > 0) return false;
  }

  detectedWallets.push({
    id: entryId,
    name,
    rdns,
    icon,
    provider: entry.provider,
    source,
  });
  return true;
}

function updateButtons() {
  const connected = Boolean(wallet && walletState);
  const canApply = walletState && [0, 4, 5, 6].includes(walletState.application);
  const selected = getSelectedWallet();
  connect.textContent = selected
    ? (wallet && provider === selected.provider ? "Reconnect " + selected.name : "Connect " + selected.name)
    : "Connect wallet";
  connect.disabled = !config || busy || (detectedWallets.length > 0 && !selected);
  switchAccount.hidden = !wallet;
  switchAccount.disabled = !wallet || busy;
  rescanWallets.disabled = busy;
  stake.disabled = !connected || busy || Boolean(pendingHash) || !canApply
    || walletState.balance < walletState.minimum || walletState.gas === 0n;
  register.disabled = !connected || busy || ![1, 2].includes(walletState.application)
    || !enrollment || enrollment.registered;
  refreshButton.disabled = !wallet || busy;
  start.disabled = busy || !enrollment?.registered || enrollment.launch_status === "starting"
    || enrollment.local_node !== "Local node unavailable";
}

const hex = (value) => "0x" + value.toString(16);
const word = (value) => value.toString(16).toLowerCase().replace(/^0x/, "").padStart(64, "0");
const amount = (value) => {
  const units = value / 10n ** 18n;
  const decimals = wordDecimals(value);
  return units.toString() + (decimals ? "." + decimals : "");
};
function wordDecimals(value) {
  return (value % 10n ** 18n).toString().padStart(18, "0").replace(/0+$/, "");
}

function request(method, params = []) {
  return provider.request({ method, params });
}

async function checkWallet(address) {
  const [chain, accounts] = await Promise.all([
    request("eth_chainId"),
    request("eth_accounts"),
  ]);
  if (BigInt(chain) !== BigInt(config.l1_chain_id) || accounts[0]?.toLowerCase() !== address) {
    throw new Error("Select the connected wallet on Sepolia and try again.");
  }
}

async function call(to, data) {
  const result = await request("eth_call", [{ to, data }, "latest"]);
  if (typeof result !== "string" || !/^0x(?:[a-fA-F0-9]{64})+$/.test(result)) {
    throw new Error("The Sepolia RPC returned an invalid contract response.");
  }
  return result;
}

async function readWallet(address) {
  await checkWallet(address);
  const [gas, balance, minimum, application] = await Promise.all([
    request("eth_getBalance", [address, "latest"]),
    call(config.token, config.balance + word(address)),
    call(config.staking, config.minimum),
    call(config.staking, config.validator + word(address)),
  ]);
  if (application.length < 130 || BigInt(minimum) === 0n) {
    throw new Error("Could not read the R7 staking contract.");
  }
  return {
    gas: BigInt(gas),
    balance: BigInt(balance),
    minimum: BigInt(minimum),
    application: Number(BigInt("0x" + application.slice(66, 130))),
  };
}

async function refresh() {
  if (!wallet || refreshing) return;
  refreshing = true;
  const address = wallet;
  try {
    const [nextWalletState, nextEnrollment] = await Promise.all([
      readWallet(address),
      api("status", { address }),
    ]);
    if (address !== wallet) return;
    walletState = nextWalletState;
    enrollment = nextEnrollment;
    byId("balance").textContent = amount(walletState.balance) + " tCREG · " + amount(walletState.gas) + " Sepolia ETH";
    byId("stake-detail").textContent = "The R7 contract requires " + amount(walletState.minimum)
      + " tCREG and Sepolia ETH for gas. Application: "
      + (["None", "Pending", "Active", "Unbonding", "Withdrawn", "Rejected", "Expired"][walletState.application] || "Unknown");
    byId("registration").textContent = enrollment.registered ? "Identity registered" : "Awaiting signed registration";
    const adm = enrollment.admission;
    byId("admission").textContent = enrollment.admitted ? "Active in the validator set"
      : adm?.stage ? adm.stage.replaceAll("_", " ")
      : walletState.application === 2 ? "Approved on Sepolia · waiting for R7 to sync"
      : walletState.application === 1 ? "Awaiting approval from the validator set"
      : "Awaiting application";
    if (adm) {
      const l1Req = adm.l1_approvals_required || "?";
      const l1Col = adm.l1_approvals_collected ?? 0;
      const trPart = adm.staking_state === "active" || adm.transition_approvals_collected > 0 || adm.voting_activation_height != null
        ? ` · Transition ${adm.transition_approvals_collected ?? 0}/${adm.transition_approvals_required || "?"}`
        : "";
      byId("approvals").textContent = `${l1Col}/${l1Req} Sepolia approvals${trPart}`;
      byId("approval-tx").textContent = adm.l1_tx_hash
        ? `${adm.l1_tx_status} · ${adm.l1_tx_hash}${adm.l1_confirmed_block != null ? " (block #" + adm.l1_confirmed_block + ")" : ""}`
        : (adm.l1_tx_status || "not_submitted");
      byId("activation").textContent = adm.voting_activation_height != null
        ? `Block #${adm.voting_activation_height}`
        : "Pending certified transition";
      const warnings = adm.stage === "active_voting"
        ? []
        : [...(enrollment.signer_warnings || []), ...(adm.missing_signer_setup || [])]
            .filter((v, i, arr) => v && arr.indexOf(v) === i);
      byId("blocker").textContent = warnings.length
        ? `${adm.stage_reason} · Signer notice: ${warnings.join("; ")}`
        : (adm.stage_reason || "—");
    } else {
      byId("approvals").textContent = enrollment.admission_unavailable_reason ? "Telemetry unavailable (node rollout pending)" : "—";
      byId("approval-tx").textContent = "—";
      byId("activation").textContent = enrollment.admitted ? "Certified" : "Pending";
      byId("blocker").textContent = enrollment.admission_unavailable_reason
        || (enrollment.signer_warnings?.length ? enrollment.signer_warnings.join("; ") : "—");
    }
    byId("local").textContent = enrollment.local_node;
    let launchMessage = "";
    if (enrollment.local_node === "Local node unavailable") {
      if (enrollment.launch_status === "starting") {
        launchMessage = "Starting Docker. Log: " + config.launch_log;
      } else if (enrollment.launch_error) {
        launchMessage = enrollment.launch_error;
      } else if (enrollment.launch_status === "failed") {
        launchMessage = "Launcher failed. Log: " + config.launch_log;
      } else if (enrollment.launch_status === "started") {
        launchMessage = "Launcher completed. Waiting for node health.";
      }
    }
    byId("launch").textContent = launchMessage;
    const voteH = enrollment.vote_height ?? adm?.first_finalized_vote_height;
    byId("vote").textContent = voteH == null ? "Waiting for a matching commit"
      : "First finalized vote recorded at block #" + voteH;
    if (pendingHash) {
      const receipt = await request("eth_getTransactionReceipt", [pendingHash]);
      if (address === wallet && receipt) {
        pendingHash = undefined;
        notice(BigInt(receipt.status) === 1n ? "Transaction confirmed. Review the updated state."
          : "Transaction reverted. Review the wallet state before trying again.", BigInt(receipt.status) !== 1n);
      }
    }
  } finally {
    refreshing = false;
    updateButtons();
  }
}

function resetWallet() {
  wallet = undefined;
  connectedWalletName = null;
  walletState = undefined;
  enrollment = undefined;
  pendingHash = undefined;
  for (const id of ["wallet", "balance", "transaction"]) byId(id).textContent = "";
  byId("stake-detail").textContent = "The application uses Sepolia ETH for gas.";
  byId("registration").textContent = "Connect your wallet to check registration.";
  byId("admission").textContent = "Awaiting application";
  byId("approvals").textContent = "—";
  byId("approval-tx").textContent = "—";
  byId("activation").textContent = "—";
  byId("blocker").textContent = "—";
  byId("local").textContent = "Waiting for status";
  byId("launch").textContent = "";
  byId("vote").textContent = "Waiting for a matching commit";
  updateButtons();
}

function renderWalletList() {
  if (detectedWallets.length === 1 && !selectedWalletId) {
    selectedWalletId = detectedWallets[0].id;
  }
  if (detectedWallets.length === 0) {
    walletHint.textContent = "No browser wallet detected yet. Enable or unlock an Ethereum wallet extension and click Rescan wallets.";
  } else if (detectedWallets.length === 1) {
    walletHint.textContent = "Detected " + detectedWallets[0].name + ". Click Connect to continue.";
  } else if (!selectedWalletId) {
    walletHint.textContent = "Multiple wallets detected — choose the wallet you want to use for this validator.";
  } else {
    const selected = getSelectedWallet();
    walletHint.textContent = "Selected " + (selected ? selected.name : "wallet")
      + ". You can switch wallets below before or after connecting.";
  }

  walletList.replaceChildren();
  for (const entry of detectedWallets) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "wallet-option";
    button.setAttribute("role", "radio");
    button.setAttribute("aria-checked", String(entry.id === selectedWalletId));
    button.dataset.walletId = entry.id;
    if (entry.icon) {
      const img = document.createElement("img");
      img.src = entry.icon;
      img.alt = "";
      button.appendChild(img);
    }
    const label = document.createElement("span");
    label.textContent = entry.name;
    button.appendChild(label);
    if (entry.rdns) {
      const rdns = document.createElement("span");
      rdns.className = "wallet-rdns";
      rdns.textContent = entry.rdns;
      button.appendChild(rdns);
    }
    button.addEventListener("click", () => {
      if (busy) return;
      selectedWalletId = entry.id;
      if (wallet && provider !== entry.provider) {
        resetWallet();
        notice("Selected " + entry.name + ". Click Connect " + entry.name + " to continue.");
      }
      renderWalletList();
      updateButtons();
    });
    walletList.appendChild(button);
  }
}

function discoverWallets() {
  try {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
  } catch {
    // Ignore environments where custom Event dispatch fails.
  }

  const legacyProviders = Array.isArray(window.ethereum?.providers) ? window.ethereum.providers : [];
  for (const candidate of legacyProviders) {
    registerWalletCandidate({
      name: inferLegacyWalletName(candidate),
      provider: candidate,
      source: "legacy",
    });
  }

  const namedCandidates = [
    { name: inferLegacyWalletName(window.ethereum), provider: window.ethereum, source: "legacy" },
    { name: "OKX Wallet", provider: window.okxwallet, source: "legacy" },
    { name: "Rabby Wallet", provider: window.rabby, source: "legacy" },
    { name: "Phantom", provider: window.phantom?.ethereum, source: "legacy" },
    { name: "Coinbase Wallet", provider: window.coinbaseWalletExtension, source: "legacy" },
    { name: "Trust Wallet", provider: window.trustwallet, source: "legacy" },
  ];
  for (const candidate of namedCandidates) {
    registerWalletCandidate(candidate);
  }

  renderWalletList();
  updateButtons();
}

window.addEventListener("eip6963:announceProvider", (event) => {
  const detail = event?.detail;
  const info = detail?.info;
  if (registerWalletCandidate({
    id: info?.uuid ? "eip6963:" + info.uuid : undefined,
    name: info?.name,
    rdns: info?.rdns,
    icon: info?.icon,
    provider: detail?.provider,
    source: "eip6963",
  })) {
    renderWalletList();
    updateButtons();
  }
});

discoverWallets();
setTimeout(discoverWallets, 150);
setTimeout(discoverWallets, 800);

function bindProviderEvents(targetProvider) {
  if (!targetProvider?.on || boundProviders.has(targetProvider)) return;
  boundProviders.add(targetProvider);
  for (const event of ["accountsChanged", "chainChanged"]) {
    targetProvider.on(event, () => {
      if (provider !== targetProvider || !wallet) return;
      resetWallet();
      notice("Your wallet changed. Reconnect to continue.");
    });
  }
}

async function withAction(action) {
  if (busy) return;
  busy = true;
  updateButtons();
  try {
    await action();
  } catch (error) {
    notice(error.message || "The action could not be completed.", true);
  } finally {
    busy = false;
    updateButtons();
  }
}

async function connectSelectedWallet({ requestPermissions = false } = {}) {
  if (detectedWallets.length === 0) {
    discoverWallets();
  }
  const selected = getSelectedWallet();
  if (!selected) {
    throw new Error(detectedWallets.length > 0
      ? "Select one of the detected wallets above first."
      : "Open this page in a browser with an Ethereum wallet extension.");
  }
  provider = selected.provider;
  bindProviderEvents(provider);
  resetWallet();

  if (requestPermissions) {
    try {
      await request("wallet_requestPermissions", [{ eth_accounts: {} }]);
    } catch (error) {
      if (error?.code === 4001) throw error;
    }
  }

  await request("eth_requestAccounts");
  const chainId = hex(config.l1_chain_id);
  if (BigInt(await request("eth_chainId")) !== BigInt(config.l1_chain_id)) {
    try {
      await request("wallet_switchEthereumChain", [{ chainId }]);
    } catch (error) {
      if ((error?.code === 4902 || error?.data?.originalError?.code === 4902)
        && Number(config.l1_chain_id) === 11155111) {
        await request("wallet_addEthereumChain", [{
          chainId: "0xaa36a7",
          chainName: "Sepolia",
          nativeCurrency: { name: "Sepolia ETH", symbol: "ETH", decimals: 18 },
          rpcUrls: ["https://ethereum-sepolia-rpc.publicnode.com"],
          blockExplorerUrls: ["https://sepolia.etherscan.io"],
        }]);
        await request("wallet_switchEthereumChain", [{ chainId }]);
      } else {
        throw error;
      }
    }
  }
  const accounts = await request("eth_accounts");
  const address = accounts[0]?.toLowerCase();
  if (!/^0x[0-9a-f]{40}$/.test(address || "")) throw new Error("The wallet returned no Sepolia account.");
  wallet = address;
  connectedWalletName = selected.name;
  byId("wallet").textContent = connectedWalletName + " · " + address;
  await refresh();
  notice(selected.name + " connected. Follow the application and registration steps below.");
}

connect.addEventListener("click", () => withAction(() => connectSelectedWallet({ requestPermissions: false })));
switchAccount.addEventListener("click", () => withAction(() => connectSelectedWallet({ requestPermissions: true })));
rescanWallets.addEventListener("click", () => {
  discoverWallets();
  notice(detectedWallets.length > 0
    ? "Rescanned wallets: " + detectedWallets.map((w) => w.name).join(", ") + "."
    : "No browser wallet detected yet. Check your extension settings and retry.");
});

async function send(address, to, data, label) {
  await checkWallet(address);
  if (wallet !== address) throw new Error("Reconnect the wallet before continuing.");
  notice("Confirm " + label + " in your wallet.");
  const hash = await request("eth_sendTransaction", [{ from: address, to, data }]);
  if (!/^0x[0-9a-f]{64}$/i.test(hash)) throw new Error("Wallet returned an invalid transaction hash.");
  pendingHash = hash;
  byId("transaction").textContent = label + " · " + hash;
  for (let attempt = 0; attempt < 45; attempt++) {
    await checkWallet(address);
    const receipt = await request("eth_getTransactionReceipt", [hash]);
    if (receipt) {
      pendingHash = undefined;
      if (BigInt(receipt.status) !== 1n) throw new Error(label + " reverted. Check the wallet before trying again.");
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 2000));
  }
  throw new Error(label + " is still awaiting a receipt. Use Refresh status to check it.");
}

stake.addEventListener("click", () => withAction(async () => {
  const address = wallet;
  const current = await readWallet(address);
  if (![0, 4, 5, 6].includes(current.application)) throw new Error("This wallet already has a validator record.");
  if (current.gas === 0n || current.balance < current.minimum) throw new Error("Fund your wallet with tCREG and Sepolia ETH first.");
  const allowance = await call(config.token, config.allowance + word(address) + word(config.staking));
  if (BigInt(allowance) < current.minimum) {
    await send(address, config.token, config.approve + word(config.staking) + word(current.minimum), "token approval");
  }
  const latest = await readWallet(address);
  if (latest.minimum !== current.minimum || ![0, 4, 5, 6].includes(latest.application)) {
    await refresh();
    throw new Error("The staking requirement or application state changed. Review the updated status.");
  }
  await send(address, config.staking, config.apply + word(current.minimum), "validator application");
  await refresh();
  notice("Application submitted. Sign the validator identity registration next.");
}));

register.addEventListener("click", () => withAction(async () => {
  const address = wallet;
  const current = await readWallet(address);
  if (![1, 2].includes(current.application)) throw new Error("Submit the validator application before registering.");
  const { message } = await api("challenge", { address });
  const data = "0x" + Array.from(new TextEncoder().encode(message), (byte) => byte.toString(16).padStart(2, "0")).join("");
  await checkWallet(address);
  notice("Sign the validator identity message in your wallet.");
  const signature = await request("personal_sign", [data, address]);
  await checkWallet(address);
  await api("register", { address, signature });
  await refresh();
  notice("Identity registered. R7 will pick up approval and node sync as they happen.");
}));

start.addEventListener("click", () => withAction(async () => {
  await api("start", { address: wallet });
  notice("Starting local R7 sync. You can follow its progress below.");
  await refresh();
}));

refreshButton.addEventListener("click", () => withAction(refresh));
setInterval(() => {
  if (!busy && wallet) refresh().catch((error) => notice(error.message, true));
}, 12000);

api("config").then((result) => {
  config = result;
  byId("identity").textContent = config.node_id + " · " + config.public_key;
  byId("faucet").href = config.faucet;
  start.hidden = !config.launch_log;
  updateButtons();
  notice("R7 verified. Connect a Sepolia wallet to begin.");
}).catch((error) => notice(error.message, true));
