import type { Request, Response } from "express";

/**
 * Catch-all for requests that no route handled, including unknown methods on
 * known paths. Must be registered after every route and before
 * errorHandlerMiddleware, otherwise Express's default HTML 404 is returned.
 *
 * Returns 404 { error, code: "NOT_FOUND" }.
 */
export function notFoundMiddleware(_req: Request, res: Response): void {
  res.status(404).json({ error: "Not found", code: "NOT_FOUND" });
}
