import { expect, test } from "./fixtures";

// Guards against a server that boots but serves a broken router (for example
// route-syntax errors that only show up per path). Extension point: add
// /healthz and /readyz here once they exist on main.
test.describe("server and router", () => {
  test("parameterized API route resolves and reports a JSON 404", async ({
    request,
  }) => {
    const res = await request.get(
      "/api/documents/00000000-0000-0000-0000-000000000000",
    );
    expect(res.status()).toBe(404);
    expect(await res.json()).toHaveProperty("error");
  });

  test("OpenAPI spec is served", async ({ request }) => {
    const res = await request.get("/api/openapi.json");
    expect(res.ok()).toBeTruthy();
    const spec = await res.json();
    expect(spec.openapi).toMatch(/^3\./);
    expect(Object.keys(spec.paths)).toContain("/api/documents");
  });

  test("SSE endpoint streams", async () => {
    // request.get would wait for the body of an endless stream, so only
    // check the headers through a short-lived fetch.
    const controller = new AbortController();
    const res = await fetch(`${process.env.FOLIO_BASE_URL}/events`, {
      signal: controller.signal,
    });
    expect(res.headers.get("content-type")).toContain("text/event-stream");
    controller.abort();
  });

  test("static assets are served", async ({ request }) => {
    const res = await request.get("/static/style.css");
    expect(res.ok()).toBeTruthy();
  });

  test("detail page for an unknown document is a 404", async ({ request }) => {
    const res = await request.get(
      "/documents/00000000-0000-0000-0000-000000000000",
    );
    expect(res.status()).toBe(404);
  });
});
