import express from "express";
import request from "supertest";
import { requireOwnAddress } from "../requireOwnAddress";

const OWNER = "G" + "A".repeat(55);
const OTHER = "G" + "B".repeat(55);

function buildApp(walletAddress: string | undefined) {
  const app = express();
  app.put(
    "/api/users/:address",
    (req, _res, next) => {
      (req as express.Request & { walletAddress?: string }).walletAddress = walletAddress;
      next();
    },
    requireOwnAddress,
    (_req, res) => {
      res.json({ ok: true });
    },
  );
  return app;
}

describe("requireOwnAddress", () => {
  it("calls next when the authenticated wallet matches the path address", async () => {
    const res = await request(buildApp(OWNER)).put(`/api/users/${OWNER}`);
    expect(res.status).toBe(200);
  });

  it("returns 403 FORBIDDEN when the authenticated wallet is a different address", async () => {
    const res = await request(buildApp(OTHER)).put(`/api/users/${OWNER}`);
    expect(res.status).toBe(403);
    expect(res.body).toEqual({ error: { code: "FORBIDDEN", message: "Wallet does not own this address" } });
  });

  it("returns 403 when no wallet was authenticated", async () => {
    const res = await request(buildApp(undefined)).put(`/api/users/${OWNER}`);
    expect(res.status).toBe(403);
  });
});
