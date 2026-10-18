import { PrismaClient } from "@prisma/client";
import { logger } from "./logger";

// Singleton guard: in development, ts-node-dev hot-reloads modules which would
// create a new PrismaClient on every reload, exhausting the connection pool.
// Storing the instance on globalThis ensures only one client exists per process.

declare global {
  // eslint-disable-next-line no-var
  var __prisma: PrismaClient | undefined;
}

/**
 * B-100: Prisma logs are routed through pino in event mode instead of Prisma's
 * own stdout logger, which prints query parameters (wallet addresses, request
 * fields) verbatim. Only the SQL template, duration and error target are logged.
 */
function createPrismaClient(): PrismaClient {
  const client = new PrismaClient({
    log: [
      { emit: "event", level: "query" },
      { emit: "event", level: "warn" },
      { emit: "event", level: "error" },
    ],
  });

  client.$on("query", (e) => {
    logger.debug({ query: e.query, durationMs: e.duration }, "prisma query");
  });
  client.$on("warn", (e) => {
    logger.warn({ target: e.target }, "prisma warning");
  });
  client.$on("error", (e) => {
    logger.error({ target: e.target }, "prisma error");
  });

  return client;
}

export const db: PrismaClient = globalThis.__prisma ?? createPrismaClient();

if (process.env.NODE_ENV !== "production") {
  globalThis.__prisma = db;
}
