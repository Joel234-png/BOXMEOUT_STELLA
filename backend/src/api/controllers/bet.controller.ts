import { Request, Response, NextFunction } from "express";
import { BetSide } from "@prisma/client";
import { z } from "zod";
import { logger } from "../../logger";
import * as betService from "../../services/bet.service";
import { ok, fail } from "../response";

// ─── Validation schemas ───────────────────────────────────────────────────────

// Stellar public keys: start with G, 56 chars, base32
const stellarAddressSchema = z
  .string()
  .regex(/^G[A-Z0-9]{55}$/, "Invalid Stellar address");

const betsByAddressQuerySchema = z.object({
  status: z.enum(["pending", "won", "lost", "claimed"]).optional(),
  marketId: z.string().optional(),
  page: z.coerce.number().int().positive().default(1),
  limit: z.coerce.number().int().positive().max(100).default(20),
});

const positionsQuerySchema = z.object({
  page: z.coerce.number().int().positive().default(1),
  limit: z.coerce.number().int().positive().max(100).default(20),
});

// ─── Handlers ─────────────────────────────────────────────────────────────────

/**
 * GET /api/bets/:address
 * Returns all bets for a Stellar address with optional filtering and pagination.
 */
export async function getBetsByAddressHandler(req: Request, res: Response): Promise<void> {
  const addressParsed = stellarAddressSchema.safeParse(req.params.address);
  if (!addressParsed.success) {
    fail(res, 400, "INVALID_ADDRESS", "Invalid Stellar address");
    return;
  }

  const parsed = betsByAddressQuerySchema.safeParse(req.query);
  if (!parsed.success) {
    fail(res, 400, "VALIDATION_ERROR", "Validation failed", { details: parsed.error.flatten() });
    return;
  }

  try {
    const { status, marketId, page, limit } = parsed.data;
    const bets = await betService.getBetsByAddress(addressParsed.data, { status, marketId });
    // Apply pagination after filtering (consistent with leaderboard pattern)
    const skip = (page - 1) * limit;
    const paginated = bets.slice(skip, skip + limit);
    ok(res, paginated, { meta: { page, limit, total: bets.length } });
  } catch (err) {
    logger.error({ err }, "getBetsByAddressHandler failed");
    fail(res, 500, "INTERNAL_ERROR", "Internal server error");
  }
}

/**
 * GET /api/bets/:address/portfolio  (issue #907)
 * Returns portfolio summary (total staked, winnings, ROI) for an address.
 * Returns zero-value summary (never 404) for unknown addresses.
 */
export async function getPortfolioHandler(
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> {
  try {
    const addressParsed = stellarAddressSchema.safeParse(req.params.address);
    if (!addressParsed.success) {
      fail(res, 400, "VALIDATION_ERROR", "Invalid Stellar address format");
      return;
    }

    const portfolio = await betService.getPortfolioSummary(addressParsed.data);
    ok(res, portfolio);
  } catch (err) {
    next(err);
  }
}

/**
 * GET /api/users/:address/bets   (#1088)
 * Alias route — same as /api/bets/:address but under /users path.
 */
export async function getUserBetsHandler(req: Request, res: Response): Promise<void> {
  return getBetsByAddressHandler(req, res);
}

/**
 * GET /api/users/:address/positions  (#1088)
 * Returns open/active bet positions for a user with pagination.
 */
export async function getUserPositionsHandler(req: Request, res: Response): Promise<void> {
  const addressParsed = stellarAddressSchema.safeParse(req.params.address);
  if (!addressParsed.success) {
    fail(res, 400, "INVALID_ADDRESS", "Invalid Stellar address");
    return;
  }

  const parsed = positionsQuerySchema.safeParse(req.query);
  if (!parsed.success) {
    fail(res, 400, "VALIDATION_ERROR", "Validation failed", { details: parsed.error.flatten() });
    return;
  }

  try {
    const { page, limit } = parsed.data;
    // Positions = active (unclaimed) bets
    const bets = await betService.getBetsByAddress(addressParsed.data, { status: "pending" });
    const skip = (page - 1) * limit;
    const paginated = bets.slice(skip, skip + limit);
    ok(res, paginated, { meta: { page, limit, total: bets.length } });
  } catch (err) {
    logger.error({ err }, "getUserPositionsHandler failed");
    fail(res, 500, "INTERNAL_ERROR", "Internal server error");
  }
}

/**
 * GET /api/bets/payout-estimate
 */
export async function getPayoutEstimateHandler(req: Request, res: Response): Promise<void> {
  const { market_id, side, amount } = req.query as Record<string, string>;

  if (!market_id || !side || !amount) {
    fail(res, 400, "VALIDATION_ERROR", "Validation failed", { details: { required: ["market_id", "side", "amount"] } });
    return;
  }

  if (!["FighterA", "FighterB"].includes(side)) {
    fail(res, 400, "VALIDATION_ERROR", "Validation failed", { details: { side: "must be FighterA or FighterB" } });
    return;
  }

  const parsedAmount = parseInt(amount, 10);
  if (isNaN(parsedAmount) || parsedAmount <= 0) {
    fail(res, 400, "VALIDATION_ERROR", "Validation failed", { details: { amount: "must be a positive integer" } });
    return;
  }

  try {
    const estimatedPayout = await betService.calculatePotentialPayout(
      market_id,
      side as BetSide,
      BigInt(parsedAmount)
    );
    ok(res, { estimatedPayout: estimatedPayout.toString() });
  } catch (err) {
    logger.error({ err }, "getPayoutEstimateHandler failed");
    fail(res, 500, "INTERNAL_ERROR", "Internal server error");
  }
}
