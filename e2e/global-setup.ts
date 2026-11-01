import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";

const upScript = path.resolve(
  __dirname,
  "../.claude/skills/verify/scripts/up.sh",
);

// Reuse an instance started elsewhere (E2E_BASE_URL) or start an ephemeral one
// with the same script the verify skill uses. FOLIO_BIN selects the binary.
export default async function globalSetup() {
  if (process.env.E2E_BASE_URL) {
    process.env.FOLIO_BASE_URL = process.env.E2E_BASE_URL;
    return;
  }
  const envFile = execFileSync(upScript, { encoding: "utf8" }).trim();
  const vars = Object.fromEntries(
    readFileSync(envFile, "utf8")
      .split("\n")
      .filter(Boolean)
      .map((line) => line.split(/=(.*)/s).slice(0, 2)),
  );
  process.env.FOLIO_BASE_URL = vars.BASE_URL;
  process.env.FOLIO_VERIFY_ENV = envFile;
}
