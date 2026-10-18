import { Request, Response, NextFunction } from "express";
import { MarketStatus } from "@prisma/client";
import { z } from "zod";
import { logger } from "../../logger";
import * as marketService from "../../services/market.service";
import { searchMarkets } from "../../repositories/market.repository";
import * as oracleService from "../../services/oracle.service";
import { db } from "../../db";

// ─── Validation schemas ───────────────────────────────────────────────────────

const marketsQuerySchema = z.object({
  status: z.nativeEnum(MarketStatus).optional(),
  weightClass: z.string().optional(),
  page: z.coerce.number().int().positive().default(1),
  limit: z.coerce.number().int().positive().max(100).default(20),
});

const marketBetsQuerySchema = z.object({
  page: z.coerce.number().int().positive().default(1),
  limit: z.coerce.number().int().positive().max(100).default(20),
});

const oddsHistoryQuerySchema = z.object({
  interval: z.enum(["5m", "1h"]).optional(),
});

const resolveMarketBodySchema = z.object({
  oracle_result_id: z.coerce.number().int().positive(),
});

export const createMarketSchema = z.object({
  id: z.string().min(1),
  contractAddress: z.string().min(1),
  fighterA: z.record(z.unknown()),
  fighterB: z.record(z.unknown()),
  scheduledAt: z.string().datetime({ message: "scheduledAt must be a valid ISO 8601 datetime" }),
  bettingEndsAt: z.string().datetime({ message: "bettingEndsAt must be a valid ISO 8601 datetime" }),
  createdBy: z.string().min(1),
  oracleAddress: z.string().min(1),
  txHash: z.string().optional(),
});

// ─── Handlers ─────────────────────────────────────────────────────────────────

/**
 * GET /api/markets/search?q=&page=&limit=
 * Full-text search across question + description. Returns paginated { data, total }.
 */
export async function searchMarketsHandler(
  req: Request,
  res: Response
): Promise<void> {
  const q = String(req.query.q ?? "").trim();
  if (!q) {
    res.status(400).json({ error: "q is required", code: "VALIDATION_ERROR" });
    return;
  }
  const page = Math.max(1, parseInt(String(req.query.page ?? "1"), 10) || 1);
  const limit = Math.min(100, Math.max(1, parseInt(String(req.query.limit ?? "20"), 10) || 20));
  const result = await searchMarkets(q, page, limit);
  res.json(result);
}

/**
 * GET /api/markets
 * Optional query params: status, weightClass, page, limit
 */
export async function getMarketsHandler(req: Request, res: Response): Promise<void> {
  const parsed = marketsQuerySchema.safeParse(req.query);
  if (!parsed.success) {
    res.status(400).json({
      error: "Validation failed",
      code: "VALIDATION_ERROR",
      details: parsed.error.flatten(),
    });
    return;
  }

  try {
    const { status, weightClass, page, limit } = parsed.data;
    const markets = await marketService.getAllMarkets(
      { status, weightClass },
      { page, pageSize: limit }
    );
    res.json({ data: markets, page, limit });
  } catch (err) {
    logger.error({ err }, "getMarketsHandler failed");
    res.status(500).json({ error: "Internal server error" });
  }
}

/**
 * GET /api/markets/:id
 */
export async function getMarketByIdHandler(req: Request, res: Response): Promise<void> {
  try {
    const market = await marketService.getMarketById(req.params.id);
    if (!market) {
      res.status(404).json({ error: "Market not found" });
      return;
    }
    res.json({ data: market });
  } catch (err) {
    logger.error({ err }, "getMarketByIdHandler failed");
    res.status(500).json({ error: "Internal server error" });
  }
}

/**
 * GET /api/markets/:id/odds-history
 * Optional query param: interval=5m|1h
 * Returns an array of OddsSnapshot { timestamp, poolA, poolB, oddsA, oddsB }
 * derived from bets. When interval is omitted, raw per-bet snapshots are returned.
 */
export async function getMarketOddsHistoryHandler(
  req: Request,
  res: Response
): Promise<void> {
  const parsed = oddsHistoryQuerySchema.safeParse(req.query);
  if (!parsed.success) {
    res.status(400).json({
      error: "Validation failed",
      code: "VALIDATION_ERROR",
      details: parsed.error.flatten(),
    });
    return;
  }

  try {
    const snapshots = await marketService.getOddsHistory(
      req.params.id,
      parsed.data.interval
    );
    res.json({ data: snapshots });
  } catch (err: any) {
    if (err?.code === "NOT_FOUND") {
      res.status(404).json({ error: "Market not found" });
      return;
    }
    logger.error({ err }, "getMarketOddsHistoryHandler failed");
    res.status(500).json({ error: "Internal server error" });
  }
}

/**
 * POST /api/markets
 * Creates a new market record.
 * Auth: walletAuthMiddleware (challenge/response, caller proves wallet ownership).
 * Body: validated by validate(createMarketSchema) middleware before this handler runs.
 */
