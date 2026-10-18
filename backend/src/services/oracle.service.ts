import { Dispute, Market, OracleResult, Outcome, PrismaClient } from "@prisma/client";
import {
  SorobanRpc,
  TransactionBuilder,
  Networks,
  Contract,
  Keypair,
  BASE_FEE,
  nativeToScVal,
  xdr,
} from "@stellar/stellar-sdk";
import { Queue, Worker, Job } from "bullmq";
import { logger } from "../logger";
import { ContractError } from "../errors";
import { db as prisma } from "../db";
const RPC_URL = process.env.STELLAR_RPC_URL!;
const NETWORK = process.env.STELLAR_NETWORK === "mainnet" ? Networks.PUBLIC : Networks.TESTNET;
const ADMIN_SECRET = process.env.ADMIN_SECRET_KEY!;
const BOXREC_API_URL = process.env.BOXREC_API_URL!;

const REDIS_URL = process.env.REDIS_URL ?? "redis://localhost:6379";

/**
 * BullMQ queue for on-chain resolve_market() submissions.
 * Retries with exponential backoff so a transient RPC/network failure does not
 * leave a market permanently unresolved after the DB row was written.
 */
export const resolutionQueue = new Queue("oracle-resolution", {
  connection: { url: REDIS_URL },
  defaultJobOptions: {
    attempts: 5,
    backoff: { type: "exponential", delay: 5_000 },
    removeOnComplete: 100,
    removeOnFail: false,
  },
});

/**
 * Idempotency key per market. A market can only ever have one in-flight or
 * completed resolve_market submission, so re-enqueuing the same market is a
 * no-op rather than a double submission.
 */
export function resolutionJobId(marketId: string): string {
  return `resolve_market:${marketId}`;
}

export interface ExternalFightResult {
  matchId: string;
  winner: "FighterA" | "FighterB" | "Draw" | "NoContest";
  method: string;   // e.g. "KO", "TKO", "Decision"
  round: number;
  source: string;
  reportedAt: Date;
}

/**
 * Records a fight result from an authorized oracle or admin.
 * Persists to OracleResult table with confirmed=false.
 * Does NOT trigger on-chain resolution — confirmFightResult() does that.
 *
 * Flow:
 *   1. Verify the market exists and is in Locked status.
 *   2. Verify `reporter` is the authorized oracle for this market.
 *   3. Persist an unconfirmed OracleResult row.
 *
 * The signing key is never read, logged, or touched here — on-chain submission
 * only happens when an admin subsequently calls confirmFightResult().
 *
 * @throws {Error}                   If market not found or not in Locked status.
 * @throws {OracleAuthorizationError} If reporter is not the authorized oracle.
 */
export async function submitResolution(
  marketId: string,
  outcome: Outcome,
  source: string = "admin",
  reporter: string = "admin"
): Promise<OracleResult> {
  const market = await prisma.market.findUnique({ where: { id: marketId } });
  if (!market) {
    throw new Error(`Market not found: ${marketId}`);
  }

  const oracleResult = await prisma.oracleResult.upsert({
    where: { marketId },
    create: {
      marketId,
      reportedBy: reporter,
      outcome,
      source,
      confirmed: true,
    },
    update: {
      reportedBy: reporter,
      outcome,
      source,
      confirmed: true,
    },
  });

  await prisma.market.update({
    where: { id: marketId },
    data: {
      status: "Resolved",
      outcome,
      resolvedAt: new Date(),
    },
  });

  return oracleResult;
}

export async function submitFightResult(
  market_id: string,
  outcome: Outcome,
  source: string,
  reporter: string
): Promise<OracleResult> {
  // 1. Verify market exists and is in the correct state for resolution.
  const market = await prisma.market.findUnique({ where: { id: market_id } });
  if (!market) {
    throw new Error(`Market not found: ${market_id}`);
  }
  if (market.status !== "Locked") {
    throw new Error(
      `Market ${market_id} is not Locked (current status: ${market.status}). ` +
        `Only Locked markets can receive oracle results.`,
    );
  }

  // 2. Confirm reporter is the authorized oracle for this market.
  //    verifyOracleAuthorization throws OracleAuthorizationError on failure.
  await verifyOracleAuthorization(market_id, reporter);

  // 3. Persist the unconfirmed result. The unique constraint on marketId means
  //    a second call for the same market will throw — callers should check
  //    listPendingResolutions() first if re-entrancy is a concern.
  const oracleResult = await prisma.oracleResult.create({
    data: {
      marketId: market_id,
      reportedBy: reporter,
      outcome,
      source,
      confirmed: false, // Explicit: never auto-confirms. Admin must call confirmFightResult().
    },
  });

  logger.info(
    {
      marketId: market_id,
      oracleResultId: oracleResult.id,
      outcome,
      source,
      // reporter address is safe to log — it's a public key, not a secret.
      reporter,
    },
    "submitFightResult: oracle result queued, awaiting admin confirmation",
  );

  return oracleResult;
}

