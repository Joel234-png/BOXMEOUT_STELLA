/**
 * Runs before each test file. config.ts validates env vars at import time,
 * so give it harmless test values unless the test sets its own.
 */
const testEnv: Record<string, string> = {
  NODE_ENV: "test",
  DATABASE_URL: "postgresql://boxmeout:boxmeout@localhost:5432/boxmeout_test",
  STELLAR_RPC_URL: "https://soroban-testnet.stellar.org",
  STELLAR_HORIZON_URL: "https://horizon-testnet.stellar.org",
  MARKET_FACTORY_CONTRACT_ID: "CTEST",
  TREASURY_CONTRACT_ID: "CTEST",
  ADMIN_SECRET_KEY: "STESTKEY",
  ADMIN_API_KEY: "test-admin-key",
  ORACLE_API_KEY: "test-oracle-key",
  REDIS_URL: "redis://localhost:6379",
};

for (const [key, value] of Object.entries(testEnv)) {
  process.env[key] ??= value;
}
