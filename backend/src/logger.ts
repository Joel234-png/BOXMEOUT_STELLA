import pino from "pino";
import pinoHttp from "pino-http";
import type { IncomingMessage } from "http";

const isDev = process.env.NODE_ENV !== "production";

export const logger = pino({
  level: isDev ? "debug" : "info",
  timestamp: pino.stdTimeFunctions.isoTime,
  base: { service: "boxmeout-backend" },
  ...(isDev && {
    transport: {
      target: "pino-pretty",
      options: {
        colorize: true,
        translateTime: "SYS:standard",
        ignore: "pid,hostname,service",
      },
    },
  }),
});

export const httpLogger = pinoHttp({
  logger,
  // B-59: Carry the request ID (set by requestIdMiddleware) into every log line
  genReqId(req: IncomingMessage): string {
    // req.id is set by requestIdMiddleware before pinoHttp runs
    return (req as IncomingMessage & { id?: string }).id ?? "";
  },
  customLogLevel(_req, res) {
    if (res.statusCode >= 500) return "error";
    if (res.statusCode >= 400) return "warn";
    return "info";
  },
  serializers: {
    req(req) {
      return { method: req.method, url: req.url, reqId: req.id };
    },
    res(res) {
      return { statusCode: res.statusCode };
    },
  },
});
