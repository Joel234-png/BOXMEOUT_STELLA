/**
 * B-60: Readiness check controller
 *
 * GET /ready — verifies all external dependencies:
 *   - PostgreSQL (via Prisma $queryRaw)
 *   - Redis (via ioredis PING)
 *   - Soroban RPC (via SorobanRpc.Server.getHealth)
 *
 * Each check runs with an individual timeout (5 s by default) so a hung
 * dependency does not stall the entire probe indefinitely.
 *
 * Returns 200 with { status: "ok", checks: { ... } } when all pass.
 * Returns 503 with { status: "not_ready", checks: { ... } } if any fail,
 * including per-dependency status and (in non-production) the error message.
 */

import type { Request, Response } from "express";
import Redis from "ioredis";
import { SorobanRpc } from "@stellar/stellar-sdk";
import { db } from "../../db";

const CHECK_TIMEOUT_MS = 5_000;

type CheckStatus = "ok" | "error";

interface DependencyResult {
  status: CheckStatus;
  error?: string;
}

/** Races a promise against a timeout. Rejects with a timeout error if the
 *  promise does not settle within `ms` milliseconds. */
function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
  return Promise.race([
    promise,
    new Promise<T>((_, reject) =>
      setTimeout(() => reject(new Error(`${label} check timed out after ${ms}ms`)), ms)
    ),
  ]);
}

async function checkDb(): Promise<DependencyResult> {
  try {
    await withTimeout(db.$queryRaw`SELECT 1`, CHECK_TIMEOUT_MS, "db");
    return { status: "ok" };
  } catch (err) {
    return { status: "error", error: errorMessage(err) };
  }
}

async function checkRedis(): Promise<DependencyResult> {
  let client: Redis | null = null;
  try {
    // Create a short-lived client for the probe rather than reusing a shared
    // instance — avoids interfering with connection pool state on failure.
    client = new Redis(process.env.REDIS_URL ?? "redis://localhost:6379", {
      // Disable auto-reconnect for the probe client
      lazyConnect: true,
      maxRetriesPerRequest: 0,
    });
    await withTimeout(
      (async () => {
        await client!.connect();
        await client!.ping();
      })(),
      CHECK_TIMEOUT_MS,
      "redis"
    );
    return { status: "ok" };
  } catch (err) {
    return { status: "error", error: errorMessage(err) };
  } finally {
    if (client) {
      client.disconnect();
    }
  }
}

async function checkRpc(): Promise<DependencyResult> {
  try {
    const rpcUrl = process.env.STELLAR_RPC_URL ?? "";
    if (!rpcUrl) throw new Error("STELLAR_RPC_URL not configured");
    const server = new SorobanRpc.Server(rpcUrl);
    await withTimeout(server.getHealth(), CHECK_TIMEOUT_MS, "rpc");
    return { status: "ok" };
  } catch (err) {
    return { status: "error", error: errorMessage(err) };
  }
}

function errorMessage(err: unknown): string {
  if (process.env.NODE_ENV === "production") {
    // Do not leak internal error details in production
    return "check failed";
  }
  return err instanceof Error ? err.message : String(err);
}

/**
 * GET /ready
 */
export async function readinessHandler(req: Request, res: Response): Promise<void> {
  const [dbResult, redisResult, rpcResult] = await Promise.all([
    checkDb(),
    checkRedis(),
    checkRpc(),
  ]);

  const checks = {
    db: dbResult,
    redis: redisResult,
    rpc: rpcResult,
  };

  const allOk = Object.values(checks).every((c) => c.status === "ok");

  if (allOk) {
    res.status(200).json({ status: "ok", checks });
  } else {
    res.status(503).json({ status: "not_ready", checks });
  }
}
