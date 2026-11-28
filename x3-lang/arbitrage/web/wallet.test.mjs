// Protocol fixtures exercise the adapter; these are not real-wallet execution evidence.
import { test } from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { createWalletAdapter } from "./wallet.mjs";
const ACCOUNT = "0x1111111111111111111111111111111111111111";
const OTHER = "0x2222222222222222222222222222222222222222";
class Provider extends EventEmitter {
  calls = [];
  async request({ method }) {
    this.calls.push(method);
    if (method === "eth_requestAccounts") return [ACCOUNT];
    if (method === "eth_chainId") return "0x2105";
    throw new Error("unexpected method");
  }
}
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
test("connection is explicit and requests only account and chain reads", async () => {
  const provider = new Provider(), changes = [];
  const wallet = createWalletAdapter(provider, value => changes.push(value));
  assert.deepEqual(provider.calls, []);
  assert.equal(wallet.state().status, "disconnected");
  const result = await wallet.connect();
  assert.deepEqual(result, { status: "connected", account: ACCOUNT, chainId: "0x2105", error: null });
  assert.deepEqual(provider.calls, ["eth_requestAccounts", "eth_chainId"]);
  result.account = OTHER;
  assert.equal(wallet.state().account, ACCOUNT);
  await wallet.connect();
  assert.equal(provider.calls.length, 2);
  assert.equal(changes.at(-1).status, "connected");
});

test("duplicate connect calls share one permission request", async () => {
  const provider = new Provider(), permission = deferred();
  provider.request = ({ method }) => { provider.calls.push(method); return method === "eth_requestAccounts" ? permission.promise : Promise.resolve("0x1"); };
  const wallet = createWalletAdapter(provider);
  const first = wallet.connect(), second = wallet.connect();
  assert.equal(first, second);
  await Promise.resolve();
  assert.deepEqual(provider.calls, ["eth_requestAccounts"]);
  permission.resolve([ACCOUNT]);
  assert.equal((await first).status, "connected");
});

test("local disconnect during permission prompt blocks late connection and duplicate prompts", async () => {
  const provider = new Provider(), permission = deferred();
  provider.request = ({ method }) => { provider.calls.push(method); return permission.promise; };
  const wallet = createWalletAdapter(provider), attempt = wallet.connect();
  await Promise.resolve();
  wallet.disconnect();
  assert.equal(wallet.connect(), attempt);
  permission.resolve([ACCOUNT]);
  assert.equal((await attempt).status, "disconnected");
  assert.deepEqual(provider.calls, ["eth_requestAccounts"]);
  assert.equal(provider.listenerCount("accountsChanged"), 0);
});

test("account and chain changes update public state; empty accounts clears it", async () => {
  const provider = new Provider(), wallet = createWalletAdapter(provider);
  await wallet.connect();
  provider.emit("accountsChanged", [OTHER]);
  provider.emit("chainChanged", "0x0001");
  assert.equal(wallet.state().account, OTHER);
  assert.equal(wallet.state().chainId, "0x1");
  provider.emit("accountsChanged", []);
  assert.equal(wallet.state().status, "disconnected");
  assert.equal(wallet.state().chainId, null);
  assert.equal(provider.listenerCount("chainChanged"), 0);
});

test("events during connect take precedence over stale response values", async () => {
  const provider = new Provider(), chain = deferred();
  provider.request = ({ method }) => {
    provider.calls.push(method);
    return method === "eth_requestAccounts" ? Promise.resolve([ACCOUNT]) : chain.promise;
  };
  const wallet = createWalletAdapter(provider), attempt = wallet.connect();
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  provider.emit("accountsChanged", [OTHER]);
  provider.emit("chainChanged", "0xa4b1");
  chain.resolve("0x1");
  assert.deepEqual(await attempt, { status:"connected", account:OTHER, chainId:"0xa4b1", error:null });
});

