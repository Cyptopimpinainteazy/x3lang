import { units, edge, matches } from "./format.mjs";
const $ = (id) => document.getElementById(id);
const chain = (value) =>
  ({
    sol: "Solana",
    near: "NEAR",
    eth: "Ethereum",
    arb: "Arbitrum",
    pol: "Polygon",
    bsc: "BSC",
    "cex-inventory": "Exchange inventory",
  })[value] || value;
let state, selected, pendingKey;
function el(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}
function signed(value, decimals) {
  return value !== null && BigInt(value) > 0n
    ? "+" + units(value, decimals)
    : units(value, decimals);
}
function symbol(asset) {
  return (
    state.asset_symbols?.[`${asset.network}/${asset.asset_id}`] ||
    asset.asset_id
  );
}

async function request(path, body) {
  const response = await fetch(
    path,
    body === undefined
      ? {}
      : {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "X-X3-CSRF": state.csrf,
          },
          body: JSON.stringify(body),
        },
  );
  const result = await response.json();
  if (!response.ok) throw new Error(result.message || result.error);
  return result;
}
function render() {
  $("mode").textContent =
    state.mode === "demo" ? "CONSTRUCTED DEMO" : "LIVE OBSERVATIONS";
  $("notice").textContent =
    state.mode === "demo"
      ? "DEMO DATA — Constructed opportunities for exploring the workflow. Refresh regenerates examples; these are not market prices."
      : "LIVE DISCOVERY — Quotes are indicative. External costs are operator estimates. Sequential routes carry partial-execution risk.";
  $("route-count").textContent = state.routes.length;
  $("eligible-count").textContent = state.routes.filter(
    (row) => row.paper_eligible,
  ).length;
  const eligibleRows = state.routes.filter((row) => row.paper_eligible);
  $("best-edge").textContent = eligibleRows.length
    ? edge(Math.max(...eligibleRows.map((row) => row.net_profit_bps)))
    : "—";
  $("best-edge").className = eligibleRows.length ? "positive" : "";
  $("updated").textContent = state.refreshing
    ? "Fetching provider observations…"
    : state.updated_at
      ? `Last scan ${new Date(state.updated_at * 1000).toLocaleTimeString()}`
      : "Refresh to fetch public markets";
  $("refresh").disabled = state.refreshing;
  const rows = state.routes.filter((row) =>
    matches(row, {
      search: $("search").value,
      kind: $("kind").value,
      minimum: $("edge").value,
      eligible: $("eligible").checked,
    }),
  );
  $("routes").replaceChildren();
  for (const row of rows) {
    const tr = el("tr"),
      name = el("td");
    name.append(
      el("div", row.label, "route-name"),
      el(
        "small",
        `${row.demo ? "DEMO · " : ""}${row.kind === "cross-chain" ? "1Click solver quotes" : "Prefunded CEX inventory"} · ${row.legs.length} legs`,
      ),
    );
    const input = el("td", units(row.input_units, row.numeraire.decimals));
    input.append(
      el("small", `${symbol(row.numeraire)} · ${chain(row.numeraire.network)}`),
    );
    const profit = el(
      "td",
      signed(row.net_profit_units, row.numeraire.decimals),
      row.net_profit_bps > 0 ? "positive" : "negative",
    );
    profit.append(el("small", `in source ${symbol(row.numeraire)}`));
    const status = el("td");
    status.append(
      el(
        "span",
        row.paper_eligible ? "Paper eligible" : "Needs review",
        `chip ${row.paper_eligible ? "" : "blocked"}`,
      ),
    );
    const action = el("td"),
      button = el("button", "Inspect ↗", "inspect");
    button.addEventListener("click", () => inspect(row));
    action.append(button);
    tr.append(
      name,
      input,
      profit,
      el(
        "td",
        edge(row.net_profit_bps),
        row.net_profit_bps > 0 ? "positive" : "",
      ),
      status,
      action,
    );
    $("routes").append(tr);
  }
  $("visible-count").textContent =
    `${rows.length} of ${state.routes.length} routes`;
  $("empty").hidden = rows.length > 0;
  $("providers").replaceChildren();
  const providers = [
    ...new Set(
      state.routes.flatMap((row) => row.legs.map((leg) => leg.provider)),
    ),
  ];
  for (const provider of providers) {
    const node = el(
      "div",
      provider === "oneclick" ? "NEAR Intents / 1Click" : provider,
      "provider",
    );
    node.append(
      el(
        "small",
        state.mode === "demo"
          ? "Constructed fixture · no provider request"
          : "Observation received · freshness checked per route",
      ),
    );
    $("providers").append(node);
  }
  for (const error of state.errors) {
    const node = el(
      "div",
      `${error.provider} · ${error.code}`,
      "provider error",
    );
    node.append(el("small", error.message));
    $("providers").append(node);
  }
  if (!providers.length && !state.errors.length)
    $("providers").append(el("p", "No provider observations yet."));
  $("orders").replaceChildren();
  for (const order of state.orders) {
    const node = el("div", undefined, "order"),
      name = el("div", order.assessment.label);
    name.append(
      el(
        "small",
        `${new Date(order.created_at * 1000).toLocaleString()} · ${order.key}`,
      ),
    );
    node.append(
      name,
      el("span", order.status.replaceAll("_", " "), "chip"),
      el("small", "Simulation · realized P&L unavailable"),
    );
    $("orders").append(node);
  }
  $("cycles").replaceChildren();
  for (const cycle of state.cycles || []) {
    const row = el("div", undefined, "order");
    row.append(
      el("b", cycle.id),
      el("span", cycle.phase.replaceAll("_", " "), "chip"),
      el(
        "small",
        `${cycle.completed_legs.length}/2 legs · ${cycle.paper_profit_units === null ? "P&L not closed" : units(cycle.paper_profit_units, cycle.profit_asset?.decimals ?? 6) + " simulated source-asset profit"}`,
      ),
    );
    $("cycles").append(row);
  }
  if (!(state.cycles || []).length)
    $("cycles").append(
      el("p", "Inspect a demo route to run both legs automatically.", "empty"),
    );
  if (!state.orders.length)
    $("orders").append(
      el(
        "p",
        "No paper orders yet. Inspect a route to project its outcome.",
        "empty",
      ),
    );
}
function inspect(row) {
  selected = row;
  pendingKey = crypto.randomUUID();
  $("dialog-result").textContent = "";
  $("detail-title").textContent = row.label;
  const body = $("detail-body");
  body.replaceChildren();
  row.legs.forEach((leg, index) => {
    const node = el("article", undefined, "leg");
    node.append(
      el(
        "h3",
        `${String(index + 1).padStart(2, "0")} · ${chain(leg.asset_in.network)} → ${chain(leg.asset_out.network)}`,
      ),
      el(
        "small",
        `Input ${units(leg.amount_in, leg.asset_in.decimals, 8)} → quoted floor ${units(leg.minimum_out, leg.asset_out.decimals, 8)}`,
      ),
      el("small", `${leg.asset_in.asset_id} → ${leg.asset_out.asset_id}`),
      el(
        "small",
        `${leg.provider} · ${leg.evidence} · expires ${new Date(leg.expires_at * 1000).toLocaleTimeString()}`,
      ),
    );
    body.append(node);
  });
  const detail = el("div", undefined, "breakdown");
  detail.append(
    el("span", "External cost estimate"),
    el("b", units(row.external_cost_units, row.numeraire.decimals)),
    el("span", "Projected net profit"),
    el("b", signed(row.net_profit_units, row.numeraire.decimals)),
  );
  body.append(detail, el("p", row.cost_source), el("p", row.risk, "risk"));
  if (row.reasons.length)
    body.append(el("p", row.reasons.join(" · ").replaceAll("_", " "), "risk"));
  $("export-leg").replaceChildren();
  row.legs.forEach((leg, i) => {
    const option = el(
      "option",
      `${i + 1} · ${chain(leg.asset_in.network)} → ${chain(leg.asset_out.network)}`,
    );
    option.value = i;
    $("export-leg").append(option);
  });
  $("export-form").hidden = row.kind !== "cross-chain";
  $("paper").disabled = !row.paper_eligible;
  $("auto-paper").disabled = !row.paper_eligible || !row.demo;
  $("auto-programs").hidden = true;
  $("refund").value = "";
  $("recipient").value = "";
  $("mandate").value = "";
  $("detail").showModal();
}
async function update() {
  state = await request("/api/state");
  render();
}
for (const id of ["search", "kind", "edge", "eligible"])
  $(id).addEventListener("input", () => state && render());
