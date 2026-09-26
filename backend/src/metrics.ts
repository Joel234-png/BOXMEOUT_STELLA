/**
 * B-61: Prometheus metrics
 *
 * Exposes:
 *   - Default Node.js process metrics (prom-client collectDefaultMetrics)
 *   - httpRequestDuration histogram  — HTTP request latency by method/path/status
 *   - indexerLedgerLag gauge         — latest RPC ledger minus last indexed ledger
 *   - lockJobSuccess / lockJobFailure counters
 *   - finalizeJobSuccess / finalizeJobFailure counters
 *
 * All metrics are registered on the default prom-client registry.
 * The /metrics route is exported as an Express Router for use in app.ts.
 */

import { Router } from "express";
import client, { Registry } from "prom-client";

// ─── Registry & default metrics ───────────────────────────────────────────────

// Use a dedicated registry so tests can create isolated instances
export const metricsRegistry: Registry = new client.Registry();

client.collectDefaultMetrics({ register: metricsRegistry });

// ─── HTTP request duration histogram ─────────────────────────────────────────

export const httpRequestDuration = new client.Histogram({
  name: "http_request_duration_seconds",
  help: "HTTP request latency in seconds",
  labelNames: ["method", "path", "status"],
  buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
  registers: [metricsRegistry],
});

// ─── Indexer lag gauge ────────────────────────────────────────────────────────

/**
 * Tracks the difference between the latest ledger seen from the Soroban RPC
 * and the last ledger the indexer successfully processed.
 * A rising value indicates the indexer is falling behind.
 */
export const indexerLedgerLag = new client.Gauge({
  name: "indexer_ledger_lag",
  help: "Number of ledgers between latest RPC ledger and last indexed ledger",
  registers: [metricsRegistry],
});

// ─── Job outcome counters ─────────────────────────────────────────────────────

export const lockJobSuccess = new client.Counter({
  name: "lock_job_success_total",
  help: "Total number of successful lock-market job executions",
  registers: [metricsRegistry],
});

export const lockJobFailure = new client.Counter({
  name: "lock_job_failure_total",
  help: "Total number of failed lock-market job executions",
  registers: [metricsRegistry],
});

export const finalizeJobSuccess = new client.Counter({
  name: "finalize_job_success_total",
  help: "Total number of successful finalize-market job executions",
  registers: [metricsRegistry],
});

export const finalizeJobFailure = new client.Counter({
  name: "finalize_job_failure_total",
  help: "Total number of failed finalize-market job executions",
  registers: [metricsRegistry],
});

// ─── /metrics router ──────────────────────────────────────────────────────────

export const metricsRouter = Router();

/**
 * GET /metrics — Prometheus scrape endpoint.
 * Returns all registered metrics in the Prometheus text exposition format.
 */
metricsRouter.get("/metrics", async (_req, res) => {
  try {
    const metrics = await metricsRegistry.metrics();
    res.set("Content-Type", metricsRegistry.contentType);
    res.end(metrics);
  } catch (err) {
    res.status(500).end(String(err));
  }
});
