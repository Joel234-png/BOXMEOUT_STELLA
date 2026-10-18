import { Router, type Request, type Response } from "express";
import { searchMarketsHandler } from "../controllers/marketSearch.controller";
import {
  getMarketsHandler,
  getMarketByIdHandler,
  getMarketStatsHandler,
  getMarketBetsHandler,
  getMarketOddsHistoryHandler,
  createMarketHandler,
} from "../controllers/market.controller";
import { createMarketSchema } from "../schemas/market.schema";
import { walletAuthMiddleware } from "../middleware/walletAuth.middleware";
import { enforceCreatorBinding } from "../middleware/creatorBinding.middleware";
import { rateLimitMiddleware } from "../middleware/rateLimit.middleware";
import { validate } from "../middleware/validate";

// B-99 (#1397): search is unauthenticated and runs a full-text query per call,
// so it gets the shared Redis sliding-window limiter.
const searchRateLimiter = rateLimitMiddleware({
  windowMs: 60 * 1000,
  max: 60,
  keyPrefix: "markets:search",
});

const router = Router();

const HEARTBEAT_INTERVAL_MS = 15_000;

// GET /api/markets/search?q=...  — must come before /:id (#1405 B-107)
router.get("/search", searchRateLimiter, searchMarketsHandler);

// GET  /api/markets
router.get("/", getMarketsHandler);

// POST /api/markets
// Creator must prove wallet ownership via challenge/response (#1225 B-47).
// Body is validated with zod, then createdBy must match the signing wallet (#1403).
router.post(
  "/",
  walletAuthMiddleware(),
  validate({ body: createMarketSchema }),
  enforceCreatorBinding,
  createMarketHandler,
);

// GET /api/markets/:id/stream — Server-Sent Events for live pool/status updates
router.get("/:id/stream", (req: Request, res: Response) => {
  const { id } = req.params;

  res.setHeader("Content-Type", "text/event-stream");
  res.setHeader("Cache-Control", "no-cache, no-transform");
  res.setHeader("Connection", "keep-alive");
  res.setHeader("X-Accel-Buffering", "no");
  res.flushHeaders?.();

  const send = (event: string, data: unknown) => {
    res.write(`event: ${event}\n`);
    res.write(`data: ${JSON.stringify(data)}\n\n`);
  };

  // Initial comment so clients/proxies open the stream immediately.
  res.write(": connected\n\n");

  const onPoolChange = (payload: unknown) => send("pool", payload);
  const onStatusChange = (payload: unknown) => send("status", payload);

  marketEvents.on("pool", onPoolChange);
  marketEvents.on("status", onStatusChange);

  const heartbeat = setInterval(() => {
    res.write(": heartbeat\n\n");
  }, HEARTBEAT_INTERVAL_MS);

  const cleanup = () => {
    clearInterval(heartbeat);
    marketEvents.off("pool", onPoolChange);
    marketEvents.off("status", onStatusChange);
  };

  req.on("close", cleanup);
  res.on("close", cleanup);
});

// GET /api/markets/:id
router.get("/:id", getMarketByIdHandler);

// GET /api/markets/:id/stats
router.get("/:id/stats", getMarketStatsHandler);

// GET /api/markets/:id/bets
router.get("/:id/bets", getMarketBetsHandler);

// GET /api/markets/:id/odds-history?interval=5m|1h
router.get("/:id/odds-history", getMarketOddsHistoryHandler);

export default router;
