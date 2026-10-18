/**
 * Runs once before the whole Jest run.
 * Resets the schema of the test database so every run starts from a clean,
 * fully migrated state. Refuses to run against any database whose name does
 * not look like a test database, so a misconfigured env can never wipe dev or
 * prod data.
 */

import { execFileSync } from "child_process";
import path from "path";

export default function globalSetup(): void {
  const testUrl = process.env.DATABASE_URL_TEST;

  if (!testUrl) {
    throw new Error(
      "DATABASE_URL_TEST is not set. Point it at a throwaway PostgreSQL database."
    );
  }

  const dbName = decodeURIComponent(new URL(testUrl).pathname.replace(/^\//, ""));

  if (!/test/i.test(dbName)) {
    throw new Error(
      `Refusing to reset database "${dbName}": its name must contain "test" ` +
        "because `prisma migrate reset --force` drops all data."
    );
  }

  const backendDir = path.resolve(__dirname, "..");

  execFileSync("npx", ["prisma", "migrate", "reset", "--force", "--skip-generate", "--skip-seed"], {
    cwd: backendDir,
    env: { ...process.env, DATABASE_URL: testUrl },
    stdio: "inherit",
  });
}
