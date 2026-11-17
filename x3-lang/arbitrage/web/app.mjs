import { units, edge, matches, selectTokenPair, pairIssue } from "./format.mjs";
const $ = (id) => document.getElementById(id);
const chain = (value) =>
  ({
    sol: "Solana",
    near: "NEAR",
    eth: "Ethereum",
    arb: "Arbitrum",
    base: "Base",
    ethereum: "Ethereum",
    solana: "Solana",
    arbitrum: "Arbitrum",
    pol: "Polygon",
    bsc: "BSC",
    "cex-inventory": "Exchange inventory",
  })[value] || value;
let state, selected, pendingKey;
let selectedPair = {},
  strategyDraft = null,
  compiledDraft = null,
  strategyKey = null;
let strategyRevision = 0;
function renderSelection() {
  const active = !!(
    selectedPair.origin_asset || selectedPair.destination_asset
  );
  $("scanner-selection").hidden = !active;
  if (!active || !state.catalogue) return;
  const assets = state.catalogue.assets;
  const label = (id) => {
    const asset = assets.find((asset) => asset.asset_id === id);
    return asset
      ? `${asset.symbol || "Unnamed"} on ${chain(asset.network)}`
      : "Choose a token";
  };
  $("selected-pair").textContent =
    `${label(selectedPair.origin_asset)} → ${label(selectedPair.destination_asset)} → origin`;
  const issue = pairIssue(selectedPair, assets);
  $("selection-note").textContent =
    issue === "SAME_CHAIN"
      ? "Choose tokens on different chains."
      : issue
        ? "Complete both token selections."
        : "Pair selected. Set amount and quote addresses, then add the dry scan.";
  $("configure-selected").disabled = !!issue;
}
function chooseToken(asset, role) {
  const current = {
    origin_asset: $("route-origin").value,
    destination_asset: $("route-destination").value,
  };
  selectedPair = selectTokenPair(current, asset, role);
  $("route-origin").value = selectedPair.origin_asset;
  $("route-destination").value = selectedPair.destination_asset;
  renderTokens();
  renderSelection();
  if (!pairIssue(selectedPair, state.catalogue.assets)) {
    $("route-builder").scrollIntoView({ behavior: "smooth", block: "start" });
    $("route-amount").focus({ preventScroll: true });
  }
}

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
    const previousOrigin = $("route-origin").value,
      previousDestination = $("route-destination").value;
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
    if (catalogue.assets.some((asset) => asset.asset_id === previousOrigin))
      $("route-origin").value = previousOrigin;
    else if (origin) $("route-origin").value = origin.asset_id;
    if (
      catalogue.assets.some((asset) => asset.asset_id === previousDestination)
    )
      $("route-destination").value = previousDestination;
    else if (destination) $("route-destination").value = destination.asset_id;
  }
  const rows = catalogue.assets.filter(
    (asset) =>
      (!$("token-chain").value || asset.network === $("token-chain").value) &&
      `${asset.symbol} ${asset.asset_id} ${asset.contract_address || ""}`
        .toLowerCase()
        .includes($("token-search").value.toLowerCase()),
  );
  const active = document.activeElement;
  const focusAsset = active?.closest(".token-card")?.dataset.assetId,
    focusRole = active?.dataset.pairRole;
  $("token-list").replaceChildren();
  for (const asset of rows.slice(0, tokenLimit)) {
    const card = el("article", undefined, "token-card");
    card.dataset.assetId = asset.asset_id;
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
    const actions = el("div", undefined, "token-actions");
    for (const [role, text] of [
      ["origin_asset", "Start here"],
      ["destination_asset", "Scan via"],
    ]) {
      const button = el("button", text, "secondary");
      button.type = "button";
      button.dataset.pairRole = role;
      button.setAttribute(
        "aria-label",
        `${role === "origin_asset" ? "Start with" : "Scan via"} ${asset.symbol || "Unnamed"} on ${chain(asset.network)}`,
      );
      button.setAttribute(
        "aria-pressed",
        String(selectedPair[role] === asset.asset_id),
      );
      button.disabled = asset.deprecated;
      button.addEventListener("click", () => chooseToken(asset, role));
      actions.append(button);
    }
    card.append(actions);
    if (asset.limitation) card.append(el("small", asset.limitation));
    $("token-list").append(card);
  }
  if (focusAsset && focusRole) {
    const card = [...$("token-list").children].find(
      (card) => card.dataset.assetId === focusAsset,
    );
    const action =
      card &&
      [...card.querySelectorAll("button")].find(
        (button) => button.dataset.pairRole === focusRole,
      );
    action?.focus({ preventScroll: true });
  }
  if (!rows.length)
    $("token-list").append(
      el("p", "No listed assets match these filters.", "empty"),
    );
  $("more-tokens").hidden = rows.length <= tokenLimit;
  $("add-route").disabled =
    state.refreshing ||
    state.configured_route_count >= 4 ||
    !!pairIssue(
      {
        origin_asset: $("route-origin").value,
        destination_asset: $("route-destination").value,
      },
      catalogue.assets,
    );
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
  renderSelection();
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
    if (row.kind === "cross-chain") {
      const policy = el("button", ".x3 policy", "inspect x3-policy");
      policy.addEventListener("click", () => openStrategy(row));
      action.append(policy);
    }
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
  $("open-strategy").disabled = row.kind !== "cross-chain";
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
  if (state.pools && (!poolReport || state.pools.loaded_at !== poolReport.loaded_at))
    showPoolReport(state.pools);
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
function readStrategyDraft() {
  return {
    ...strategyDraft,
    mandates: [$("strategy-mandate-0").value, $("strategy-mandate-1").value],
    addresses: [
      $("strategy-origin-address").value,
      $("strategy-destination-address").value,
    ],
    minimum_profit: $("strategy-profit").value,
    max_external_cost: $("strategy-cost").value,
  };
}
function strategyChanged() {
  compiledDraft = null;
  strategyKey = crypto.randomUUID();
  $("run-strategy").disabled = true;
  $("strategy-programs").hidden = true;
  $("strategy-result").textContent =
    "Draft changed. Compile the .x3 mandates before paper execution.";
}
async function openStrategy(row) {
  const revision = ++strategyRevision;
  if ($("detail").open) $("detail").close();
  $("strategy-dialog").showModal();
  $("strategy-title").textContent = row.label;
  $("compile-strategy").disabled = true;
  $("run-strategy").disabled = true;
  $("strategy-result").textContent = "Preparing editable .x3 mandate sources…";
  $("strategy-programs").hidden = true;
  strategyDraft = null;
  compiledDraft = null;
  strategyKey = crypto.randomUUID();
  try {
    const result = await request("/api/strategy/template", {
      route_id: row.id,
    });
    if (revision !== strategyRevision) return;
    strategyDraft = result.draft;
    $("strategy-origin-address").value = strategyDraft.addresses[0];
    if ($("strategy-profit-unit"))
      $("strategy-profit-unit").textContent =
        `Minimum profit (${symbol(row.numeraire)})`;
    if ($("strategy-cost-unit"))
      $("strategy-cost-unit").textContent =
        `External cost ceiling (${symbol(row.numeraire)})`;
    $("strategy-destination-address").value = strategyDraft.addresses[1];
    $("strategy-mandate-0").value = strategyDraft.mandates[0];
    $("strategy-mandate-1").value = strategyDraft.mandates[1];
    $("strategy-profit").value = strategyDraft.minimum_profit;
    $("strategy-cost").value = strategyDraft.max_external_cost;
    $("strategy-result").textContent =
      "Edit the .x3 policy, then compile and validate both legs. No funds move.";
    $("compile-strategy").disabled = false;
  } catch (error) {
    if (revision !== strategyRevision) return;
    $("strategy-result").textContent = error.message;
  }
}
$("strategy-dialog").addEventListener("close", () => {
  strategyRevision += 1;
});
$("open-strategy").addEventListener("click", () => openStrategy(selected));
$("close-strategy").addEventListener("click", () =>
  $("strategy-dialog").close(),
);
for (const id of [
  "strategy-mandate-0",
  "strategy-mandate-1",
  "strategy-origin-address",
  "strategy-destination-address",
  "strategy-profit",
  "strategy-cost",
])
  $(id).addEventListener("input", strategyChanged);