test("rejections expose fixed safe messages and permit explicit retry", async () => {
  const provider = new Provider();
  provider.request = async () => { throw {code:4001,message:"private text must not be displayed"}; };
  const wallet = createWalletAdapter(provider);
  assert.equal((await wallet.connect()).error, "Wallet connection was declined.");
  assert.equal(provider.listenerCount("accountsChanged"), 0);
  provider.request = Provider.prototype.request;
  assert.equal((await wallet.connect()).status, "connected");
});

for (const invalid of ["0x0", "1", "0xxyz", "0x"+"f".repeat(65), null]) {
  test(`malformed chain data clears connection (${String(invalid)})`, async () => {
    const provider = new Provider(), wallet = createWalletAdapter(provider);
    await wallet.connect();
    provider.emit("chainChanged", invalid);
    assert.equal(wallet.state().status, "error");
    assert.equal(wallet.state().account, null);
    assert.equal(provider.listenerCount("accountsChanged"), 0);
  });
}

for (const invalid of ["0xsecret", [ACCOUNT,"wrong"], ["0x"+"0".repeat(40)], null]) {
  test("malformed account event clears connection", async () => {
    const provider = new Provider(), wallet = createWalletAdapter(provider);
    await wallet.connect();
    provider.emit("accountsChanged", invalid);
    assert.equal(wallet.state().status, "error");
    assert.equal(wallet.state().account, null);
  });
}

test("provider disconnect clears state and never sends a revoke or signing request", async () => {
  const provider = new Provider(), wallet = createWalletAdapter(provider);
  const unrelated = () => {};
  provider.on("disconnect", unrelated);
  await wallet.connect();
  provider.emit("disconnect", {code:4900,message:"private"});
  assert.equal(wallet.state().status, "disconnected");
  assert.equal(wallet.state().account, null);
  assert.equal(provider.listenerCount("disconnect"), 1);
  wallet.disconnect();
  assert.deepEqual(provider.calls, ["eth_requestAccounts","eth_chainId"]);
});

test("missing provider fails without throwing or accessing wallet methods", async () => {
  const wallet = createWalletAdapter(null);
  assert.equal((await wallet.connect()).status, "error");
  assert.equal(wallet.state().account, null);
});

test("reentrant UI observer cannot issue another account prompt", async () => {
  const provider = new Provider();
  let wallet;
  wallet = createWalletAdapter(provider, state => { if(state.status === "connecting") wallet.connect(); });
  await wallet.connect();
  assert.deepEqual(provider.calls, ["eth_requestAccounts","eth_chainId"]);
});

test("disconnect during listener registration leaves no registered adapter listeners", async () => {
  const provider = new Provider();
  provider.on = function(event, listener) {
    EventEmitter.prototype.on.call(this,event,listener);
    if (event === "accountsChanged") this.emit("accountsChanged",[]);
    return this;
  };
  const wallet = createWalletAdapter(provider);
  assert.equal((await wallet.connect()).status, "disconnected");
  assert.deepEqual(provider.calls, []);
  assert.equal(provider.listenerCount("chainChanged"), 0);
});

test("invalid initial account or chain responses never create a connected state", async () => {
  for (const badMethod of ["eth_requestAccounts", "eth_chainId"]) {
    const provider = new Provider();
    provider.request = async ({method}) => {
      provider.calls.push(method);
      if(method === badMethod) return {result:"not a direct EIP-1193 result"};
      return method === "eth_requestAccounts" ? [ACCOUNT] : "0x1";
    };
    const wallet = createWalletAdapter(provider);
    const result = await wallet.connect();
    assert.equal(result.status,"error");
    assert.equal(result.account,null);
    assert.equal(result.chainId,null);
  }
});

test("callbacks retained by a broken provider cannot reconnect after local disconnect", async () => {
  const provider = new Provider();
  provider.removeListener = () => { throw new Error("broken removal"); };
  const wallet = createWalletAdapter(provider);
  await wallet.connect();
  wallet.disconnect();
  provider.emit("accountsChanged",[OTHER]);
  provider.emit("chainChanged","0x1");
  assert.equal(wallet.state().status,"disconnected");
  assert.equal(wallet.state().account,null);
});
