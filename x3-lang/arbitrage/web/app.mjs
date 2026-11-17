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
let tokenLimit = 12,
  renderedCatalogue = null;
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
function renderTokens() {
  const catalogue = state.catalogue;
  if (!catalogue) return;
  $("token-summary").textContent =
    `${catalogue.asset_count} provider-listed assets across ${Object.keys(catalogue.chains).length} chains · no execution approval`;
  if (renderedCatalogue !== catalogue.loaded_at) {
    renderedCatalogue = catalogue.loaded_at;
    const allChains = el("option", "All provider chains");
    allChains.value = "";
    $("token-chain").replaceChildren(allChains);
    for (const network of Object.keys(catalogue.chains).sort()) {
      const option = el(
        "option",
        `${chain(network)} (${catalogue.chains[network]})`,
      );
      option.value = network;
      $("token-chain").append(option);
    }
    for (const id of ["route-origin", "route-destination"]) {
      $(id).replaceChildren();
      for (const asset of catalogue.assets.filter(
        (asset) => !asset.deprecated,
      )) {
        const option = el(
          "option",
          `${asset.symbol || "Unnamed"} · ${chain(asset.network)} · ${asset.decimals} decimals`,
        );
        option.value = asset.asset_id;
        $(id).append(option);
      }
    }
    const origin = catalogue.assets.find(
      (asset) => asset.network === "sol" && asset.symbol === "USDC",
    );
    const destination = catalogue.assets.find(
      (asset) => asset.network === "near" && asset.symbol === "USDC",
    );
    if (origin) $("route-origin").value = origin.asset_id;
    if (destination) $("route-destination").value = destination.asset_id;
  }
  const rows = catalogue.assets.filter(
    (asset) =>
      (!$("token-chain").value || asset.network === $("token-chain").value) &&
      `${asset.symbol} ${asset.asset_id} ${asset.contract_address || ""}`
        .toLowerCase()
        .includes($("token-search").value.toLowerCase()),
  );
  $("token-list").replaceChildren();
  for (const asset of rows.slice(0, tokenLimit)) {
    const card = el("article", undefined, "token-card");
    const id = el("code", asset.asset_id);
    id.title = asset.asset_id;
    card.append(
      el("b", `${asset.symbol || "Unnamed asset"} · ${chain(asset.network)}`),
      el(
        "small",
        `${asset.decimals} decimals · ${asset.deprecated ? "DEPRECATED" : asset.x3_export_candidate ? ".x3 export candidate" : "Discovery only"}`,
      ),
      id,
    );
    if (asset.limitation) card.append(el("small", asset.limitation));
    $("token-list").append(card);
  }
  if (!rows.length)
    $("token-list").append(
      el("p", "No listed assets match these filters.", "empty"),
    );
  $("more-tokens").hidden = rows.length <= tokenLimit;
  $("add-route").disabled =
    state.refreshing || state.configured_route_count >= 4;
  const start = catalogue.assets.find(
    (asset) => asset.asset_id === $("route-origin").value,
  );
  if (start) {
    $("route-amount-label").textContent =
      `Input amount (${start.symbol} · ${start.decimals} decimals)`;
    $("route-cost-label").textContent =
      `Both-leg external costs (${start.symbol}, optional)`;
  }
}
function render() {
  renderTokens();
  $("mode").textContent =
    state.mode === "demo"
      ? "CONSTRUCTED DEMO"
      : state.mode === "mixed"
        ? "DEMO + LIVE DISCOVERY"
        : "LIVE OBSERVATIONS";
  $("notice").textContent =
    state.mode === "demo"
      ? "DEMO DATA — Constructed opportunities for exploring the workflow. Refresh regenerates examples; these are not market prices."
      : state.mode === "mixed"
        ? "MIXED WORKSPACE — Demo rows stay labeled; configured routes use live dry quotes. No funds move."
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
$("load-tokens").addEventListener("click", async () => {
  $("load-tokens").disabled = true;
  $("token-summary").textContent = "Fetching the public provider catalogue…";
  try {
    await request("/api/catalogue", {});
    await update();
  } catch (error) {
    $("token-summary").textContent = error.message;
  } finally {
    $("load-tokens").disabled = false;
  }
});
for (const id of ["token-search", "token-chain"])
  $(id).addEventListener("input", () => {
    tokenLimit = 12;
    renderTokens();
  });
$("route-origin").addEventListener("change", renderTokens);
$("more-tokens").addEventListener("click", () => {
  tokenLimit += 24;
  renderTokens();
});
$("route-builder").addEventListener("submit", async (event) => {
  event.preventDefault();
  $("add-route").disabled = true;
  try {
    await request("/api/routes", {
      origin_asset: $("route-origin").value,
      destination_asset: $("route-destination").value,
      amount: $("route-amount").value,
      origin_address: $("route-source-address").value,
      destination_address: $("route-destination-address").value,
      external_cost: $("route-cost").value,
      cost_source: $("route-cost-source").value,
    });
    $("route-result").textContent =
      "Route added. Requesting signed dry quotes; no deposit address or funds requested.";
    await update();
  } catch (error) {
    $("route-result").textContent = error.message;
    $("add-route").disabled = false;
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
