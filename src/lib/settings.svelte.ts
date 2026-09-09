// Display preferences. Purely local and cosmetic, so they live in browser
// storage rather than the encrypted vault.

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

/// Public node used until someone points this at their own.
export const DEFAULT_MONERO_DAEMON = "xmr-node.cakewallet.com:18081";

// Where monero-wallet-rpc listens out of the box.
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
    /* a lost preference costs nothing */
  }
}

class Settings {
  currency = $state<CurrencyCode>(
    (CURRENCIES.find((c) => c.code === read(KEY, "usd"))?.code ??
      "usd") as CurrencyCode,
  );

  /** Which unit leads in the asset list: the coin itself or its cash value. */
  denominate = $state<"coin" | "fiat">(
    read(DENOM_KEY, "coin") === "fiat" ? "fiat" : "coin",
  );

  theme = $state<Theme>(
    (THEMES.find((t) => t.value === read(THEME_KEY, "dark"))?.value ??
      "dark") as Theme,
  );

  /** Null means whatever the current theme defines, which is the default. */
  accent = $state<string | null>(read(ACCENT_KEY, "") || null);

  /** Empty until the user points the wallet at their Monero daemon. Monero
   *  stays read-only and unspendable until then. */
  moneroEndpoint = $state<string>(read(MONERO_KEY, ""));

  get moneroReady() {
    return this.moneroEndpoint.trim().length > 0;
  }

  setMoneroEndpoint(endpoint: string) {
    this.moneroEndpoint = endpoint.trim();
    write(MONERO_KEY, this.moneroEndpoint);
  }

  /** Which node the Monero wallet reads the chain from. Remembered, so
   *  starting automatically uses the one that was chosen rather than
   *  quietly falling back to the public default. */
  moneroDaemon = $state<string>(read(MONERO_DAEMON_KEY, DEFAULT_MONERO_DAEMON));

  setMoneroDaemon(daemon: string) {
    this.moneroDaemon = daemon.trim() || DEFAULT_MONERO_DAEMON;
    write(MONERO_DAEMON_KEY, this.moneroDaemon);
  }

  /** Pushes theme and accent onto the document. Called once at startup and
   *  again whenever either changes. */
  apply() {
    const root = document.documentElement;
    if (this.theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", this.theme);

    // An explicit accent overrides the theme default; clearing it hands
    // control back to the stylesheet rather than freezing a dark-theme colour
    // into the light one.
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

  /** Formats a cash amount in the chosen currency. */
  money(value: number) {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency: this.currency.toUpperCase(),
      // Yen has no minor unit; everything else here has two.
      maximumFractionDigits: this.currency === "jpy" ? 0 : 2,
    }).format(value);
  }
}

export const settings = new Settings();