export async function createMarketHandler(req: Request, res: Response): Promise<void> {
  try {
    const data = req.body as z.infer<typeof createMarketSchema>;
    const market = await marketService.createMarketRecord({
      id: data.id,
      contractAddress: data.contractAddress,
      fighterA: data.fighterA,
      fighterB: data.fighterB,
      scheduledAt: new Date(data.scheduledAt),
      bettingEndsAt: new Date(data.bettingEndsAt),
      createdAt: new Date(),
      createdBy: data.createdBy,
      oracleAddress: data.oracleAddress,
      txHash: data.txHash,
    });
    res.status(201).json({ data: market });
  } catch (err) {
    logger.error({ err }, "createMarketHandler failed");
    res.status(500).json({ error: "Internal server error" });
  }
}

/**
 * GET /api/markets/:id/stats
 */
export async function getMarketStatsHandler(req: Request, res: Response): Promise<void> {
  try {
    const stats = await marketService.getMarketStats(req.params.id);
    res.json({ data: stats });
  } catch (err: any) {
    if (err?.code === "NOT_FOUND") {
      res.status(404).json({ error: "Market not found" });
      return;
    }
    logger.error({ err }, "getMarketStatsHandler failed");
    res.status(500).json({ error: "Internal server error" });
  }
}

/**
 * GET /api/markets/:id/bets
 * Returns leaderboard/bets for a market with page pagination.
 * Query params: page, limit (max 100).
 */
export async function getMarketBetsHandler(req: Request, res: Response): Promise<void> {
  const parsed = marketBetsQuerySchema.safeParse(req.query);
  if (!parsed.success) {
    res.status(400).json({
      error: "Validation failed",
      code: "VALIDATION_ERROR",
      details: parsed.error.flatten(),
    });
    return;
  }

  try {
    const { page, limit } = parsed.data;
    const data = await marketService.getMarketLeaderboard(req.params.id, { page, pageSize: limit });
    res.json({ data, page, limit });
  } catch (err) {
    logger.error({ err }, "getMarketBetsHandler failed");
    res.status(500).json({ error: "Internal server error" });
  }
}

/**
 * POST /api/admin/markets/resolve
 * Body: { oracle_result_id: number }
 * Admin-protected. Confirms an oracle result and triggers on-chain market resolution.
 * Admin mutations are recorded by auditLogMiddleware (#1258).
 */
export async function resolveMarketHandler(
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> {
  const parsed = resolveMarketBodySchema.safeParse(req.body ?? {});
  if (!parsed.success) {
    res.status(400).json({
      error: "Validation failed",
      code: "VALIDATION_ERROR",
      details: parsed.error.flatten(),
    });
    return;
  }

  try {
    const admin = (req as any).user?.id || "admin";
    await oracleService.confirmFightResult(String(parsed.data.oracle_result_id), admin);
    res.status(200).json({ status: "ok" });
  } catch (err) {
    next(err);
  }
}

/**
 * POST /api/admin/markets/dispute/resolve
 */
export async function resolveDisputeHandler(
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> {
  try {
    const { dispute_id, disputeId, outcome, notes, decisionNotes } = req.body;
    const targetDisputeId = dispute_id || disputeId;
    const notesText = notes || decisionNotes;

    if (!targetDisputeId) {
      res.status(400).json({ error: "dispute_id is required", code: "VALIDATION_ERROR" });
      return;
    }

    if (!outcome) {
      res.status(400).json({ error: "outcome is required", code: "VALIDATION_ERROR" });
      return;
    }

    if (!notesText || !String(notesText).trim()) {
      res.status(400).json({ error: "Notes explaining the decision are required for audit", code: "VALIDATION_ERROR" });
      return;
    }

    const admin = (req as any).user?.id || (req as any).admin || "admin";

    await oracleService.resolveDispute(String(targetDisputeId), outcome, admin);
    const result = { disputeId: String(targetDisputeId), outcome, notes: String(notesText) };

    res.status(200).json({ success: true, status: "ok", data: result });
  } catch (err) {
    next(err);
  }
}

/**
 * GET /api/admin/markets/pending
 */
export async function getPendingResolutionsHandler(
  req: Request,
  res: Response
): Promise<void> {
  try {
    const markets = await marketService.getAllMarkets({ status: "Locked" });
    res.json({ data: markets });
  } catch (err) {
    logger.error({ err }, "getPendingResolutionsHandler failed");
    res.status(500).json({ error: "Internal server error" });
  }
}

/**
 * GET /health
 */
export async function healthCheckHandler(
  req: Request,
  res: Response
): Promise<void> {
  try {
    await db.$queryRaw`SELECT 1`;
    res.status(200).json({ status: "ok", db: "connected" });
  } catch {
    res.status(503).json({ status: "degraded", db: "disconnected" });
  }
}