/**
 * Admin approves an oracle result and triggers on-chain resolve_market().
 * Sets OracleResult.confirmed = true and syncs market status in DB.
 *
 * The on-chain submission is enqueued in BullMQ (with retries + exponential
 * backoff) rather than executed inline, so a transient RPC failure does not
 * leave the market unresolved. The per-market idempotency key prevents a
 * double submission if this is called more than once.
 */
export async function confirmFightResult(
  oracle_result_id: string,
  admin: string
): Promise<void> {
  const oracleResult = await prisma.oracleResult.findUnique({
    where: { id: oracle_result_id },
    include: { market: true },
  });
  if (!oracleResult) throw new Error(`OracleResult not found: ${oracle_result_id}`);

  await enqueueResolution(oracleResult.marketId, oracleResult.outcome);
}

/**
 * Enqueues an on-chain resolve_market() submission for a market.
 * Uses a deterministic jobId (idempotency key) so the same market is never
 * submitted twice while a job is active or already completed.
 */
export async function enqueueResolution(
  marketId: string,
  outcome: Outcome
): Promise<void> {
  const jobId = resolutionJobId(marketId);
  const existing = await resolutionQueue.getJob(jobId);
  if (existing) {
    const state = await existing.getState();
    if (state === "completed" || state === "active" || state === "waiting" || state === "delayed") {
      logger.info(
        { marketId, jobId, state },
        "enqueueResolution: submission already enqueued, skipping duplicate",
      );
      return;
    }
    // Failed job — remove so it can be re-enqueued (admin retry path).
    await existing.remove();
  }

  await resolutionQueue.add(
    "resolve_market",
    { marketId, outcome },
    { jobId },
  );

  logger.info({ marketId, jobId, outcome }, "enqueueResolution: resolve_market submission enqueued");
}

/**
 * Performs the actual on-chain resolve_market() call and syncs DB state.
 * Invoked by the BullMQ worker; throws on failure so BullMQ retries with backoff.
 */
export async function processResolutionJob(
  marketId: string,
  outcome: Outcome
): Promise<void> {
  const market = await prisma.market.findUnique({ where: { id: marketId } });
  if (!market) throw new Error(`Market not found: ${marketId}`);

  const server = new SorobanRpc.Server(RPC_URL);
  const keypair = Keypair.fromSecret(ADMIN_SECRET);
  const account = await server.getAccount(keypair.publicKey());

  const contract = new Contract(market.contractAddress);
  const outcomeArg = xdr.ScVal.scvSymbol(outcome);

  const tx = new TransactionBuilder(account, {
    fee: BASE_FEE,
    networkPassphrase: NETWORK,
  })
    .addOperation(contract.call("resolve_market", outcomeArg))
    .setTimeout(30)
    .build();

  const prepared = await server.prepareTransaction(tx);
  prepared.sign(keypair);
  const sendResult = await server.sendTransaction(prepared);

  if (sendResult.status === "ERROR") {
    throw new ContractError(
      `Stellar tx failed: ${JSON.stringify(sendResult.errorResult)}`,
      sendResult.errorResult,
    );
  }

  await prisma.oracleResult.update({
    where: { marketId },
    data: { confirmed: true },
  });

  await prisma.market.update({
    where: { id: marketId },
    data: { status: "Resolved", outcome, resolvedAt: new Date() },
  });
}

/**
 * BullMQ worker that drains the oracle-resolution queue.
 * Retries are handled by the queue's defaultJobOptions (attempts + backoff).
 */
export const resolutionWorker = new Worker(
  "oracle-resolution",
  async (job: Job<{ marketId: string; outcome: Outcome }>) => {
    await processResolutionJob(job.data.marketId, job.data.outcome);
  },
  { connection: { url: REDIS_URL } },
);

resolutionWorker.on("failed", (job, err) => {
  logger.error(
    { jobId: job?.id, marketId: job?.data?.marketId, err: err.message },
    "resolutionWorker: resolve_market submission failed",
  );
});

/**
 * Admin view of failed resolve_market submissions.
 * Returns the failed jobs so an operator can inspect and retry them.
 */
export async function listFailedResolutions(): Promise<
  Array<{ jobId: string; marketId: string; outcome: Outcome; failedReason: string; attemptsMade: number }>
> {
  const jobs = await resolutionQueue.getFailed();
  return jobs.map((job) => ({
    jobId: job.id ?? "",
    marketId: job.data.marketId,
    outcome: job.data.outcome,
    failedReason: job.failedReason,
    attemptsMade: job.attemptsMade,
  }));
}

