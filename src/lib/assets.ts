// Coin metadata and amount formatting.

import type { AssetId } from "./ipc";

import btcLogo from "../assets/coins/btc.svg";
import ltcLogo from "../assets/coins/ltc.svg";

import xmrLogo from "../assets/coins/xmr.svg";
import ethLogo from "../assets/coins/eth.svg";
import solLogo from "../assets/coins/sol.svg";
import trxLogo from "../assets/coins/trx.svg";
import usdcLogo from "../assets/coins/usdc.svg";
import usdtLogo from "../assets/coins/usdt.svg";

export interface AssetMeta {
  id: AssetId;
  name: string;

  ticker: string;

  logo: string;

  decimals: number;

  utxo: boolean;

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
