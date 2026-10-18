import { Router } from "express";
import rateLimit from "express-rate-limit";
import {
  getBetsByAddressHandler,
  getPortfolioHandler,
  getPayoutEstimateHandler,
  getLeaderboardHandler,
} from "../controllers/bet.controller";
import { validateAddressParam } from "../middleware/validate";
import { fail } from "../response";

const payoutEstimateLimiter = rateLimit({
  windowMs: 60 * 1000,
  max: 30,
  standardHeaders: true,
  legacyHeaders: false,
  handler: (_req, res) => {
    fail(res, 429, "RATE_LIMITED", "Too many requests", { details: { retryAfter: 60 } });
  },
});

const router = Router();

// GET /api/bets/leaderboard  — must come before /:address
router.get("/leaderboard", getLeaderboardHandler);

// GET /api/bets/payout-estimate  — must come before /:address
router.get("/payout-estimate", payoutEstimateLimiter, getPayoutEstimateHandler);

// GET /api/bets/:address/portfolio
router.get("/:address/portfolio", validateAddressParam, getPortfolioHandler);

// GET /api/bets/:address
router.get("/:address", validateAddressParam, getBetsByAddressHandler);

export default router;
