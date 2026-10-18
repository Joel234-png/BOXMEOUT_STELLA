import { execSync } from "child_process";

// Resets the test database once before the suite. The URL must point at a
// throwaway database (see DATABASE_URL_TEST in .env.example).
export default function globalSetup(): void {
  const url = process.env.DATABASE_URL_TEST || process.env.DATABASE_URL;
  if (!url) {
    throw new Error("Set DATABASE_URL_TEST (or DATABASE_URL) before running tests");
  }
  execSync("npx prisma migrate reset --force --skip-generate --skip-seed", {
    stdio: "inherit",
    env: { ...process.env, DATABASE_URL: url },
  });
}
