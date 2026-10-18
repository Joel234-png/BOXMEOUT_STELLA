import { timingSafeEqual } from "crypto";
import type { NextFunction, Request, RequestHandler, Response } from "express";
import { fail } from "../response";

export interface ApiKeyAuthOptions {
  /** Request header that carries the key, e.g. `x-admin-key`. Case-insensitive. */
  header: string;
  /** Returns the expected key. An empty or missing value rejects every request. */
  getKey: () => string | undefined;
}

/**
 * Builds a static API-key check for one header.
 *
 * Returns 401 { error, code: "UNAUTHORIZED" } when the header is missing, the
 * expected key is unset, or the provided key does not match. The comparison
 * uses crypto.timingSafeEqual so response timing does not leak key content.
 */
export function apiKeyAuth({ header, getKey }: ApiKeyAuthOptions): RequestHandler {
  const headerName = header.toLowerCase();

  return (req: Request, res: Response, next: NextFunction): void => {
    const provided = req.headers[headerName];
    const expected = getKey() ?? "";

    if (typeof provided !== "string" || !provided || !expected || !keysMatch(provided, expected)) {
      fail(res, 401, "UNAUTHORIZED", "Unauthorized");
      return;
    }

    next();
  };
}

function keysMatch(provided: string, expected: string): boolean {
  const a = Buffer.from(provided);
  const b = Buffer.from(expected);

  // timingSafeEqual throws on unequal lengths, so compare against a copy of
  // `a` to keep the work constant before rejecting a length mismatch.
  if (a.length !== b.length) {
    timingSafeEqual(a, a);
    return false;
  }

  return timingSafeEqual(a, b);
}
