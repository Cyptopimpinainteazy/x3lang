export function units(value, decimals, places = 4) {
  if (value === null || value === undefined) return "Unknown";
  const number = BigInt(value),
    negative = number < 0n,
    absolute = negative ? -number : number;
  const scale = 10n ** BigInt(decimals),
    whole = absolute / scale;
  const fraction = (absolute % scale)
    .toString()
    .padStart(decimals, "0")
    .slice(0, places)
    .replace(/0+$/, "");
  return `${negative ? "−" : ""}${whole.toLocaleString("en-US")}${fraction ? `.${fraction}` : ""}`;
}
export function edge(bps) {
  return bps === null
    ? "Unknown"
    : `${bps > 0 ? "+" : ""}${(bps / 100).toFixed(2)}%`;
}
export function matches(
  row,
  { search = "", kind = "", minimum = "", eligible = false },
) {
  return (
    (!kind || row.kind === kind) &&
    (!eligible || row.paper_eligible) &&
    (minimum === "" ||
      (row.net_profit_bps !== null && row.net_profit_bps > Number(minimum))) &&
    JSON.stringify([row.label, row.legs])
      .toLowerCase()
      .includes(search.toLowerCase())
  );
}