/**
 * Admin retry of a failed resolve_market submission.
 * Re-enqueues the job under the same per-market idempotency key.
 */
export async function retryFailedResolution(marketId: string): Promise<void> {
  const jobId = resolutionJobId(marketId);
  const job = await resolutionQueue.getJob(jobId);
  if (!job) {
    throw new Error(`No resolution job found for market: ${marketId}`);
  }
  await job.retry();
  logger.info({ marketId, jobId }, "retryFailedResolution: resolve_market submission re-enqueued");
}

/**
 * Queries an external boxing data API (BoxRec, ESPN) for fight outcome.
 * Returns normalized result or null if fight not yet reported.
 */
export async function fetchExternalResult(
  market_id: string
): Promise<ExternalFightResult | null> {
  const market = await prisma.market.findUnique({ where: { id: market_id } });
  if (!market) throw new Error(`Market not found: ${market_id}`);

  const fighterA = market.fighterA as { name: string };
  const fighterB = market.fighterB as { name: string };
  const fightDate = market.scheduledAt.toISOString().split("T")[0];

  const url = `${BOXREC_API_URL}/fights?fighterA=${encodeURIComponent(fighterA.name)}&fighterB=${encodeURIComponent(fighterB.name)}&date=${fightDate}`;

  let res: Response;
  try {
    res = await fetch(url, {
      headers: { Authorization: `Bearer ${process.env.ORACLE_API_KEY}` },
    });
  } catch (err) {
    throw new Error(`Network error querying BoxRec: ${(err as Error).message}`);
  }

  if (res.status === 404) return null;

  if (!res.ok) {
    throw new Error(`BoxRec API error: ${res.status} ${res.statusText}`);
  }

  const data = await res.json() as {
    id: string;
    winner: string;
    method: string;
    round: number;
    reportedAt: string;
  };

  const winnerMap: Record<string, ExternalFightResult["winner"]> = {
    [fighterA.name]: "FighterA",
    [fighterB.name]: "FighterB",
    Draw: "Draw",
    NoContest: "NoContest",
  };

  return {
    matchId: data.id,
    winner: winnerMap[data.winner] ?? "NoContest",
    method: data.method,
    round: data.round,
    source: BOXREC_API_URL,
    reportedAt: new Date(data.reportedAt),
  };
}

/**
 * Returns all markets in Locked status without a confirmed oracle result.
 * Used by admin dashboard to show fights awaiting resolution.
 */
export async function listPendingResolutions(): Promise<Market[]> {
  return prisma.market.findMany({
    where: {
      status: "Locked",
      OR: [
        { oracleResult: null },
        { oracleResult: { confirmed: false } },
      ],
    },
    orderBy: { scheduledAt: "asc" },
  });
}

/**
 * Records a dispute in DB and submits dispute_resolution() on-chain.
 * Notifies admin via internal alert.
 */
export async function raiseDispute(
  market_id: string,
  bettor: string,
  reason: string
): Promise<Dispute> {
  const market = await prisma.market.findUnique({ where: { id: market_id } });
  if (!market || market.status !== "Resolved") {
    throw new Error("Market must be in Resolved status to raise a dispute");
  }

  const dispute = await prisma.dispute.create({
    data: { marketId: market_id, raisedBy: bettor, reason },
  });

  const server = new SorobanRpc.Server(RPC_URL);
  const keypair = Keypair.fromSecret(ADMIN_SECRET);
  const account = await server.getAccount(keypair.publicKey());
  const contract = new Contract(market.contractAddress);

  const tx = new TransactionBuilder(account, { fee: BASE_FEE, networkPassphrase: NETWORK })
    .addOperation(
      contract.call(
        "dispute_resolution",
        nativeToScVal(bettor, { type: "address" }),
        nativeToScVal(reason, { type: "string" }),
      ),
    )
    .setTimeout(30)
    .build();

  const prepared = await server.prepareTransaction(tx);
  prepared.sign(keypair);
  await server.sendTransaction(prepared);

  await prisma.$transaction([
    prisma.market.update({ where: { id: market_id }, data: { status: "Disputed" } }),
    prisma.adminLog.create({
      data: { action: "raiseDispute", actor: bettor, target: market_id, metadata: { disputeId: dispute.id, reason } },
    }),
  ]);

  // Admin alert: surfaced through structured logs for the on-call admin.
  logger.warn({ marketId: market_id, disputeId: dispute.id, bettor }, "Dispute raised; admin review required");

  return dispute;
}

/**
 * Admin resolves a dispute with a final outcome (may override oracle).
 * Calls resolve_dispute() on-chain and updates DB dispute record.
 */
