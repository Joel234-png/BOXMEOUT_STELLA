/**
 * Runs before each test file. `src/config.ts` validates the environment at
 * import time, so tests need every required variable present. CI and local
 * `.env` values take precedence; these defaults only fill the gaps.
 */
const testDefaults: Record<string, string> = {
  DATABASE_URL: "postgresql://boxmeout:boxmeout@localhost:5432/boxmeout_test",
  REDIS_URL: "redis://localhost:6379",
  STELLAR_RPC_URL: "https://soroban-testnet.stellar.org",
  STELLAR_HORIZON_URL: "https://horizon-testnet.stellar.org",
  MARKET_FACTORY_CONTRACT_ID: "CTESTFACTORY",
  TREASURY_CONTRACT_ID: "CTESTTREASURY",
  ADMIN_SECRET_KEY: "STESTADMINSECRET",
  ADMIN_API_KEY: "test-admin-key",
  ORACLE_API_KEY: "test-oracle-key",
};

for (const [key, value] of Object.entries(testDefaults)) {
  process.env[key] ??= value;
}
