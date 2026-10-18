/**
 * Fills in placeholder values for the env vars config.ts requires, so unit and
 * route-level tests can import the app without a real .env file.
 * Values already present in the environment (for example from CI) are kept.
 */
const TEST_DEFAULTS: Record<string, string> = {
  DATABASE_URL: "postgresql://test:test@localhost:5432/boxmeout_test",
  STELLAR_RPC_URL: "https://soroban-testnet.stellar.org",
  STELLAR_HORIZON_URL: "https://horizon-testnet.stellar.org",
  MARKET_FACTORY_CONTRACT_ID: "CTESTMARKETFACTORY",
  TREASURY_CONTRACT_ID: "CTESTTREASURY",
  ADMIN_SECRET_KEY: "STESTSECRETKEY",
  ADMIN_API_KEY: "test-admin-key",
  ORACLE_API_KEY: "test-oracle-key",
  REDIS_URL: "redis://localhost:6379",
};

for (const [key, value] of Object.entries(TEST_DEFAULTS)) {
  if (process.env[key] === undefined) {
    process.env[key] = value;
  }
}
