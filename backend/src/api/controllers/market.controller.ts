import { Request, Response, NextFunction } from "express";
import { MarketStatus } from "@prisma/client";
import { z } from "zod";
import { logger } from "../../logger";
import * as marketService from "../../services/market.service";
import { searchMarkets } from "../../repositories/market.repository";
import * as oracleService from "../../services/oracle.service";
import { db } from "../../db";
import { ok, fail } from "../response";

// ─── Validation schemas ───────────────────────────────────────────────────────

const marketsQuerySchema = z.object({
  status: z.nativeEnum(MarketStatus).optional(),
  weightClass: z.string().optional(),
  page: z.coerce.number().int().positive().default(1),
  limit: z.coerce.number().int().positive().max(100).default(20),
});

const marketBetsQuerySchema = z.object({
  cursor: z.string().optional(),
  limit: z.coerce.number().int().positive().max(100).default(20),
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
    fail(res, 400, "VALIDATION_ERROR", "q is required");
    return;
  }
  const page = Math.max(1, parseInt(String(req.query.page ?? "1"), 10) || 1);
  const limit = Math.min(100, Math.max(1, parseInt(String(req.query.limit ?? "20"), 10) || 20));
  const result = await searchMarkets(q, page, limit);
  ok(res, result.data, { meta: { total: result.total } });
}

/**
 * GET /api/markets
 * Optional query params: status, weightClass, page, limit
 */
export async function getMarketsHandler(req: Request, res: Response): Promise<void> {
  const parsed = marketsQuerySchema.safeParse(req.query);
  if (!parsed.success) {
    fail(res, 400, "VALIDATION_ERROR", "Validation failed", { details: parsed.error.flatten() });
    return;
  }

  try {
    const { status, weightClass, page, limit } = parsed.data;
    const markets = await marketService.getAllMarkets(
      { status, weightClass },
      { page, limit }
    );
    ok(res, markets, { meta: { page, limit } });
  } catch (err) {
    logger.error({ err }, "getMarketsHandler failed");
    fail(res, 500, "INTERNAL_ERROR", "Internal server error");
  }
}

/**
 * GET /api/markets/:id
 */
export async function getMarketByIdHandler(req: Request, res: Response): Promise<void> {
  try {
    const market = await marketService.getMarketById(req.params.id);
    if (!market) {
      fail(res, 404, "NOT_FOUND", "Market not found");
      return;
    }
    ok(res, market);
  } catch (err) {
    logger.error({ err }, "getMarketByIdHandler failed");
    fail(res, 500, "INTERNAL_ERROR", "Internal server error");
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
    fail(res, 400, "VALIDATION_ERROR", "Validation failed", { details: parsed.error.flatten() });
    return;
  }

  try {
    const snapshots = await marketService.getMarketOddsHistory(
      req.params.id,
      parsed.data.interval
    );
    ok(res, snapshots);
  } catch (err: any) {
    if (err?.code === "NOT_FOUND") {
      fail(res, 404, "NOT_FOUND", "Market not found");
      return;
    }
    logger.error({ err }, "getMarketOddsHistoryHandler failed");
    fail(res, 500, "INTERNAL_ERROR", "Internal server error");
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
    ok(res, market, { status: 201 });
  } catch (err) {
    logger.error({ err }, "createMarketHandler failed");
    fail(res, 500, "INTERNAL_ERROR", "Internal server error");
  }
}

/**
 * GET /api/markets/:id/stats
 */
export async function getMarketStatsHandler(req: Request, res: Response): Promise<void> {
  try {
    const stats = await marketService.getMarketStats(req.params.id);
    ok(res, stats);
  } catch (err: any) {
    if (err?.code === "NOT_FOUND") {
      fail(res, 404, "NOT_FOUND", "Market not found");
      return;
    }
    logger.error({ err }, "getMarketStatsHandler failed");
    fail(res, 500, "INTERNAL_ERROR", "Internal server error");
  }
}

/**
 * GET /api/markets/:id/bets
 * Returns leaderboard/bets for a market with cursor pagination.
 * Query params: cursor, limit (max 100). Response includes nextCursor.
 */
export async function getMarketBetsHandler(req: Request, res: Response): Promise<void> {
  const parsed = marketBetsQuerySchema.safeParse(req.query);
  if (!parsed.success) {
    fail(res, 400, "VALIDATION_ERROR", "Validation failed", { details: parsed.error.flatten() });
    return;
  }

  try {
    const { cursor, limit } = parsed.data;
    const { data, nextCursor } = await marketService.getMarketLeaderboard(req.params.id, {
      cursor,
      limit,
    });
    ok(res, data, { meta: { nextCursor, limit } });
  } catch (err) {
    logger.error({ err }, "getMarketBetsHandler failed");
    fail(res, 500, "INTERNAL_ERROR", "Internal server error");
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
      fail(res, 400, "VALIDATION_ERROR", "dispute_id is required");
      return;
    }

    if (!outcome) {
      fail(res, 400, "VALIDATION_ERROR", "outcome is required");
      return;
    }

    if (!notesText || !String(notesText).trim()) {
      fail(res, 400, "VALIDATION_ERROR", "Notes explaining the decision are required for audit");
      return;
    }

    const admin = (req as any).user?.id || (req as any).admin || "admin";

    await oracleService.resolveDispute(String(targetDisputeId), outcome, admin);

    ok(res, { success: true });
  } catch (err) {
    next(err);
  }
}

