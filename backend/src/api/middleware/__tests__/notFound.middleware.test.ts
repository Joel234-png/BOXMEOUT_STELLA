import express from "express";
import request from "supertest";
import { notFoundMiddleware } from "../notFound.middleware";
import { errorHandlerMiddleware } from "../errorHandler.middleware";

// Mirrors the registration order in app.ts: routes, then 404, then error handler.
function buildApp() {
  const app = express();
  app.use(express.json());
  app.get("/api/known", (_req, res) => {
    res.json({ ok: true });
  });
  app.get("/api/boom", () => {
    throw new Error("boom");
  });
  app.use(notFoundMiddleware);
  app.use(errorHandlerMiddleware);
  return app;
}

describe("notFoundMiddleware", () => {
  it("returns a JSON 404 with code NOT_FOUND for an unknown path", async () => {
    const res = await request(buildApp()).get("/api/does-not-exist");
    expect(res.status).toBe(404);
    expect(res.headers["content-type"]).toMatch(/application\/json/);
    expect(res.body).toEqual({ error: { code: "NOT_FOUND", message: "Not found" } });
  });

  it("returns a JSON 404 for an unknown method on a known path", async () => {
    const res = await request(buildApp()).post("/api/known").send({});
    expect(res.status).toBe(404);
    expect(res.body).toEqual({ error: { code: "NOT_FOUND", message: "Not found" } });
  });

  it("does not swallow responses from matched routes", async () => {
    const res = await request(buildApp()).get("/api/known");
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ ok: true });
  });

  it("still routes thrown errors to the error handler", async () => {
    const res = await request(buildApp()).get("/api/boom");
    expect(res.status).toBe(500);
    expect(res.body).toMatchObject({ error: { code: "INTERNAL_ERROR" } });
  });
});
