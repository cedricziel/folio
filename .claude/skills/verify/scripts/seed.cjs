// Deterministic demo data for screen tours and visual snapshots.
// Usage as a module: `await seed(request)` with a Playwright APIRequestContext
// (cookies included, so it works with auth on after login).
// Usage from the shell: `node seed.cjs <env-file>` (auth off only).

const correspondents = [
  "Stadtwerke Köln",
  "HUK-Coburg",
  "Finanzamt Köln-Mitte",
  "Dr. med. Weber",
];
const documentTypes = ["Invoice", "Contract", "Tax notice", "Letter"];
const tags = [
  "Utilities",
  "Insurance",
  "Tax 2025",
  "Health",
  "Car",
  "Apartment",
];

// Each entry becomes one PDF upload. `from`/`type`/`tags` are applied after the
// worker has finished, so the inbox shows a mix of filed and unfiled rows.
const documents = [
  {
    title: "Jahresabrechnung Strom 2025",
    from: "Stadtwerke Köln",
    type: "Invoice",
    tags: ["Utilities", "Tax 2025"],
    pages: [
      [
        "Stadtwerke Köln GmbH",
        "Jahresabrechnung Strom 2025",
        "Rechnungsnr. 2025-00318",
        "Verbrauch 2.184 kWh",
        "Nachzahlung 84,20 EUR",
      ],
      [
        "Verbrauchsübersicht",
        "Zählerstand alt 41.233",
        "Zählerstand neu 43.417",
      ],
      ["Allgemeine Hinweise", "Fällig am 01.04.2025"],
    ],
  },
  {
    title: "Beitragsrechnung Kfz-Versicherung",
    from: "HUK-Coburg",
    type: "Invoice",
    tags: ["Insurance", "Car"],
    pages: [
      [
        "HUK-Coburg",
        "Beitragsrechnung 2025",
        "Kfz-Haftpflicht K-AB 1234",
        "Jahresbeitrag 412,60 EUR",
      ],
    ],
  },
  {
    title: "Einkommensteuerbescheid 2024",
    from: "Finanzamt Köln-Mitte",
    type: "Tax notice",
    tags: ["Tax 2025"],
    pages: [
      [
        "Finanzamt Köln-Mitte",
        "Bescheid für 2024 über Einkommensteuer",
        "Erstattung 1.204,00 EUR",
      ],
    ],
  },
  {
    title: "Laborbefund",
    from: "Dr. med. Weber",
    tags: ["Health"],
    pages: [
      [
        "Praxis Dr. med. Weber",
        "Laborbefund vom 10.03.2025",
        "Alle Werte im Normbereich",
      ],
    ],
  },
  {
    title: "Mietvertrag Ehrenfeld",
    type: "Contract",
    tags: ["Apartment"],
    pages: [
      [
        "Mietvertrag",
        "Wohnung Venloer Str. 12, 50823 Köln",
        "Kaltmiete 980,00 EUR",
      ],
      ["Paragraph 2", "Mietdauer unbefristet"],
    ],
  },
  {
    title: "Kündigungsbestätigung Fitnessstudio",
    pages: [["Kündigungsbestätigung", "Ihr Vertrag endet zum 30.06.2025"]],
  },
  {
    // A long title to exercise truncation in rows and headers.
    title:
      "Nebenkostenabrechnung für den Abrechnungszeitraum 01.01.2024 bis 31.12.2024 Venloer Straße",
    from: "Stadtwerke Köln",
    type: "Letter",
    pages: [["Nebenkostenabrechnung", "Guthaben 132,18 EUR"]],
  },
];

// Not a PDF: the worker fails on it, which gives the failed-processing state.
const brokenDocument = { title: "scan_0042", filename: "scan_0042.pdf" };

function pdf(pages) {
  const objects = [];
  const add = (body) => objects.push(body) && objects.length;
  const catalog = add(null);
  const pagesId = add(null);
  const font = add(
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
  );
  const kids = pages.map((lines) => {
    const esc = (s) => s.replace(/[\\()]/g, (c) => `\\${c}`);
    const text = lines
      .map(
        (l, i) =>
          `BT /F1 ${i === 0 ? 18 : 12} Tf 64 ${760 - i * 28} Td (${esc(l)}) Tj ET`,
      )
      .join("\n");
    const content = add(
      `<< /Length ${Buffer.byteLength(text, "latin1")} >>\nstream\n${text}\nendstream`,
    );
    return add(
      `<< /Type /Page /Parent ${pagesId} 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 ${font} 0 R >> >> /Contents ${content} 0 R >>`,
    );
  });
  objects[catalog - 1] = `<< /Type /Catalog /Pages ${pagesId} 0 R >>`;
  objects[pagesId - 1] =
    `<< /Type /Pages /Kids [${kids.map((k) => `${k} 0 R`).join(" ")}] /Count ${kids.length} >>`;
  let out = "%PDF-1.4\n";
  const offsets = objects.map((body, i) => {
    const at = Buffer.byteLength(out, "latin1");
    out += `${i + 1} 0 obj\n${body}\nendobj\n`;
    return at;
  });
  const xref = Buffer.byteLength(out, "latin1");
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  out += offsets
    .map((o) => `${String(o).padStart(10, "0")} 00000 n \n`)
    .join("");
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(out, "latin1");
}

