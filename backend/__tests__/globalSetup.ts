/**
 * Runs once before the whole Jest run. Database migrations are applied by the
 * CI workflow (`prisma migrate deploy`) before `npm test`, so there is nothing
 * to prepare here yet.
 */
export default async function globalSetup(): Promise<void> {}