$("compile-strategy").addEventListener("click", async () => {
  if (!strategyDraft) return;
  const revision = strategyRevision;
  const draft = readStrategyDraft(),
    submitted = JSON.stringify(draft);
  compiledDraft = null;
  $("compile-strategy").disabled = true;
  $("run-strategy").disabled = true;
  $("strategy-result").textContent =
    "Compiling .x3 mandates and checking both route legs…";
  try {
    const result = await request("/api/strategy/compile", { draft });
    if (revision !== strategyRevision) return;
    if (JSON.stringify(readStrategyDraft()) !== submitted)
      throw new Error(
        "Draft changed during compilation. Compile the current source again.",
      );
    compiledDraft = submitted;
    $("strategy-source").textContent = result.programs.join(
      "\n\n// NEXT LEG\n\n",
    );
    $("strategy-programs").hidden = false;
    $("run-strategy").disabled = false;
    $("strategy-result").textContent =
      "Compiled and validated both .x3 legs. Ready for paper execution; no wallet authorization.";
  } catch (error) {
    if (revision !== strategyRevision) return;
    $("strategy-result").textContent = error.message;
    $("strategy-programs").hidden = true;
  } finally {
    if (revision === strategyRevision) $("compile-strategy").disabled = false;
  }
});
$("run-strategy").addEventListener("click", async () => {
  const revision = strategyRevision;
  const draft = readStrategyDraft();
  if (JSON.stringify(draft) !== compiledDraft) return strategyChanged();
  $("run-strategy").disabled = true;
  try {
    const result = await request("/api/strategy/run", {
      draft,
      key: "x3-" + strategyKey,
      scenario: "success",
    });
    if (revision !== strategyRevision) return;
    $("strategy-result").textContent =
      JSON.stringify(readStrategyDraft()) === compiledDraft
        ? `${result.phase} · ${result.completed_legs.length}/2 legs under your .x3 mandates · no real funds moved.`
        : `${result.phase} for the submitted draft. Current edits need compilation; no real funds moved.`;
    await update();
  } catch (error) {
    if (revision !== strategyRevision) return;
    $("strategy-result").textContent = error.message;
  } finally {
    if (revision === strategyRevision)
      $("run-strategy").disabled =
        JSON.stringify(readStrategyDraft()) !== compiledDraft;
  }
});
$("configure-selected").addEventListener("click", () => {
  $("route-builder").scrollIntoView({ behavior: "smooth", block: "start" });
  $("route-amount").focus({ preventScroll: true });
});
$("clear-selected").addEventListener("click", () => {
  selectedPair = {};
  renderTokens();
  renderSelection();
});
$("route-destination").addEventListener("change", () => {
  selectedPair = {
    origin_asset: $("route-origin").value,
    destination_asset: $("route-destination").value,
  };
  renderTokens();
  renderSelection();
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
$("route-origin").addEventListener("change", () => {
  selectedPair = {
    origin_asset: $("route-origin").value,
    destination_asset: $("route-destination").value,
  };
  renderTokens();
  renderSelection();
});
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

let poolReport = null;
function renderPools() {
  if (!poolReport) return;
  const network = $("pool-chain").value;
  const search = $("pool-search").value.trim().toLowerCase();
  const rows = poolReport.pools.filter(pool => (!network || pool.network === network) &&
    [pool.dex, pool.pair_address, pool.base_token.symbol, pool.base_token.address,
      pool.quote_token.symbol, pool.quote_token.address].join(" ").toLowerCase().includes(search));
  $("pool-list").replaceChildren();
  for (const pool of rows) {
    const card = document.createElement("article");
    card.className = "pool-card";
    const title = document.createElement("h3");
    title.textContent = `${pool.base_token.symbol} / ${pool.quote_token.symbol}`;
    const venue = document.createElement("p");
    venue.textContent = `${chain(pool.network)} · ${pool.dex}`;
    const liquidity = document.createElement("p");
    liquidity.textContent = pool.liquidity_usd === null
      ? "USD liquidity unavailable · on-chain reserves observed"
      : `Reported liquidity: $${Number(pool.liquidity_usd).toLocaleString(undefined, {maximumFractionDigits: 0})}`;
    const identity = document.createElement("small");
    identity.textContent = `Pool: ${pool.pair_address}`;
    const assets = document.createElement("small");
    assets.textContent = `${pool.base_token.address} / ${pool.quote_token.address}`;
    const link = document.createElement("a");
    link.href = pool.source_url;
    link.target = "_blank";
    link.rel = "noopener noreferrer";
    link.textContent = "View pool source ↗";
    card.append(title, venue, liquidity, identity, assets, link);
    $("pool-list").append(card);
  }
  const errors = poolReport.errors.map(error => `${chain(error.network)}: ${error.message}`).join(" ");
  $("pool-status").textContent = `${rows.length} of ${poolReport.pools.length} pools · fetched ${new Date(poolReport.loaded_at * 1000).toLocaleTimeString()} · provider observations may lag · discovery only. ${errors}`;
}
function showPoolReport(report) {
  poolReport = report;
  const selected = $("pool-chain").value;
  $("pool-chain").replaceChildren(new Option("All chains", ""));
  for (const network of [...new Set(report.pools.map(pool => pool.network))].sort())
    $("pool-chain").append(new Option(chain(network), network));
  if ([...$("pool-chain").options].some(option => option.value === selected))
    $("pool-chain").value = selected;
  renderPools();
}
$("pool-chain").addEventListener("change", renderPools);
$("pool-search").addEventListener("input", renderPools);
$("load-pools").addEventListener("click", async () => {
  $("load-pools").disabled = true;
  $("pool-status").textContent = "Reading public pool sources…";
  try {
    showPoolReport(await request("/api/pools", {}));
  } catch (error) {
    $("pool-status").textContent = error.message;
  } finally {
    $("load-pools").disabled = false;
  }
});