async function json(res, what) {
  if (!res.ok())
    throw new Error(`seed: ${what} -> ${res.status()} ${await res.text()}`);
  const type = res.headers()["content-type"] ?? "";
  return type.includes("json") ? res.json() : null;
}

async function waitProcessed(request, ids, { ocrOnly = false, timeoutMs = 60_000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const { documents: docs } = await json(
      await request.get("/api/documents"),
      "list documents",
    );
    const pending = ids.filter((id) => {
      const d = docs.find((x) => x.id === id);
      const busy = (st) => st === "pending" || st === "running";
      return !d || busy(d.ocr_status) || (!ocrOnly && busy(d.embed_status));
    });
    if (pending.length === 0) return;
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error("seed: documents were not processed in time");
}

// Idempotent: does nothing when documents already exist.
async function seed(request, { withBroken = true } = {}) {
  const existing = await json(
    await request.get("/api/documents"),
    "list documents",
  );
  if ((existing.documents ?? []).length > 0) return { skipped: true };

  const ids = { correspondents: {}, types: {}, tags: {} };
  for (const name of correspondents)
    ids.correspondents[name] = (
      await json(
        await request.post("/api/correspondents", { data: { name } }),
        name,
      )
    ).id;
  for (const name of documentTypes)
    ids.types[name] = (
      await json(
        await request.post("/api/document_types", { data: { name } }),
        name,
      )
    ).id;
  for (const name of tags)
    ids.tags[name] = (
      await json(await request.post("/api/tags", { data: { name } }), name)
    ).id;

  const uploaded = [];
  // Oldest first, so the list order (newest first) is stable.
  for (const doc of [...documents].reverse()) {
    const res = await request.post("/api/documents/upload", {
      multipart: {
        title: doc.title,
        file: {
          name: `${doc.title.slice(0, 24).replace(/\W+/g, "_")}.pdf`,
          mimeType: "application/pdf",
          buffer: pdf(doc.pages),
        },
      },
    });
    uploaded.push({ doc, id: (await json(res, `upload ${doc.title}`)).id });
  }
  await waitProcessed(
    request,
    uploaded.map((u) => u.id),
  );

  for (const { doc, id } of uploaded) {
    if (doc.from)
      await json(
        await request.post(`/api/documents/${id}/correspondent`, {
          form: { correspondent_id: ids.correspondents[doc.from] },
        }),
        "set correspondent",
      );
    if (doc.type)
      await json(
        await request.post(`/api/documents/${id}/document_type`, {
          form: { document_type_id: ids.types[doc.type] },
        }),
        "set type",
      );
    for (const t of doc.tags ?? [])
      await json(
        await request.post(`/api/documents/${id}/tags`, {
          form: { tag_id: ids.tags[t] },
        }),
        "add tag",
      );
  }

  await json(
    await request.post("/api/workflows", {
      data: {
        name: "File power bills",
        enabled: true,
        trigger: "document.processed",
        conditions: { filename_glob: "*strom*" },
        actions: [
          {
            type: "set_correspondent",
            correspondent_id: ids.correspondents["Stadtwerke Köln"],
          },
          { type: "add_tags", tag_ids: [ids.tags.Utilities] },
        ],
      },
    }),
    "create workflow",
  );
  await json(
    await request.post("/api/sources", {
      data: {
        name: "Scanner inbox",
        enabled: false,
        config: { type: "folder", path: "/srv/scans", recursive: false },
      },
    }),
    "create source",
  );

  if (withBroken) {
    const res = await request.post("/api/documents/upload", {
      multipart: {
        title: brokenDocument.title,
        file: {
          name: brokenDocument.filename,
          mimeType: "application/pdf",
          buffer: Buffer.from("not a pdf\n"),
        },
      },
    });
    const broken = await json(res, "upload broken");
    // Wait for the worker to give up, so screenshots show a settled state.
    await waitProcessed(request, [broken.id], { ocrOnly: true });
  }
  return { skipped: false, ids, documents: uploaded };
}

module.exports = { seed, pdf, documents, brokenDocument };

if (require.main === module) {
  const { readFileSync } = require("node:fs");
  const path = require("node:path");
  const envFile = process.argv[2] ?? process.env.FOLIO_VERIFY_ENV;
  if (!envFile) throw new Error("usage: node seed.cjs <env-file>");
  const env = Object.fromEntries(
    readFileSync(envFile, "utf8")
      .split("\n")
      .filter(Boolean)
      .map((l) => l.split(/=(.*)/s).slice(0, 2)),
  );
  const { request } = require(
    require.resolve("@playwright/test", { paths: [path.resolve(__dirname, "../../../../e2e")] }),
  );
  (async () => {
    const ctx = await request.newContext({ baseURL: env.BASE_URL });
    console.log(JSON.stringify(await seed(ctx), null, 2));
    await ctx.dispose();
  })().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
