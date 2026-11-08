import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm, symlink } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const binary =
  process.env.FOLIO_BIN ?? path.join(repo, "target/debug/folio-app");
const directory = await mkdtemp(path.join(tmpdir(), "folio-telemetry-"));
const captures = [];
// The server's own protobuf export, kept raw: only its trace IDs are checked.
const serverExports = [];
const receiver = createServer(async (request, response) => {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  const body = Buffer.concat(chunks);
  if (request.headers["content-type"] === "application/x-protobuf")
    serverExports.push({ signal: request.url.split("/").pop(), body });
  else
    captures.push({
      signal: request.url.split("/").pop(),
      headers: request.headers,
      body: JSON.parse(body.toString()),
    });
  response.writeHead(200, { "content-type": "application/json" });
  response.end("{}");
});
let app;
let browser;
let output = "";

function attributes(values) {
  return Object.fromEntries(
    values.map(({ key, value }) => [key, Object.values(value)[0]]),
  );
}

async function listen(server) {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return server.address().port;
}

try {
  const collectorPort = await listen(receiver);
  const reservation = createServer();
  const appPort = await listen(reservation);
  await new Promise((resolve) => reservation.close(resolve));
  const origin = `http://127.0.0.1:${appPort}`;
  await symlink(path.join(repo, "static"), path.join(directory, "static"));
  app = spawn(binary, [], {
    cwd: directory,
    env: {
      PATH: process.env.PATH,
      HOME: directory,
      RUST_LOG: "info",
      FOLIO_SERVER__HOST: "127.0.0.1",
      FOLIO_SERVER__PORT: String(appPort),
      FOLIO_SERVER__PUBLIC_URL: origin,
      FOLIO_DATABASE__URL: `sqlite:${directory}/folio.db?mode=rwc`,
      FOLIO_STORAGE__PATH: path.join(directory, "storage"),
      FOLIO_AI__PROVIDER: "local",
      FOLIO_AUTH__REQUIRED: "true",
      FOLIO_TELEMETRY__CLIENTS_ENABLED: "true",
      FOLIO_TELEMETRY__OTLP_HTTP_ENDPOINT: `http://127.0.0.1:${collectorPort}`,
      OTEL_EXPORTER_OTLP_ENDPOINT: `http://127.0.0.1:${collectorPort}`,
      OTEL_EXPORTER_OTLP_PROTOCOL: "http/protobuf",
      OTEL_SERVICE_NAME: "folio-server",
      OTEL_EXPORTER_OTLP_HEADERS: "x-api-key=local-server-key",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  app.stdout.on("data", (chunk) => {
    output += chunk;
  });
  app.stderr.on("data", (chunk) => {
    output += chunk;
  });
  let startError;
  app.on("error", (error) => {
    startError = error;
  });
  await expect
    .poll(
      async () => {
        if (startError) throw startError;
        if (app.exitCode !== null)
          throw new Error(`Folio exited with ${app.exitCode}`);
        return fetch(`${origin}/readyz`)
          .then((response) => response.status)
          .catch(() => 0);
      },
      { timeout: 30_000 },
    )
    .toBe(200);
  assert.equal((await fetch(`${origin}/api/telemetry/config`)).status, 401);
  assert.equal(
    (
      await fetch(`${origin}/api/telemetry/v1/traces`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: "{}",
      })
    ).status,
    401,
  );
  const setupToken = output.match(/setup\?token=([A-Za-z0-9_-]+)/)?.[1];
  assert.ok(setupToken, "ephemeral owner setup token was logged");
  browser = await chromium.launch();
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${origin}/setup?token=${setupToken}`);
  await page.getByLabel("Username", { exact: true }).fill("telemetry-owner");
  await page
    .getByLabel("Password", { exact: true })
    .fill("temporary-telemetry-password");
  await page.getByRole("button", { name: "Create account" }).click();
  await page.waitForURL(`${origin}/`);
  await expect
    .poll(() =>
      page.evaluate(() =>
        localStorage.getItem("folio.telemetry.installation.id"),
      ),
    )
    .toMatch(/^[0-9a-f-]{36}$/);
  const configuration = await page.evaluate(() =>
    fetch("/api/telemetry/config").then((response) => response.json()),
  );
  assert.equal(configuration.enabled, true);
  assert.ok(!JSON.stringify(configuration).includes("local-server-key"));
  const requests = [];
  page.on("request", (request) => {
    if (request.url().includes("private-query"))
      requests.push(request.allHeaders());
  });
  await page.evaluate(async () => {
    await fetch(
      "/api/documents/00000000-0000-0000-0000-000000000000?search=private-query",
    );
    await new Promise((resolve) => {
      const xhr = new XMLHttpRequest();
      xhr.open("GET", "/api/documents?search=private-query");
      xhr.onload = resolve;
      xhr.send();
    });
  });
  await expect
    .poll(() => captures.map((capture) => capture.signal), { timeout: 15_000 })
    .toEqual(expect.arrayContaining(["traces", "metrics", "logs"]));
  const traceResources = captures
    .filter((capture) => capture.signal === "traces")
    .flatMap((capture) => capture.body.resourceSpans);
  const spans = traceResources
    .flatMap((resource) => resource.scopeSpans)
    .flatMap((scope) => scope.spans);
  const clientHeaders = await Promise.all(requests);
  assert.equal(clientHeaders.length, 2);
  for (const headers of clientHeaders) {
    const [, traceId, spanId] = headers.traceparent.split("-");
    assert.ok(
      spans.some((span) => span.traceId === traceId && span.spanId === spanId),
      "Folio API received the exported browser client span context",
    );
    assert.equal(headers["x-api-key"], undefined);
  }
  for (const capture of captures) {
    assert.equal(capture.headers["x-api-key"], "local-server-key");
    assert.equal(capture.headers.cookie, undefined);
    const resources =
      capture.body.resourceSpans ??
      capture.body.resourceMetrics ??
      capture.body.resourceLogs;
    const resource = attributes(resources[0].resource.attributes);
    assert.equal(resource["service.name"], "folio-web");
    assert.equal(resource["service.namespace"], "folio");
    assert.match(resource["folio.installation.id"], /^[0-9a-f-]{36}$/);
  }
  assert.ok(
    !JSON.stringify(captures.map((capture) => capture.body)).includes(
      "private-query",
    ),
  );
  await expect
    .poll(
      () =>
        captures
          .filter((capture) => capture.signal === "logs")
          .flatMap((capture) => capture.body.resourceLogs)
          .flatMap((resource) => resource.scopeLogs)
          .flatMap((scope) => scope.logRecords)
          .filter((record) => record.eventName === "folio.screen.view")
          .map((record) => attributes(record.attributes)),
      { timeout: 15_000 },
    )
    .toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          "app.screen.name": "inbox",
          "session.id": expect.stringMatching(/^[0-9a-f-]{36}$/),
        }),
      ]),
    );
  const pageLoad = spans.find(
    (span) =>
      span.name === "folio.navigation" &&
      attributes(span.attributes)["folio.navigation.type"] === "document",
  );
  assert.match(
    pageLoad?.parentSpanId ?? "",
    /^[0-9a-f]{16}$/,
    "the page load joined the server request named in Server-Timing",
  );
  await expect
    .poll(
      () =>
        serverExports.some(
          (capture) =>
            capture.signal === "traces" &&
            capture.body.includes(Buffer.from(pageLoad.traceId, "hex")) &&
            capture.body.includes(Buffer.from(pageLoad.parentSpanId, "hex")),
        ),
      {
        message: "the server exported the request span the page load names",
        timeout: 20_000,
      },
    )
    .toBe(true);
  assert.deepEqual(errors, []);
  console.log(
    "Authenticated browser → Folio → local OTLP receiver verified: traces, metrics, logs, traceparent, session and screen events, Server-Timing page-load link, private resources, server-only credentials.",
  );
} finally {
  await browser?.close();
  if (app && app.exitCode === null) {
    app.kill("SIGTERM");
    await new Promise((resolve) => {
      const timeout = setTimeout(() => {
        app.kill("SIGKILL");
        resolve();
      }, 3000);
      app.once("exit", () => {
        clearTimeout(timeout);
        resolve();
      });
    });
  }
  await new Promise((resolve) => receiver.close(resolve));
  await rm(directory, { recursive: true, force: true });
}
