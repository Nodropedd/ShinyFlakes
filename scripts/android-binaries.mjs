// Packs Tor and the Monero wallet daemon into the Android build.
//
// On the desktop the app downloads both the first time they are used. Android
// will not run a file an app downloaded — since Android 10 an app's own data
// is mounted no-exec for it — but it will run one the package installer
// extracted from the APK's native-library folder. So both are fetched here, at
// build time, and dropped into jniLibs under lib*.so names; the manifest's
// extractNativeLibs puts them on disk as runnable files, and src-tauri's
// bundled.rs finds them there.
//
// Nothing is built or modified. Each archive is the project's own Android
// release, checked against a SHA-256 pinned below before anything is taken out
// of it, and the one file used is copied out byte for byte. The pins came from
// the projects' published lists:
//   https://archive.torproject.org/tor-package-archive/torbrowser/15.0.22/sha256sums-signed-build.txt
//   https://www.getmonero.org/downloads/hashes.txt
//
// Runs as part of `tauri android build` / `tauri android dev` and does nothing
// for any other platform, so desktop builds never touch the network for it.
// Downloads are cached under src-tauri/target, so only the first build pays.

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  createReadStream,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const TOR = "15.0.22";
const MONERO = "v0.18.5.1";
const TOR_BASE = `https://archive.torproject.org/tor-package-archive/torbrowser/${TOR}`;
const MONERO_BASE = "https://downloads.getmonero.org/cli";

// Tor's Android bundle already names its executable libTor.so for exactly
// this reason. Monero publishes no x86 Android build, so x86_64 carries Tor
// alone and the app says why Monero is missing there.
const BINARIES = {
  "arm64-v8a": [
    {
      name: "libtor.so",
      url: `${TOR_BASE}/tor-expert-bundle-android-aarch64-${TOR}.tar.gz`,
      sha256: "ed4bc23065ee10f68efcdae63ea318ffa4b02b04ba00f13a3f59f8e3832fdfad",
      member: "tor/libTor.so",
    },
    {
      name: "libmonero_wallet_rpc.so",
      url: `${MONERO_BASE}/monero-android-armv8-${MONERO}.tar.bz2`,
      sha256: "a2c0fb240c5eaa947f5a2382ece4613c59b299645ad4d1480ef24e71b8aa8c8f",
      member: "/monero-wallet-rpc",
    },
  ],
  "armeabi-v7a": [
    {
      name: "libtor.so",
      url: `${TOR_BASE}/tor-expert-bundle-android-armv7-${TOR}.tar.gz`,
      sha256: "2bf7d66307db90fc3f76ca0d412723de9e37755454cbeb22d806a3b4c9c22595",
      member: "tor/libTor.so",
    },
    {
      name: "libmonero_wallet_rpc.so",
      url: `${MONERO_BASE}/monero-android-armv7-${MONERO}.tar.bz2`,
      sha256: "daa56844251a9e9f296caaaafcf72c60dade54ae93146085d627ffc883b0fec3",
      member: "/monero-wallet-rpc",
    },
  ],
  x86_64: [
    {
      name: "libtor.so",
      url: `${TOR_BASE}/tor-expert-bundle-android-x86_64-${TOR}.tar.gz`,
      sha256: "88c8c6fe3d11db84bb6ea90bae801419f39b4650aac80baeff38528a82d3fd91",
      member: "tor/libTor.so",
    },
  ],
};

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const cache = join(root, "src-tauri", "target", "android-binaries");
const jniLibs = join(root, "src-tauri", "gen", "android", "app", "src", "main", "jniLibs");

if (process.env.TAURI_ENV_PLATFORM !== "android" && !process.argv.includes("--force")) {
  process.exit(0);
}

function sha256(path) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    createReadStream(path)
      .on("data", (chunk) => hash.update(chunk))
      .on("end", () => resolve(hash.digest("hex")))
      .on("error", reject);
  });
}

async function download(url, to) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`${url}: HTTP ${response.status}`);
  }
  // Written beside the target and renamed, so an interrupted download never
  // leaves a partial file under the final name to be mistaken for a good one.
  const partial = `${to}.partial`;
  writeFileSync(partial, Buffer.from(await response.arrayBuffer()));
  renameSync(partial, to);
}

/** The verified archive, downloading it only if the cached copy is missing or wrong. */
async function archive(entry) {
  const path = join(cache, entry.url.split("/").pop());
  if (existsSync(path) && (await sha256(path)) === entry.sha256) {
    return path;
  }
  console.log(`android-binaries: downloading ${entry.url}`);
  await download(entry.url, path);
  const got = await sha256(path);
  if (got !== entry.sha256) {
    rmSync(path, { force: true });
    throw new Error(
      `${entry.url} did not match its pinned SHA-256 and was deleted.\n` +
        `  expected ${entry.sha256}\n  got      ${got}`,
    );
  }
  return path;
}

function tar(args) {
  const run = spawnSync("tar", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  if (run.status !== 0) {
    throw new Error(`tar ${args.join(" ")}: ${run.stderr || run.error}`);
  }
  return run.stdout;
}

/** The one file wanted from an archive, extracted into the cache once. */
async function extracted(entry) {
  const out = join(cache, entry.sha256, entry.name);
  if (existsSync(out)) {
    return out;
  }
  const from = await archive(entry);

  // Monero nests everything under a folder named for the build, which
  // differs per architecture, so match on the file name rather than guess it.
  const members = tar(["-tf", from]).split("\n");
  const member = members.find((m) =>
    entry.member.startsWith("/") ? m.endsWith(entry.member) : m === entry.member,
  );
  if (!member) {
    throw new Error(`${entry.member} is not in ${from}`);
  }

  const scratch = mkdtempSync(join(cache, "extract-"));
  try {
    tar(["-xf", from, "-C", scratch, member]);
    mkdirSync(dirname(out), { recursive: true });
    renameSync(join(scratch, member), out);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
  return out;
}

mkdirSync(cache, { recursive: true });

for (const [abi, entries] of Object.entries(BINARIES)) {
  for (const entry of entries) {
    const source = await extracted(entry);
    const target = join(jniLibs, abi, entry.name);
    mkdirSync(dirname(target), { recursive: true });

    const same =
      existsSync(target) && readFileSync(target).equals(readFileSync(source));
    if (!same) {
      copyFileSync(source, target);
      chmodSync(target, 0o755);
      console.log(`android-binaries: ${abi}/${entry.name}`);
    }
  }
}
