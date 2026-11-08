import { createServer, type IncomingHttpHeaders } from "node:http";
import { readFileSync } from "node:fs";
import path from "node:path";
import { expect, test as base } from "@playwright/test";

type Capture = { signal: string; body: any; headers: IncomingHttpHeaders };
type Receiver = {
  origin: string;
  captures: Capture[];
  requests: { url: string; headers: IncomingHttpHeaders }[];
  enabled: boolean;
  configStatus: number;
  exportStatus: number;
};

const serverTraceId = "4bf92f3577b34da6a3ce929d0e0e4736";
const serverSpanId = "00f067aa0ba902b7";
const documentId = "3f6c1f4e-2b7a-4c1d-9e8f-0a1b2c3d4e5f";

/** Just enough of each Folio page for telemetry: its screen and its actions. */
function pageBody(pathname: string) {
  const screen = (name: string) =>
    `<meta name="folio-screen" content="${name}">`;
  if (pathname === "/")
    return `${screen("inbox")}<h1>Folio</h1><a data-folio-action="document.open" href="/documents/${documentId}">private-title</a>`;
  if (pathname.startsWith("/documents/"))
    return `${screen("document.detail")}<h1>Folio</h1><form data-folio-action="document.delete" action="${pathname}/delete" method="post" data-turbo="false"><button>Delete</button></form><button data-folio-action="document.secret">Unlisted</button>`;
  if (pathname === "/trash") return `${screen("trash")}<h1>Folio</h1>`;
  return "<h1>Folio</h1>";
}

const test = base.extend<{ receiver: Receiver }>({
  receiver: async ({}, use) => {
    const receiver: Receiver = {
      origin: "",
      captures: [],
      requests: [],
      enabled: true,
      configStatus: 200,
      exportStatus: 200,
    };
    const server = createServer(async (request, response) => {
      const url = request.url!;
      receiver.requests.push({ url, headers: request.headers });
      response.setHeader("access-control-allow-origin", "*");
      if (url === "/api/telemetry/config") {
        response.writeHead(receiver.configStatus, {
          "content-type": "application/json",
        });
        response.end(
          JSON.stringify({
            enabled: receiver.enabled,
            endpoint: "/api/telemetry",
            serviceNamespace: "folio",
            serviceVersion: "0.3.1",
          }),
        );
      } else if (url.startsWith("/api/telemetry/v1/")) {
        const chunks: Buffer[] = [];
        for await (const chunk of request) chunks.push(Buffer.from(chunk));
        receiver.captures.push({
          signal: url.split("/").pop()!,
          body: JSON.parse(Buffer.concat(chunks).toString()),
          headers: request.headers,
        });
        response.writeHead(receiver.exportStatus, {
          "content-type": "application/json",
        });
        response.end("{}");
      } else if (url === "/static/vendor/telemetry.js") {
        response.setHeader("content-type", "text/javascript");
        response.end(
          readFileSync(
            path.resolve(__dirname, "../../static/vendor/telemetry.js"),
          ),
        );
      } else if (url.startsWith("/api/")) {
        response.setHeader("content-type", "application/json");
        response.end('{"content":"private-response-content"}');
      } else {
        const pathname = new URL(url, receiver.origin).pathname;
        if (request.method === "POST" && pathname.endsWith("/delete")) {
          response.writeHead(303, { location: "/trash" });
          response.end();
          return;
        }
        response.setHeader("content-type", "text/html");
        response.setHeader(
          "server-timing",
          `traceparent;desc="00-${serverTraceId}-${serverSpanId}-01"`,
        );
        response.end(
          `<!doctype html><title>Folio telemetry test</title>${pageBody(pathname)}<script type="module" src="/static/vendor/telemetry.js"></script>`,
        );
      }
    });
    await new Promise<void>((resolve) =>
      server.listen(0, "127.0.0.1", resolve),
    );
    const address = server.address() as { port: number };
    receiver.origin = `http://127.0.0.1:${address.port}`;
    try {
      await use(receiver);
    } finally {
      await new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      );
    }
  },
});

test.describe.configure({ timeout: 30_000 });

function attributes(values: any[]) {
  return Object.fromEntries(
    values.map(({ key, value }) => [key, Object.values(value)[0]]),
  );
}

function spans(receiver: Receiver) {
  return receiver.captures
    .filter((capture) => capture.signal === "traces")
    .flatMap((capture) => capture.body.resourceSpans)
    .flatMap((resource) => resource.scopeSpans)
    .flatMap((scope) => scope.spans);
}

