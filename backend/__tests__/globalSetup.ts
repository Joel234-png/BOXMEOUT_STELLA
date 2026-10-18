import { execSync } from "child_process";

/**
 * Runs once before the suite. When DATABASE_URL_TEST is set, resets that
 * isolated database so integration tests start from a clean schema.
 * Without it, nothing is run and tests that need a database must mock it.
 */
export default function globalSetup(): void {
  const url = process.env.DATABASE_URL_TEST;
  if (!url) return;

  execSync("npx prisma migrate reset --force --skip-generate --skip-seed", {
    stdio: "inherit",
    env: { ...process.env, DATABASE_URL: url },
  });
}
