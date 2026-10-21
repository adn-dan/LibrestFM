// Android : le sélecteur de fichiers renvoie des adresses content://, que le cœur ne sait
// pas ouvrir. Lecture : copie locale d'abord ; écriture : fichier temporaire, puis recopie
// vers le fichier choisi. Sur PC, les chemins passent tels quels.
import { invoke } from "@tauri-apps/api/core";

const isUri = (p: string) => p.startsWith("content://");
const call = <T>(cmd: string, args: Record<string, unknown>) => invoke<T>("scrobbler_call", { cmd, args });

/** Chemin lisible par le cœur. */
export async function readable(p: string): Promise<string> {
  return isUri(p) ? (await call<{ path: string }>("localCopy", { uri: p })).path : p;
}

/** Écrit avec `write` dans le fichier choisi (name : nom du fichier temporaire, avec son extension). */
export async function writeTo<T>(p: string, name: string, write: (path: string) => Promise<T>): Promise<T> {
  if (!isUri(p)) return write(p);
  const { path } = await call<{ path: string }>("tempPath", { name });
  const r = await write(path);
  await call("copyToUri", { uri: p, from: path });
  return r;
}

/** Nom lisible d'un fichier choisi (une adresse content:// n'en dit pas plus). */
export function shownPath(p: string, name: string): string {
  return isUri(p) ? name : p;
}
