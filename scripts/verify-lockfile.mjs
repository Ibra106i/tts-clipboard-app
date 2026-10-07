#!/usr/bin/env node
// Supply-chain guard for package-lock.json.
//
// `npm ci` installs exactly what the lockfile says, which is the whole point of
// committing it - and also the whole risk. The lockfile is a file in the repo,
// so a pull request can edit it, and an edit that points a package at a
// different registry or removes its integrity hash turns `npm ci` into an
// install of whatever that registry serves. Reviewers read diffs; they do not
// read 176 `resolved` URLs.
//
// This script reads them. It asserts that every dependency still comes from the
// expected registry, over https, with a hash npm will actually verify.
//
// Usage: node scripts/verify-lockfile.mjs [path-to-lockfile]
// Exits 0 when the lockfile is intact, 1 with a list of offenders otherwise.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

/** The only registry a dependency may be fetched from. */
const ALLOWED_REGISTRY_HOST = "registry.npmjs.org";

/**
 * Hash algorithms npm will verify. sha1 predates the modern `integrity` field
 * and is not collision resistant, so a lockfile that offers only sha1 is
 * treated as unverified rather than accepted.
 */
const ALLOWED_INTEGRITY_PREFIXES = ["sha512-", "sha384-", "sha256-"];

const lockfilePath = resolve(process.argv[2] ?? "package-lock.json");

const problems = [];
const note = (message) => problems.push(message);

let lock;
try {
  lock = JSON.parse(readFileSync(lockfilePath, "utf8"));
} catch (error) {
  console.error(`Cannot read ${lockfilePath}: ${error.message}`);
  process.exit(1);
}

if (typeof lock.lockfileVersion !== "number" || lock.lockfileVersion < 3) {
  note(
    `lockfileVersion is ${lock.lockfileVersion}; version 3 or newer is required ` +
      `because earlier formats do not record every package's resolution.`,
  );
}

const entries = Object.entries(lock.packages ?? {}).filter(([path]) => path !== "");

if (entries.length === 0) {
  note("the lockfile describes no packages at all");
}

for (const [path, entry] of entries) {
  // Workspace and file links resolve to local paths by design.
  if (entry.link === true || typeof entry.resolved !== "string") {
    continue;
  }

  const resolved = entry.resolved;

  // npm's own tarball URLs are always https. A plain http URL is interceptable.
  let url;
  try {
    url = new URL(resolved);
  } catch {
    note(`${path}: resolved is not a URL: ${resolved}`);
    continue;
  }

  if (!resolved.startsWith("file:")) {
    if (url.protocol !== "https:") {
      note(`${path}: resolved is not https: ${resolved}`);
    }
    if (url.hostname !== ALLOWED_REGISTRY_HOST) {
      note(
        `${path}: resolved points at ${url.hostname}, not ${ALLOWED_REGISTRY_HOST}: ${resolved}`,
      );
    }
  }

  if (typeof entry.integrity !== "string" || entry.integrity === "") {
    note(`${path}: has no integrity hash, so npm cannot verify the tarball`);
    continue;
  }

  const algorithm = entry.integrity.split("-")[0] + "-";
  if (!ALLOWED_INTEGRITY_PREFIXES.some((prefix) => entry.integrity.startsWith(prefix))) {
    note(`${path}: integrity uses ${algorithm}, expected one of ${ALLOWED_INTEGRITY_PREFIXES.join(", ")}`);
  }
}

if (problems.length > 0) {
  console.error(`Lockfile verification failed for ${lockfilePath}:`);
  for (const problem of problems) {
    console.error(`  - ${problem}`);
  }
  console.error(
    `\n${problems.length} problem(s). Do not run npm ci against this lockfile.`,
  );
  process.exit(1);
}

console.log(
  `Lockfile OK: ${entries.length} package(s) all resolve from ${ALLOWED_REGISTRY_HOST} ` +
    `over https with a verifiable integrity hash.`,
);
