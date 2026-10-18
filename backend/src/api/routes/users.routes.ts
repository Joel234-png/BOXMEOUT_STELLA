import { Router } from "express";
import { getUserHandler, updateUserHandler } from "../controllers/user.controller";
import {
  getUserBetsHandler,
  getUserPositionsHandler,
} from "../controllers/bet.controller";
import { walletAuthMiddleware } from "../middleware/walletAuth.middleware";
import { requireOwnAddress } from "../middleware/requireOwnAddress";

const router = Router();

// GET /api/users/:address           — user profile (#1223 B-45)
router.get("/:address", getUserHandler);

// PUT /api/users/:address           — update profile, must own the address (#1223 B-45)
// whitelist: [] disables the admin whitelist, which applies to admin routes only.
router.put(
  "/:address",
  walletAuthMiddleware({ whitelist: [] }),
  requireOwnAddress,
  updateUserHandler,
);

// GET /api/users/:address/bets      — paginated bet history (#1088)
router.get("/:address/bets", getUserBetsHandler);

// GET /api/users/:address/positions — paginated open positions (#1088)
router.get("/:address/positions", getUserPositionsHandler);

export default router;
