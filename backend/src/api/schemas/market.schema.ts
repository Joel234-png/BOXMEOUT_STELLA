import { z } from "zod";
import { stellarAddress, stellarContractAddress } from "../middleware/validate";

/**
 * Body schema for POST /api/markets.
 * - contractAddress is the market's Soroban contract (C...).
 * - createdBy and oracleAddress are Stellar account addresses (G...).
 */
export const createMarketSchema = z.object({
  id: z.string().min(1),
  contractAddress: stellarContractAddress,
  fighterA: z.record(z.unknown()),
  fighterB: z.record(z.unknown()),
  scheduledAt: z.string().datetime({ message: "scheduledAt must be a valid ISO 8601 datetime" }),
  bettingEndsAt: z.string().datetime({ message: "bettingEndsAt must be a valid ISO 8601 datetime" }),
  createdBy: stellarAddress,
  oracleAddress: stellarAddress,
  txHash: z.string().optional(),
});
