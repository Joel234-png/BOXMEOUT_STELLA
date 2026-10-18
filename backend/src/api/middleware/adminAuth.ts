import { config } from "../../config";
import { apiKeyAuth } from "./apiKeyAuth";

/**
 * Admin authentication middleware.
 *
 * Scheme: static API key in the `X-Admin-Key` request header.
 * The expected key is ADMIN_API_KEY, read from the validated config.
 * Returns 401 { error, code } on any failure.
 */
export const adminAuth = apiKeyAuth({
  header: "x-admin-key",
  getKey: () => config.ADMIN_API_KEY,
});
