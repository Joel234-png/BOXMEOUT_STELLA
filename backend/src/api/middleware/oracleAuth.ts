import { config } from "../../config";
import { apiKeyAuth } from "./apiKeyAuth";

/**
 * Oracle authentication middleware.
 *
 * Scheme: static API key in the `X-Oracle-Key` request header.
 * The expected key is ORACLE_API_KEY, read from the validated config.
 * Returns 401 { error, code } on any failure.
 */
export const oracleAuth = apiKeyAuth({
  header: "x-oracle-key",
  getKey: () => config.ORACLE_API_KEY,
});
