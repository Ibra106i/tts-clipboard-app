// Self-test for scripts/verify-lockfile.mjs.
//
// A supply-chain gate that only ever passes is worse than no gate, because it
// reads as coverage. This test takes the real lockfile, introduces one specific
// violation at a time, and asserts the verifier rejects each one - and accepts
// the untouched copy. Without this, a refactor that accidentally disabled a
// check would look like a passing build.
//
// Exits 0 when every case behaves as expected, 1 otherwise.

import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const verifier = resolve("scripts/verify-lockfile.mjs");
const source = JSON.parse(readFileSync("package-lock.json", "utf8"));

/** A dependency that definitely carries a resolved URL. */
const targetKey = Object.keys(source.packages).find((key) => {
  const entry = source.packages[key];
  return key !== "" && typeof entry?.resolved === "string";
});

if (!targetKey) {
  console.error("no package with a resolved URL in package-lock.json");
  process.exit(1);
}

const originalResolved = source.packages[targetKey].resolved;

/** Each case: a name, whether the verifier should reject it, and the mutation. */
const cases = [
  {
    name: "an untouched lockfile is accepted",
    expectRejected: false,
    mutate: () => {},
  },
  {
    name: "a dependency from an unexpected registry is rejected",
    expectRejected: true,
    mutate: (lock) => {
      lock.packages[targetKey].resolved =
        "https://evil.example.com/pkg/-/pkg-1.0.0.tgz";
    },
  },
  {
    name: "a dependency served over plain http is rejected",
    expectRejected: true,
    mutate: (lock) => {
      lock.packages[targetKey].resolved = originalResolved.replace(
        "https://",
        "http://",
      );
    },
  },
  {
    name: "a dependency with no integrity hash is rejected",
    expectRejected: true,
    mutate: (lock) => {
      delete lock.packages[targetKey].integrity;
    },
  },
  {
    name: "a dependency with only a sha1 hash is rejected",
    expectRejected: true,
    mutate: (lock) => {
      lock.packages[targetKey].integrity = `sha1-${"a".repeat(40)}`;
    },
  },
  {
    name: "an outdated lockfile version is rejected",
    expectRejected: true,
    mutate: (lock) => {
      lock.lockfileVersion = 2;
    },
  },
];

const dir = mkdtempSync(join(tmpdir(), "verify-lockfile-test-"));
let failures = 0;

try {
  for (const testCase of cases) {
    const lock = JSON.parse(JSON.stringify(source));
    testCase.mutate(lock);
    const path = join(dir, "lock.json");
    writeFileSync(path, JSON.stringify(lock, null, 2));

    let rejected;
    try {
      execFileSync(process.execPath, [verifier, path], { stdio: "pipe" });
      rejected = false;
    } catch {
      rejected = true;
    }

    if (rejected === testCase.expectRejected) {
      console.log(`  ok    ${testCase.name}`);
    } else {
      console.error(
        `  FAIL  ${testCase.name} (expected ${testCase.expectRejected ? "rejection" : "acceptance"}, got the opposite)`,
      );
      failures += 1;
    }
  }
} finally {
  rmSync(dir, { recursive: true, force: true });
}

if (failures > 0) {
  console.error(`\n${failures} case(s) failed.`);
  process.exit(1);
}

console.log(`\nAll ${cases.length} cases behaved as expected.`);
