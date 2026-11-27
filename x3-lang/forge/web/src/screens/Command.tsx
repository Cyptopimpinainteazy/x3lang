import { useCallback, useEffect, useState } from "react";
import { api, errorsOf } from "../api";
import { createOperatorKey, operatorKey, publicHex, sign, supported } from "../keys";
import type { ForgeErr, Status } from "../schema";
import { Badge, Button, Errors, Field, Mono, Panel, inputCls, short } from "../ui";
import { create } from "zustand";

interface CommandState {
  status: Status | null;
  localKey: string | null;
  isCommander: boolean;
  refresh: () => Promise<void>;
}

export const useCommand = create<CommandState>((set) => ({
  status: null,
  localKey: null,
  isCommander: false,
  refresh: async () => {
    const status = await api.get<Status>("/api/status");
    const pair = await operatorKey().catch(() => undefined);
    const localKey = pair ? await publicHex(pair) : null;
    set({ status, localKey, isCommander: !!status.claimed && !!localKey && localKey === status.operator_key });
  },
}));

export function CommandChip() {
  const { status, isCommander, refresh } = useCommand();
  useEffect(() => {
    refresh().catch(() => undefined);
  }, [refresh]);
  if (!status) return null;
  if (!status.claimed) return <Badge tone="gold" title="Claim High Command on the Birth step">High Command unclaimed</Badge>;
  return isCommander
    ? <Badge tone="acid" title={`operator key ${status.operator_key}`}>High Command · this browser</Badge>
    : <Badge tone="slate" title="This browser does not hold the operator key: viewing only">viewer</Badge>;
}

export function ClaimPanel() {
  const { status, refresh } = useCommand();
  const [token, setToken] = useState("");
  const [errors, setErrors] = useState<ForgeErr[]>([]);
  const [busy, setBusy] = useState(false);
  const [canSign, setCanSign] = useState<boolean | null>(null);
  useEffect(() => {
    supported().then(setCanSign);
  }, []);
  const claim = useCallback(async () => {
    setBusy(true);
    setErrors([]);
    try {
      const pair = await createOperatorKey();
      const pk = await publicHex(pair);
      const { payload_hex } = await api.post<{ payload_hex: string }>("/api/claim/payload", { public_key: pk, token });
      const signature = await sign(pair, payload_hex);
      await api.post("/api/claim", { token, public_key: pk, signature });
      await refresh();
    } catch (e) {
      setErrors(errorsOf(e));
    } finally {
      setBusy(false);
    }
  }, [token, refresh]);
  if (!status || status.claimed) return null;
  return (
    <Panel title="Claim High Command">
      <p className="mb-3 text-sm text-slate-300">
        This Forge has no owner yet. The server printed a one-time setup token when it started. Claiming creates an
        Ed25519 key in this browser (it never leaves the browser and can't be exported) and signs the token with it.
        That key then signs every birth and every revocation. Without it, this Forge is read-only.
      </p>
      {canSign === false && <p className="mb-3 text-sm text-danger">This browser has no WebCrypto Ed25519. Use a current Chrome, Firefox or Safari.</p>}
      <div className="flex items-end gap-3">
        <div className="flex-1"><Field label="Setup token"><input className={inputCls} value={token} onChange={(e) => setToken(e.target.value.trim())} placeholder="from the server console" /></Field></div>
        <Button onClick={claim} disabled={!token || busy || canSign === false} tone="acid">{busy ? "Signing…" : "Claim"}</Button>
      </div>
      <div className="mt-3"><Errors errors={errors} /></div>
    </Panel>
  );
}

export function CommandSummary() {
  const { status, localKey, isCommander } = useCommand();
  if (!status?.claimed) return null;
  return (
    <div className="text-xs text-slate-400">
      High Command key <Mono className="text-slate-300">{short(status.operator_key, 12)}</Mono> · runtime <Mono>{short(status.runtime_id, 8)}</Mono>
      {!isCommander && <div className="mt-1 text-gold">This browser {localKey ? "holds a different key" : "holds no operator key"}: births and revocations are disabled here.</div>}
    </div>
  );
}
