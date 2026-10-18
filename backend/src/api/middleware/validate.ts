import { Request, Response, NextFunction } from "express";
import { z, ZodTypeAny } from "zod";
import { StrKey } from "@stellar/stellar-sdk";
import { fail } from "../response";

/**
 * Shared zod schema for Stellar Ed25519 public keys (G... addresses).
 */
export const stellarAddress = z
  .string()
  .refine((value) => StrKey.isValidEd25519PublicKey(value), {
    message: "INVALID_ADDRESS",
  });

/**
 * Validates a single route param against the given zod schema.
 * Responds with 400 and the schema's error code on failure.
 */
export function validateParam(
  paramName: string,
  schema: ZodTypeAny,
) {
  return (req: Request, res: Response, next: NextFunction): void => {
    const result = schema.safeParse(req.params[paramName]);
    if (!result.success) {
      const code = result.error.issues[0]?.message ?? "INVALID_ADDRESS";
      fail(res, 400, code, "Invalid request parameter");
      return;
    }
    next();
  };
}