test("exports real OTLP signals with persistent context and private correlated requests", async ({
  page,
  context,
  receiver,
}) => {
  test.setTimeout(30_000);
  await context.addCookies([
    { name: "folio_session", value: "test-session", url: receiver.origin },
  ]);
  await page.goto(
    `${receiver.origin}/documents/private-document?search=private-query#private-fragment`,
  );
  await expect
    .poll(async () =>
      page.evaluate(() =>
        localStorage.getItem("folio.telemetry.installation.id"),
      ),
    )
    .toMatch(/^[0-9a-f-]{36}$/);
  const installation = await page.evaluate(() =>
    localStorage.getItem("folio.telemetry.installation.id"),
  );
  await page.evaluate(async () => {
    await fetch("/api/documents/private-document?search=private-query", {
      headers: { authorization: "Bearer private-token" },
    });
    await fetch("/api/documents/private-document", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        title: "private-title",
        content: "private-document-content",
      }),
    });
    await new Promise<void>((resolve) => {
      const xhr = new XMLHttpRequest();
      xhr.open("GET", "/api/documents/private-xhr?search=private-query");
      xhr.onload = () => resolve();
      xhr.send();
    });
    await fetch(
      location.origin.replace("127.0.0.1", "localhost") +
        "/api/external?secret=private-external",
    );
  });
  await expect
    .poll(() => receiver.captures.map((capture) => capture.signal), {
      timeout: 15_000,
    })
    .toEqual(expect.arrayContaining(["traces", "metrics", "logs"]));
  const resource = receiver.captures.find(
    (capture) => capture.signal === "traces",
  )!.body.resourceSpans[0].resource;
  const contextAttributes = attributes(resource.attributes);
  expect(contextAttributes).toMatchObject({
    "service.name": "folio-web",
    "service.namespace": "folio",
    "service.version": "0.3.1",
    "folio.installation.id": installation,
    "browser.name": "Chrome",
    "device.type": "desktop",
  });
  expect(contextAttributes["service.instance.id"]).toMatch(/^[0-9a-f-]{36}$/);
  const request = receiver.requests.find((request) =>
    request.url.startsWith("/api/documents/private-document"),
  )!;
  expect(request.headers.traceparent).toMatch(
    /^00-[0-9a-f]{32}-[0-9a-f]{16}-01$/,
  );
  const [, traceId, spanId] = String(request.headers.traceparent).split("-");
  expect(spans(receiver)).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ traceId, spanId, name: "HTTP GET" }),
    ]),
  );
  expect(
    spans(receiver).filter((span) => span.name === "HTTP GET"),
  ).toHaveLength(2);
  expect(
    receiver.requests.find((request) =>
      request.url.startsWith("/api/external"),
    )!.headers.traceparent,
  ).toBeUndefined();
  expect(
    receiver.requests.find(
      (request) => request.url === "/api/telemetry/config",
    )!.headers.traceparent,
  ).toBeUndefined();
  expect(receiver.captures[0].headers.cookie).toContain(
    "folio_session=test-session",
  );
  expect(receiver.captures[0].headers.traceparent).toBeUndefined();
  expect(
    JSON.stringify(receiver.captures.map((capture) => capture.body)),
  ).not.toMatch(
    /private-document|private-xhr|private-query|private-fragment|private-token|private-title|private-response-content|private-external|test-session|url.full|http.url/,
  );
  expect(spans(receiver)).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        name: "folio.navigation",
        attributes: expect.arrayContaining([
          { key: "http.route", value: { stringValue: "/documents/:id" } },
        ]),
      }),
    ]),
  );
  await page.reload();
  await expect
    .poll(
      () =>
        receiver.captures.filter((capture) => capture.signal === "traces")
          .length,
    )
    .toBeGreaterThan(1);
  const nextResource = receiver.captures
    .filter((capture) => capture.signal === "traces")
    .at(-1)!.body.resourceSpans[0].resource;
  expect(attributes(nextResource.attributes)["folio.installation.id"]).toBe(
    installation,
  );
  expect(attributes(nextResource.attributes)["service.instance.id"]).not.toBe(
    contextAttributes["service.instance.id"],
  );
});

test("waits for authenticated enabled configuration and retries after Turbo sign in", async ({
  page,
  receiver,
}) => {
  receiver.configStatus = 401;
  await page.goto(receiver.origin);
  await page.evaluate(() => fetch("/api/documents?private=disabled"));
  expect(
    await page.evaluate(() =>
      localStorage.getItem("folio.telemetry.installation.id"),
    ),
  ).toBeNull();
  expect(
    receiver.requests.find((request) =>
      request.url.startsWith("/api/documents"),
    )!.headers.traceparent,
  ).toBeUndefined();
  expect(receiver.captures).toHaveLength(0);
  receiver.configStatus = 200;
  await page.evaluate(() => document.dispatchEvent(new Event("turbo:load")));
  await expect.poll(() => receiver.captures.length).toBeGreaterThan(0);
  expect(spans(receiver)).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ name: "folio.navigation" }),
    ]),
  );
});

