import express from "express";
import request from "supertest";
import { apiKeyAuth } from "../apiKeyAuth";
import { adminAuth } from "../adminAuth";
import { oracleAuth } from "../oracleAuth";
import { config } from "../../../config";

function buildApp(handler: express.RequestHandler) {
  const app = express();
  app.get("/protected", handler, (_req, res) => {
    res.json({ ok: true });
  });
  return app;
}

describe("apiKeyAuth factory", () => {
  const handler = (getKey: () => string | undefined) =>
    apiKeyAuth({ header: "X-Test-Key", getKey });

  it("calls next when the header matches the key", async () => {
    const res = await request(buildApp(handler(() => "secret"))).get("/protected").set("x-test-key", "secret");
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ ok: true });
  });

  it("returns 401 UNAUTHORIZED when the header is missing", async () => {
    const res = await request(buildApp(handler(() => "secret"))).get("/protected");
    expect(res.status).toBe(401);
    expect(res.body).toEqual({ error: "Unauthorized", code: "UNAUTHORIZED" });
  });

  it("returns 401 for a wrong key of the same length", async () => {
    const res = await request(buildApp(handler(() => "secret"))).get("/protected").set("x-test-key", "secrex");
    expect(res.status).toBe(401);
  });

  it("returns 401 for a wrong key of a different length", async () => {
    const res = await request(buildApp(handler(() => "secret"))).get("/protected").set("x-test-key", "s");
    expect(res.status).toBe(401);
  });

  it("returns 401 for every request when the expected key is unset or empty", async () => {
    const unset = await request(buildApp(handler(() => undefined))).get("/protected").set("x-test-key", "");
    const empty = await request(buildApp(handler(() => ""))).get("/protected").set("x-test-key", "");
    expect(unset.status).toBe(401);
    expect(empty.status).toBe(401);
  });

  it("reads the expected key on each request, not once at startup", async () => {
    let current = "first";
    const app = buildApp(handler(() => current));

    expect((await request(app).get("/protected").set("x-test-key", "first")).status).toBe(200);

    current = "second";
    expect((await request(app).get("/protected").set("x-test-key", "first")).status).toBe(401);
    expect((await request(app).get("/protected").set("x-test-key", "second")).status).toBe(200);
  });
});

describe("adminAuth and oracleAuth read keys from config", () => {
  it("adminAuth accepts config.ADMIN_API_KEY in X-Admin-Key", async () => {
    const res = await request(buildApp(adminAuth)).get("/protected").set("x-admin-key", config.ADMIN_API_KEY);
    expect(res.status).toBe(200);
  });

  it("adminAuth rejects the oracle key", async () => {
    const res = await request(buildApp(adminAuth)).get("/protected").set("x-admin-key", config.ORACLE_API_KEY);
    expect(res.status).toBe(401);
  });

  it("oracleAuth accepts config.ORACLE_API_KEY in X-Oracle-Key", async () => {
    const res = await request(buildApp(oracleAuth)).get("/protected").set("x-oracle-key", config.ORACLE_API_KEY);
    expect(res.status).toBe(200);
  });

  it("oracleAuth rejects the admin key", async () => {
    const res = await request(buildApp(oracleAuth)).get("/protected").set("x-oracle-key", config.ADMIN_API_KEY);
    expect(res.status).toBe(401);
  });
});
