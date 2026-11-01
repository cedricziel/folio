import { execFileSync } from "node:child_process";
import path from "node:path";

const downScript = path.resolve(
  __dirname,
  "../.claude/skills/verify/scripts/down.sh",
);

export default async function globalTeardown() {
  const envFile = process.env.FOLIO_VERIFY_ENV;
  if (envFile) execFileSync(downScript, [envFile], { stdio: "inherit" });
}
