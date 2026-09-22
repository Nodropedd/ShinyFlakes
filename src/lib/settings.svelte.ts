// Settings state.

export const CURRENCIES = [
  { code: "usd", symbol: "$", label: "US Dollar" },
  { code: "eur", symbol: "\u20AC", label: "Euro" },
  { code: "gbp", symbol: "\u00A3", label: "British Pound" },
  { code: "chf", symbol: "CHF", label: "Swiss Franc" },
  { code: "jpy", symbol: "\u00A5", label: "Japanese Yen" },
  { code: "cad", symbol: "CA$", label: "Canadian Dollar" },
  { code: "aud", symbol: "A$", label: "Australian Dollar" },
  { code: "sek", symbol: "kr", label: "Swedish Krona" },
] as const;

export type CurrencyCode = (typeof CURRENCIES)[number]["code"];

const KEY = "shinyflakes.currency";
const DENOM_KEY = "shinyflakes.denominate";
const THEME_KEY = "shinyflakes.theme";
const ACCENT_KEY = "shinyflakes.accent";
const MONERO_KEY = "shinyflakes.moneroEndpoint";
const MONERO_DAEMON_KEY = "shinyflakes.moneroDaemon";
const TOR_KEY = "shinyflakes.tor";

export const DEFAULT_MONERO_DAEMON = "xmr-node.cakewallet.com:18081";

export const DEFAULT_MONERO_ENDPOINT = "http://127.0.0.1:18082/json_rpc";

export type Theme = "system" | "light" | "dark";

export const THEMES: { value: Theme; label: string }[] = [
  { value: "system", label: "Match the system" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

function read(key: string, fallback: string) {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {

  }
}

class Settings {
  currency = $state<CurrencyCode>(
    (CURRENCIES.find((c) => c.code === read(KEY, "usd"))?.code ??
      "usd") as CurrencyCode,
  );

  denominate = $state<"coin" | "fiat">(
    read(DENOM_KEY, "coin") === "fiat" ? "fiat" : "coin",
  );

  theme = $state<Theme>(
    (THEMES.find((t) => t.value === read(THEME_KEY, "dark"))?.value ??
      "dark") as Theme,
  );

  accent = $state<string | null>(read(ACCENT_KEY, "") || null);

  moneroEndpoint = $state<string>(read(MONERO_KEY, ""));

  get moneroReady() {
    return this.moneroEndpoint.trim().length > 0;
  }

  setMoneroEndpoint(endpoint: string) {
    this.moneroEndpoint = endpoint.trim();
    write(MONERO_KEY, this.moneroEndpoint);
  }

  moneroDaemon = $state<string>(read(MONERO_DAEMON_KEY, DEFAULT_MONERO_DAEMON));

  setMoneroDaemon(daemon: string) {
    this.moneroDaemon = daemon.trim() || DEFAULT_MONERO_DAEMON;
    write(MONERO_DAEMON_KEY, this.moneroDaemon);
  }

  torEnabled = $state<boolean>(read(TOR_KEY, "no") === "yes");

  setTorEnabled(on: boolean) {
    this.torEnabled = on;
    write(TOR_KEY, on ? "yes" : "no");
  }

  apply() {
    const root = document.documentElement;
    if (this.theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", this.theme);

    if (this.accent) root.style.setProperty("--accent", this.accent);
    else root.style.removeProperty("--accent");
  }

  setTheme(theme: Theme) {
    this.theme = theme;
    write(THEME_KEY, theme);
    this.apply();
  }

  setAccent(accent: string | null) {
    this.accent = accent;
    write(ACCENT_KEY, accent ?? "");
    this.apply();
  }

  get meta() {
    return CURRENCIES.find((c) => c.code === this.currency) ?? CURRENCIES[0];
  }

  setCurrency(code: CurrencyCode) {
    this.currency = code;
    write(KEY, code);
  }

  toggleDenomination() {
    this.denominate = this.denominate === "coin" ? "fiat" : "coin";
    write(DENOM_KEY, this.denominate);
  }

  money(value: number) {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency: this.currency.toUpperCase(),

      maximumFractionDigits: this.currency === "jpy" ? 0 : 2,
    }).format(value);
  }
}

export const settings = new Settings();
