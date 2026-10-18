import { Request, Response, NextFunction } from "express";
import { MarketStatus } from "@prisma/client";
import { z } from "zod";
import { logger } from "../../logger";
import * as marketService from "../../services/market.service";
import * as oracleService from "../../services/oracle.service";
import { db } from "../../db";
import { createMarketSchema } from "../schemas/market.schema";
import {
  cursorMeta,
  cursorQuerySchema,
  pageMeta,
  pageQuerySchema,
} from "../schemas/pagination";
import * as betService from "../../services/bet.service";

// ─── Validation schemas ───────────────────────────────────────────────────────

const marketsQuerySchema = pageQuerySchema.extend({
  status: z.nativeEnum(MarketStatus).optional(),
  weightClass: z.string().optional(),
});

const marketBetsQuerySchema = cursorQuerySchema;

// ─── Handlers ─────────────────────────────────────────────────────────────────

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
    const { data, total } = await marketService.getAllMarkets(
      { status, weightClass },
      { page, pageSize: limit }
    );
    res.json({ data, page, limit, meta: pageMeta(page, limit, total) });
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
    const snapshots = await marketService.getMarketOddsHistory(
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
 * Returns leaderboard/bets for a market with cursor pagination.
 * Query params: cursor, limit (max 100). Response includes nextCursor.
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
    const { cursor, limit } = parsed.data;
    const { bets, nextCursor } = await betService.getBetsByMarketPaginated(req.params.id, {
      cursor: cursor || undefined,
      limit,
    });
    res.json({ data: bets, nextCursor, limit, meta: cursorMeta(nextCursor) });
  } catch (err) {
    logger.error({ err }, "getMarketBetsHandler failed");
    res.status(500).json({ error: "Internal server error" });
  }
}

/**
 * POST /api/admin/markets/resolve
 * Body: { oracle_result_id: number }
 * Admin-protected. Confirms an oracle result and triggers on-chain market resolution.
 */
export async function resolveMarketHandler(
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> {
  try {
    const { market_id, marketId, outcome, oracle_result_id, source, reporter } = req.body;
    const targetMarketId = market_id || marketId;

    if (oracle_result_id === undefined || oracle_result_id === null) {
      res
        .status(400)
        .json({ error: "oracle_result_id is required", code: "VALIDATION_ERROR" });
      return;
    }

    const id = Number(oracle_result_id);
    if (!Number.isInteger(id) || id <= 0) {
      res.status(400).json({
        error: "oracle_result_id must be a positive integer",
        code: "VALIDATION_ERROR",
      });
      return;
    }

    res.status(400).json({ error: "market_id (or marketId) and outcome required", code: "VALIDATION_ERROR" });
  } catch (err) {
    next(err);
  }
}

/**
 * POST /api/admin/markets/dispute/resolve
 */
export async function resolveDisputeHandler(
  req: Request,
  res: Response
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

    const result = await ResolutionService.handleDispute({
      disputeId: String(targetDisputeId),
      outcome,
      admin,
      notes: String(notesText),
    });

    res.status(200).json({ success: true, status: "ok", data: result });
  } catch (err) {
    next(err);
  }
}