$("close-detail").addEventListener("click", () => $("detail").close());
$("refresh").addEventListener("click", async () => {
  try {
    await request("/api/refresh", {});
    await update();
  } catch (error) {
    $("notice").textContent = error.message;
  }
});
$("paper").addEventListener("click", async () => {
  $("paper").disabled = true;
  try {
    const order = await request("/api/paper", {
      route_id: selected.id,
      key: pendingKey,
      scenario: $("scenario").value,
    });
    $("dialog-result").textContent =
      `${order.status.replaceAll("_", " ")} — simulation only; no transactions submitted.`;
    await update();
  } catch (error) {
    $("dialog-result").textContent = error.message;
  } finally {
    $("paper").disabled = !selected.paper_eligible;
  }
});
$("auto-paper").addEventListener("click", async () => {
  $("auto-paper").disabled = true;
  try {
    const result = await request("/api/trade", {
      route_id: selected.id,
      key: "auto-" + pendingKey,
      scenario: $("auto-scenario").value,
    });
    $("dialog-result").textContent =
      `${result.phase} · ${result.completed_legs.length}/2 legs · no real funds moved.`;
    $("auto-source").textContent = result.programs.join("\n\n// NEXT LEG\n\n");
    $("auto-programs").hidden = false;
    await update();
  } catch (error) {
    $("dialog-result").textContent = error.message;
  } finally {
    $("auto-paper").disabled = !selected.paper_eligible || !selected.demo;
  }
});
$("auto-scenario").addEventListener("change", () => {
  pendingKey = crypto.randomUUID();
});
$("scenario").addEventListener("change", () => {
  pendingKey = crypto.randomUUID();
});
$("export-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    const result = await request("/api/export", {
      route_id: selected.id,
      leg: Number($("export-leg").value),
      refund_to: $("refund").value,
      recipient: $("recipient").value,
      mandate_owner: $("mandate").value,
    });
    const url = URL.createObjectURL(
        new Blob([result.source], { type: "text/plain" }),
      ),
      link = el("a");
    link.href = url;
    link.download = result.filename;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    $("dialog-result").textContent =
      "Compiled .x3 leg downloaded. Inspect it and re-quote before execution.";
  } catch (error) {
    $("dialog-result").textContent = error.message;
  }
});
update().catch((error) => {
  $("notice").textContent = `Scanner unavailable: ${error.message}`;
});
setInterval(
  () =>
    update().catch(() => {
      $("notice").textContent =
        "Connection lost. Displayed observations may be stale; reconnect before acting.";
    }),
  5000,
);