test("disabled configuration sends no telemetry", async ({
  page,
  receiver,
}) => {
  receiver.enabled = false;
  await page.clock.install();
  await page.goto(receiver.origin);
  await page.waitForLoadState("networkidle");
  await page.evaluate(() => fetch("/api/documents"));
  await page.clock.fastForward(15_000);
  expect(
    await page.evaluate(() =>
      localStorage.getItem("folio.telemetry.installation.id"),
    ),
  ).toBeNull();
  expect(receiver.captures).toHaveLength(0);
});

test("exporter outages never break the page", async ({ page, receiver }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  receiver.exportStatus = 503;
  await page.goto(receiver.origin);
  await expect.poll(() => receiver.captures.length).toBeGreaterThan(0);
  await expect(page.getByRole("heading", { name: "Folio" })).toBeVisible();
  expect(errors).toEqual([]);
});

test("restricted browser storage and unavailable randomUUID still export anonymous context", async ({
  page,
  receiver,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(crypto, "randomUUID", { value: undefined });
    Object.defineProperty(Storage.prototype, "getItem", {
      value: () => {
        throw new Error("Storage blocked");
      },
    });
  });
  await page.goto(receiver.origin);
  await expect.poll(() => receiver.captures.length).toBeGreaterThan(0);
  const resource = receiver.captures.find(
    (capture) => capture.signal === "traces",
  )!.body.resourceSpans[0].resource;
  expect(attributes(resource.attributes)["folio.installation.id"]).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  );
  await expect(page.getByRole("heading", { name: "Folio" })).toBeVisible();
});

test("sign-in pages do not make unauthorized telemetry requests", async ({ page, receiver }) => {
  await page.goto(`${receiver.origin}/login`);
  await page.waitForLoadState("networkidle");
  expect(receiver.requests.filter((request) => request.url.startsWith("/api/telemetry/"))).toHaveLength(0);
  await page.evaluate(() => {
    history.pushState({}, "", "/");
    document.dispatchEvent(new Event("turbo:load"));
  });
  await expect.poll(() => receiver.captures.length).toBeGreaterThan(0);
});

function logRecords(receiver: Receiver) {
  return receiver.captures
    .filter((capture) => capture.signal === "logs")
    .flatMap((capture) => capture.body.resourceLogs)
    .flatMap((resource) => resource.scopeLogs)
    .flatMap((scope) => scope.logRecords)
    .sort((a, b) => Number(BigInt(a.timeUnixNano) - BigInt(b.timeUnixNano)));
}

test("follows one session across screens and actions without IDs or contents", async ({
  page,
  receiver,
}) => {
  await page.goto(receiver.origin);
  await expect
    .poll(() => receiver.requests.some((r) => r.url === "/api/telemetry/config"))
    .toBe(true);
  await page.getByRole("link", { name: "private-title" }).click();
  await page.waitForURL(`**/documents/${documentId}`);
  await page.getByRole("button", { name: "Unlisted" }).click();
  await page.getByRole("button", { name: "Delete" }).click();
  await page.waitForURL("**/trash");

  const screens = () =>
    logRecords(receiver)
      .filter((record) => record.eventName === "folio.screen.view")
      .map((record) => attributes(record.attributes)["app.screen.name"]);
  await expect
    .poll(screens, { timeout: 20_000 })
    .toEqual(["inbox", "document.detail", "trash"]);
  await expect
    .poll(() =>
      logRecords(receiver)
        .filter((record) => record.eventName === "folio.action")
        .map((record) => attributes(record.attributes)),
    )
    .toEqual([
      expect.objectContaining({
        "folio.action.name": "document.open",
        "app.screen.name": "inbox",
      }),
      expect.objectContaining({
        "folio.action.name": "document.delete",
        "app.screen.name": "document.detail",
      }),
    ]);

  const records = [...logRecords(receiver), ...spans(receiver)];
  const sessions = new Set(
    records.map((record) => attributes(record.attributes)["session.id"]),
  );
  expect([...sessions]).toHaveLength(1);
  expect([...sessions][0]).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  );
  expect(
    logRecords(receiver).filter((record) => record.eventName === "session.start"),
  ).toHaveLength(1);

  const loads = spans(receiver).filter(
    (span) =>
      span.name === "folio.navigation" &&
      attributes(span.attributes)["folio.navigation.type"] === "document",
  );
  expect(loads.length).toBeGreaterThan(0);
  for (const load of loads) {
    expect(load.traceId).toBe(serverTraceId);
    expect(load.parentSpanId).toBe(serverSpanId);
  }

  const exported = JSON.stringify(receiver.captures.map((capture) => capture.body));
  expect(exported).not.toContain(documentId);
  expect(exported).not.toContain("private-title");
  expect(exported).not.toContain("document.secret");
  expect(exported).not.toMatch(/"stringValue":"[^"]*\?/);
});
