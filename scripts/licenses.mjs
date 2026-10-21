// Liste des composants tiers et de leurs licences, affichée dans Paramètres, À propos.
// À relancer quand les dépendances changent : npm run licenses
// Sources : `cargo metadata` (bibliothèques Rust), package-lock.json (paquets npm de
// l'interface), textes officiels SPDX (téléchargés une fois, gardés dans le fichier produit).
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync } from "node:fs";

const OUT = "src/lib/third-party.json";

// Seulement ce qui est compilé dans l'appli distribuée, pour chaque plateforme construite :
// ni outils de compilation (build-dependencies, macros), ni composants d'une autre plateforme.
const TARGETS = [
  "x86_64-unknown-linux-gnu",
  "aarch64-linux-android",
  "armv7-linux-androideabi",
  "x86_64-linux-android",
  "x86_64-pc-windows-msvc",
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
];
const own = new Set(["librestfm", "librestfm-core"]);
const shipped = new Map();
for (const target of TARGETS) {
  const meta = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--locked", "--filter-platform", target], { maxBuffer: 1 << 28 }).toString(),
  );
  const byId = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const seen = new Set();
  const stack = meta.packages.filter((p) => p.name === "librestfm").map((p) => p.id);
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    const p = byId.get(id);
    if (p.targets.some((t) => t.kind.includes("proc-macro"))) continue;
    if (!own.has(p.name)) shipped.set(id, p);
    for (const d of nodes.get(id).deps) {
      if (d.dep_kinds.some((k) => k.kind === null)) stack.push(d.pkg);
    }
  }
}
const rust = [...shipped.values()]
  .filter((p) => !p.targets.some((t) => t.kind.includes("proc-macro")))
  .map((p) => ({ n: p.name, v: p.version, l: (p.license ?? "").replaceAll("/", " OR ") }));

const lock = JSON.parse(readFileSync("package-lock.json", "utf8"));
const npm = Object.entries(lock.packages)
  .filter(([k, v]) => k && !v.dev && !v.optional)
  .map(([k, v]) => ({ n: k.replace(/^.*node_modules\//, ""), v: v.version, l: v.license ?? "" }));

const packages = [...rust, ...npm].sort((a, b) => a.n.localeCompare(b.n) || a.v.localeCompare(b.v));

// Textes de chaque licence citée (sauf les alternatives jamais retenues seules).
const ids = new Set(packages.flatMap((p) => p.l.split(/[()\s]+|\bOR\b|\bAND\b/).filter((x) => x && !["OR", "AND", "WITH"].includes(x))));
const previous = existsSync(OUT) ? JSON.parse(readFileSync(OUT, "utf8")).texts : {};
const texts = {};
for (const id of [...ids].sort()) {
  if (previous[id]) {
    texts[id] = previous[id];
    continue;
  }
  const res = await fetch(`https://raw.githubusercontent.com/spdx/license-list-data/main/text/${id}.txt`);
  if (!res.ok) throw new Error(`texte de licence introuvable : ${id}`);
  texts[id] = (await res.text()).trim();
}

writeFileSync(OUT, JSON.stringify({ packages, texts }));
console.log(`${packages.length} composants, ${Object.keys(texts).length} licences → ${OUT}`);
