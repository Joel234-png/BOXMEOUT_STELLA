// Runs before each test file. Points the app at the isolated test database
// and fills in the env vars that modules read at import time.
process.env.NODE_ENV = "test";
process.env.DATABASE_URL = process.env.DATABASE_URL_TEST || process.env.DATABASE_URL || "";
process.env.REDIS_URL = process.env.REDIS_URL || "redis://localhost:6379";