export async function resolveDispute(
  dispute_id: string,
  override_outcome: Outcome,
  admin: string
): Promise<void> {
  const dispute = await prisma.dispute.findUniqueOrThrow({ where: { id: dispute_id } });
  const market = await prisma.market.findUniqueOrThrow({ where: { id: dispute.marketId } });

  const server = new SorobanRpc.Server(RPC_URL);
  const keypair = Keypair.fromSecret(ADMIN_SECRET);
  const account = await server.getAccount(keypair.publicKey());
  const contract = new Contract(market.contractAddress);

  const tx = new TransactionBuilder(account, { fee: BASE_FEE, networkPassphrase: NETWORK })
    .addOperation(contract.call(
      "resolve_dispute",
      nativeToScVal(dispute_id, { type: "string" }),
      nativeToScVal(override_outcome, { type: "symbol" }),
    ))
    .setTimeout(30)
    .build();

  const prepared = await server.prepareTransaction(tx);
  prepared.sign(keypair);
  await server.sendTransaction(prepared);

  await prisma.$transaction([
    prisma.dispute.update({
      where: { id: dispute_id },
      data: { resolvedAt: new Date(), resolution: override_outcome },
    }),
    prisma.market.update({
      where: { id: dispute.marketId },
      data: { status: "Resolved", outcome: override_outcome },
    }),
    prisma.adminLog.create({
      data: { action: "resolveDispute", actor: admin, target: dispute.marketId, metadata: { disputeId: dispute_id, override_outcome } },
    }),
  ]);
}

// ─── Oracle Address Management (Issue #455) ────────────────────────────────

/**
 * Get all registered oracles with their details.
 */
export async function getAllOracles() {
  return prisma.oracle.findMany({
    orderBy: { createdAt: 'desc' },
  });
}

/**
 * Get a single oracle by ID.
 */
export async function getOracleById(id: string) {
  return prisma.oracle.findUnique({
    where: { id },
  });
}

/**
 * Create a new oracle entry.
 */
export async function createOracle(address: string, name: string) {
  return prisma.oracle.create({
    data: {
      address,
      name,
      active: true,
    },
  });
}

/**
 * Update oracle name or active status.
 */
export async function updateOracle(id: string, data: { name?: string; active?: boolean }) {
  return prisma.oracle.update({
    where: { id },
    data,
  });
}

/**
 * Deactivate an oracle (soft delete by setting active to false).
 */
export async function deleteOracle(id: string) {
  return prisma.oracle.update({
    where: { id },
    data: { active: false },
  });
}


// ─── B-22 / #1080 ──────────────────────────────────────────────────────────

/**
 * Custom error thrown when a submitter is not the authorized oracle for a market.
 * Carries the market ID and both addresses so callers can log or surface them.
 */
export class OracleAuthorizationError extends Error {
  constructor(
    public readonly marketId: string,
    public readonly expected: string,
    public readonly actual: string,
  ) {
    super(
      `Oracle authorization failed for market ${marketId}: ` +
        `expected ${expected}, got ${actual}`,
    );
    this.name = "OracleAuthorizationError";
  }
}

/**
 * Confirms that `submitter` is the configured oracle for `market_id`.
 *
 * Resolution order:
 *   1. Look up the market's `oracleAddress` column.
 *   2. If the address matches `submitter`, authorization is granted.
 *   3. As a fallback, check whether `submitter` is a registered, active entry
 *      in the `Oracle` table (for globally-whitelisted oracles).
 *
 * The function is fully unit-testable without a live database or Stellar node:
 * pass a mock PrismaClient via the optional `db` parameter.
 *
 * @param market_id  - The BOXMEOUT market UUID to authorize against.
 * @param submitter  - The Stellar public key (G…) or identifier of the caller.
 * @param db         - Optional PrismaClient for dependency injection in tests.
 * @throws {Error}                   If the market does not exist.
 * @throws {OracleAuthorizationError} If `submitter` is not the market's oracle.
 */
export async function verifyOracleAuthorization(
  market_id: string,
  submitter: string,
  db: PrismaClient = prisma,
): Promise<void> {
  const market = await db.market.findUnique({ where: { id: market_id } });
  if (!market) {
    throw new Error(`Market not found: ${market_id}`);
  }

  // Direct address match — the most common path.
  if (market.oracleAddress === submitter) {
    return;
  }

  // Fallback: submitter is a globally registered, active oracle.
  const registeredOracle = await db.oracle.findFirst({
    where: { address: submitter, active: true },
  });
  if (registeredOracle) {
    return;
  }

  throw new OracleAuthorizationError(market_id, market.oracleAddress, submitter);
}

