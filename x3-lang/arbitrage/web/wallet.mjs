// Read-only EIP-1193 account connection. Never signs, switches chains or sends transactions.
// Contract: https://eips.ethereum.org/EIPS/eip-1193 (checked 2026-11-27).
function accountFrom(value) {
  if (!Array.isArray(value) || value.length > 100 || !value.every(account =>
    typeof account === "string" && /^0x[0-9a-fA-F]{40}$/.test(account) && !/^0x0{40}$/.test(account)))
    throw new Error("Malformed account response");
  return value.length ? value[0].toLowerCase() : null;
}
function chainFrom(value) {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]{1,64}$/.test(value) || BigInt(value) === 0n)
    throw new Error("Malformed chain response");
  return `0x${BigInt(value).toString(16)}`;
}
function connectionError(error) {
  try {
    if (error?.code === 4001) return "Wallet connection was declined.";
    if (error?.code === 4100) return "Wallet account access was not authorized.";
    if (error?.code === 4900 || error?.code === 4901) return "Wallet provider is disconnected.";
  } catch { /* Untrusted error objects must not expose arbitrary messages. */ }
  return "Wallet connection failed. Check your wallet and try again.";
}
export function createWalletAdapter(provider, onChange = () => {}) {
  let current = { status: "disconnected", account: null, chainId: null, error: null };
  let revision = 0, accountRevision = 0, chainRevision = 0, pending = null;
  const attached = [];
  let active = false;
  const state = () => ({ ...current });
  function publish(value) {
    current = value;
    try { onChange(state()); } catch { /* Observer errors cannot change wallet authority. */ }
    return state();
  }
  function detach() {
    active = false;
    for (const [event, listener] of attached.splice(0)) {
      try { provider.removeListener(event, listener); } catch { /* Best effort local cleanup. */ }
    }
  }
  function clear(status = "disconnected", error = null) {
    revision++;
    detach();
    return publish({ status, account: null, chainId: null, error });
  }
  function accountsChanged(accounts) {
    if (!active) return;
    try {
      const account = accountFrom(accounts);
      accountRevision++;
      if (!account) return clear();
      publish({ ...current, account });
    } catch { clear("error", "Wallet supplied invalid account data. Reconnect to continue."); }
  }
  function chainChanged(value) {
    if (!active) return;
    try {
      const chainId = chainFrom(value);
      chainRevision++;
      publish({ ...current, chainId });
    } catch { clear("error", "Wallet supplied invalid chain data. Reconnect to continue."); }
  }
  function providerDisconnected() {
    if (!active) return;
    clear("disconnected", "Wallet provider disconnected.");
  }
  function attach(attempt) {
    active = true;
    for (const [event, listener] of [["accountsChanged", accountsChanged],
      ["chainChanged", chainChanged], ["disconnect", providerDisconnected]]) {
      // Record before registration so cleanup also covers providers that register then throw.
      attached.push([event, listener]);
      provider.on(event, listener);
      if (revision !== attempt) { detach(); return; }
    }
  }
  function connect() {
    if (pending) return pending;
    if (current.status === "connected") return Promise.resolve(state());
    if (!provider || typeof provider.request !== "function" || typeof provider.on !== "function" ||
        typeof provider.removeListener !== "function")
      return Promise.resolve(clear("error", "No compatible injected EVM wallet is available."));
    const attempt = ++revision;
    const accountsAtStart = accountRevision, chainAtStart = chainRevision;
    // Scheduling the work prevents reentrant observers from issuing duplicate account prompts.
    pending = Promise.resolve().then(async () => {
      if (revision !== attempt) return state();
      try {
        attach(attempt);
        if (revision !== attempt) return state();
        const accounts = accountFrom(await provider.request({ method: "eth_requestAccounts" }));
        if (revision !== attempt) return state();
        const chainId = chainFrom(await provider.request({ method: "eth_chainId" }));
        if (revision !== attempt) return state();
        const account = accountRevision === accountsAtStart ? accounts : current.account;
        const chain = chainRevision === chainAtStart ? chainId : current.chainId;
        if (!account) return clear();
        return publish({ status: "connected", account, chainId: chain, error: null });
      } catch (error) {
        if (revision !== attempt) return state();
        return clear("error", connectionError(error));
      }
    }).finally(() => { pending = null; });
    publish({ status: "connecting", account: null, chainId: null, error: null });
    return pending;
  }
  return Object.freeze({ connect, disconnect: () => clear(), state });
}
