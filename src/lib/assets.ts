// Static asset registry. Names, symbols, precision and accent colours are
// fixed properties of each chain, so they live here rather than crossing the
// IPC bridge. Balances are a separate matter and come from the core once
// chain connectivity exists.

import type { AssetId } from "./ipc";

// Coin marks bundled rather than hotlinked. Most come from the
// cryptocurrency-icons set (CC0 1.0, see assets/coins/LICENSE.md); Monero and
// Solana use their own current brand marks instead, since the set carries
// older versions of both.
import btcLogo from "../assets/coins/btc.svg";
import ltcLogo from "../assets/coins/ltc.svg";
// Monero ships its mark as artwork rather than a path set, so the official
// file is used directly. It is the current two-tone version.
import xmrLogo from "../assets/coins/xmr.png";
import ethLogo from "../assets/coins/eth.svg";
import solLogo from "../assets/coins/sol.svg";
import trxLogo from "../assets/coins/trx.svg";
import usdcLogo from "../assets/coins/usdc.svg";
import usdtLogo from "../assets/coins/usdt.svg";

export interface AssetMeta {
  id: AssetId;
  name: string;
  /** Ticker shown next to amounts. */
  ticker: string;
  /** Bundled SVG logo. */
  logo: string;
  /** Decimal places between the smallest unit and one whole coin. */
  decimals: number;
  /** True for real UTXO chains, which are the only ones with a fragmentation
   *  view and per-output bucket tagging. */
  utxo: boolean;
  /** CSS custom property holding this asset's accent. */
  accent: string;
}

export const ASSETS: AssetMeta[] = [
  { id: "BTC", name: "Bitcoin", ticker: "BTC", logo: btcLogo, decimals: 8, utxo: true, accent: "var(--asset-btc)" },
  { id: "LTC", name: "Litecoin", ticker: "LTC", logo: ltcLogo, decimals: 8, utxo: true, accent: "var(--asset-ltc)" },
  { id: "XMR", name: "Monero", ticker: "XMR", logo: xmrLogo, decimals: 12, utxo: false, accent: "var(--asset-xmr)" },
  { id: "ETH", name: "Ethereum", ticker: "ETH", logo: ethLogo, decimals: 18, utxo: false, accent: "var(--asset-eth)" },
  { id: "SOL", name: "Solana", ticker: "SOL", logo: solLogo, decimals: 9, utxo: false, accent: "var(--asset-sol)" },
  { id: "TRON", name: "Tron", ticker: "TRX", logo: trxLogo, decimals: 6, utxo: false, accent: "var(--asset-tron)" },
  { id: "USDC", name: "USD Coin", ticker: "USDC", logo: usdcLogo, decimals: 6, utxo: false, accent: "var(--asset-usdc)" },
  { id: "USDT", name: "Tether", ticker: "USDT", logo: usdtLogo, decimals: 6, utxo: false, accent: "var(--asset-usdt)" },
];

export const BY_ID: Record<AssetId, AssetMeta> = Object.fromEntries(
  ASSETS.map((a) => [a.id, a]),
) as Record<AssetId, AssetMeta>;

/** Formats a smallest-unit amount for display. Input is a decimal string
 *  because satoshi and atomic-unit counts exceed what a JS number holds
 *  exactly. Trailing zeros are trimmed, but at least two places are kept so
 *  amounts line up in a column. */
export function formatAmount(minor: string, decimals: number): string {
  const negative = minor.startsWith("-");
  const digits = (negative ? minor.slice(1) : minor).replace(/\D/g, "") || "0";
  const padded = digits.padStart(decimals + 1, "0");
  const whole = padded.slice(0, padded.length - decimals);
  let fraction = decimals > 0 ? padded.slice(padded.length - decimals) : "";

  fraction = fraction.replace(/0+$/, "");
  while (fraction.length < 2) fraction += "0";

  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return `${negative ? "-" : ""}${grouped}.${fraction}`;
}

/** Whole units as typed by a person into smallest units, without floats.
 *  "0.001" at 9 decimals becomes "1000000". Throws on anything that is not a
 *  plain decimal number or that carries more precision than the asset has. */
export function toMinor(input: string, decimals: number): string {
  const text = input.trim();
  if (!/^\d*(\.\d*)?$/.test(text) || text === "" || text === ".") {
    throw new Error("Enter an amount like 0.5");
  }
  const [whole, fraction = ""] = text.split(".");
  if (fraction.length > decimals) {
    throw new Error(`At most ${decimals} decimal places for this asset`);
  }
  const padded = fraction.padEnd(decimals, "0");
  const joined = `${whole}${padded}`.replace(/^0+(?=\d)/, "");
  return joined === "" ? "0" : joined;
}
