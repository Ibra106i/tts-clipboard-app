// Which dropped or picked files this application can import.

/** Extensions the backend accepts. Keep in sync with `library::import_book`. */
export const SUPPORTED_EXTENSIONS = ["pdf", "epub"] as const;

/** Lower-cased extension without the dot, or `""` when there is none. */
export function extensionOf(fileName: string): string {
  const parts = fileName.toLowerCase().split(".");
  return parts.length > 1 ? (parts.at(-1) ?? "") : "";
}

/** True when the backend will accept this file name. */
export function isSupportedFile(fileName: string): boolean {
  return (SUPPORTED_EXTENSIONS as readonly string[]).includes(
    extensionOf(fileName),
  );
}

/** Last path segment of a Windows or POSIX path. */
export function fileNameOf(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}
