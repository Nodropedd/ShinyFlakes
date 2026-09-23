// Prepares the files of a release for publishing.
//
//   node scripts/site-release.mjs <folder>
//
// <folder> holds the builds under their public names (NAMES below). This
// writes SHA256SUMS.txt beside them and docs/release.json, which the download
// page reads to show each file's size and checksum. The page links to
// releases/latest/download/<name>, so the names never change between
// versions and a new release needs no edits to the page itself.

import { createHash } from "node:crypto";
import { createReadStream, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// Stable keys the page uses. The APK gets its version in its file name, so
// repeated downloads on a phone never pile up under one name.
const KEYS = [
  "ShinyFlakes-android.apk",
  "ShinyFlakes-windows-x64-setup.exe",
  "ShinyFlakes-windows-x64-portable.exe",
  "ShinyFlakes-linux-x86_64.AppImage",
  "ShinyFlakes-linux-amd64.deb",
  "ShinyFlakes-linux-x86_64.rpm",
  "ShinyFlakes-arch-x86_64.pkg.tar.zst",
];

const REPO = "Nodropedd/ShinyFlakes";
const folder = process.argv[2];
if (!folder) {
  console.error("usage: node scripts/site-release.mjs <folder with the release files>");
  process.exit(1);
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const conf = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const tag = `v${conf.version}`;
const fileFor = (key) =>
  key === "ShinyFlakes-android.apk" ? `ShinyFlakes-android-${conf.version}.apk` : key;

const sha256 = (path) =>
  new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    createReadStream(path)
      .on("data", (chunk) => hash.update(chunk))
      .on("end", () => resolve(hash.digest("hex")))
      .on("error", reject);
  });

const assets = [];
for (const key of KEYS) {
  const name = fileFor(key);
  const path = join(folder, name);
  let size;
  try {
    size = statSync(path).size;
  } catch {
    console.error(`missing: ${name}`);
    process.exit(1);
  }
  assets.push({
    key,
    name,
    size,
    sha256: await sha256(path),
    url: `https://github.com/${REPO}/releases/download/${tag}/${name}`,
  });
}

writeFileSync(
  join(folder, "SHA256SUMS.txt"),
  assets.map((a) => `${a.sha256}  ${a.name}`).join("\n") + "\n",
);
writeFileSync(
  join(root, "docs", "release.json"),
  JSON.stringify(
    {
      version: conf.version,
      tag,
      date: new Date().toISOString().slice(0, 10),
      assets,
    },
    null,
    2,
  ) + "\n",
);

for (const a of assets) console.log(`${a.sha256}  ${a.name}`);
