import { createWalletAdapter } from "./wallet.mjs";

export function mountWallet() {
  const $ = id => document.getElementById(id);
  const providers = [];
  const adapters = new Map();
  let adapter = null, autofill = false, filledAddress = null;
  const compatible = provider => provider && typeof provider.request === "function" &&
    typeof provider.on === "function" && typeof provider.removeListener === "function";
  const names = {"0x1":"Ethereum", "0xa4b1":"Arbitrum", "0x2105":"Base"};
  function show(state) {
    const connected = state.status === "connected";
    $("wallet-connect").disabled = state.status === "connecting" || connected || !providers.length;
    $("wallet-disconnect").disabled = state.status === "disconnected";
    $("wallet-use-address").disabled = !connected;
    $("wallet-provider").disabled = state.status === "connecting" || connected;
    if (connected) {
      $("wallet-status").textContent = `${state.account} · ${names[state.chainId] || state.chainId} · address access only`;
      if (autofill) {
        $("quote-address").value = state.account;
        filledAddress = state.account;
      }
    } else {
      $("wallet-status").textContent = state.error || (state.status === "connecting"
        ? "Approve address access in your wallet, or disconnect locally to cancel."
        : providers.length ? "Wallet disconnected. Connect only when you want to share an address."
        : "No injected EVM wallet detected. Open this URL in a browser with a wallet extension, or enter a public quote address manually.");
      if (state.status !== "connecting" && filledAddress !== null) {
        if (autofill && $("quote-address").value === filledAddress) $("quote-address").value = "";
        filledAddress = null;
      }
    }
  }
  function add(provider, name) {
    if (!compatible(provider) || providers.some(row => row.provider === provider) || providers.length >= 16) return;
    providers.push({provider, name});
    $("wallet-provider").append(new Option(name, String(providers.length - 1)));
    if (!adapter) show({status:"disconnected", error:null});
  }
  window.addEventListener("eip6963:announceProvider", event => {
    const detail = event.detail;
    if (!detail || !detail.info || typeof detail.info.name !== "string") return;
    const name = detail.info.name;
    if (!name.trim() || name.length > 80 || /[\x00-\x1f]/.test(name)) return;
    add(detail.provider, name);
  });
  window.addEventListener("ethereum#initialized", () => add(window.ethereum, "Browser wallet"));
  add(window.ethereum, "Browser wallet");
  window.dispatchEvent(new Event("eip6963:requestProvider"));
  show({status:"disconnected", error:null});
  $("quote-address").addEventListener("input", () => { autofill = false; });
  $("wallet-connect").addEventListener("click", async () => {
    // Legacy injection may arrive after page initialization.
    add(window.ethereum, "Browser wallet");
    const selected = providers[Number($("wallet-provider").value)];
    if (!selected) return;
    if (adapter && adapter !== adapters.get(selected.provider)) adapter.disconnect();
    autofill = true;
    if (!adapters.has(selected.provider)) {
      const instance = createWalletAdapter(selected.provider, state => {
        if (adapter === instance) show(state);
      });
      adapters.set(selected.provider, instance);
    }
    adapter = adapters.get(selected.provider);
    show(adapter.state());
    await adapter.connect();
  });
  $("wallet-disconnect").addEventListener("click", () => adapter?.disconnect());
  $("wallet-use-address").addEventListener("click", () => {
    if (adapter?.state().status !== "connected") return;
    autofill = true;
    show(adapter.state());
  });
  $("wallet-provider").addEventListener("change", () => {
    adapter?.disconnect();
    adapter = null;
    show({status:"disconnected", error:null});
  });
}
