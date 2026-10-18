/**
 * Runs once per test file, before any module is imported.
 * `src/config.ts` validates process.env at import time, so every required
 * variable must exist before the first test file loads. Values already set in
 * the environment (e.g. by CI) always win.
 */

process.env.NODE_ENV = "test";

// Tests always talk to the isolated test database, never the dev/prod one.
if (process.env.DATABASE_URL_TEST) {
  process.env.DATABASE_URL = process.env.DATABASE_URL_TEST;
}

process.env.DATABASE_URL ??=
  "postgresql://boxmeout:boxmeout@localhost:5432/boxmeout_test";
process.env.REDIS_URL ??= "redis://localhost:6379";
process.env.STELLAR_NETWORK ??= "testnet";
process.env.STELLAR_RPC_URL ??= "https://soroban-testnet.stellar.org";
process.env.STELLAR_HORIZON_URL ??= "https://horizon-testnet.stellar.org";
process.env.MARKET_FACTORY_CONTRACT_ID ??= "CTESTMARKETFACTORY";
process.env.TREASURY_CONTRACT_ID ??= "CTESTTREASURY";
process.env.ADMIN_SECRET_KEY ??= "STESTADMINSECRET";
process.env.ADMIN_API_KEY ??= "test-admin-key";
process.env.ORACLE_API_KEY ??= "test-oracle-key";
