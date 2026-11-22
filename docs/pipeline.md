# Processing pipeline

Uploaded documents flow through these stages:

```
upload -> ProcessDocument (thumbnails)
       -> ExtractText   (if processing.auto_ocr)
       -> GenerateEmbedding (if processing.auto_embed and text was found)
```

Each document tracks `ocr_status` and `embed_status`
(`pending`, `running`, `done`, `failed`, `skipped`). A successful OCR run that
finds no text stores an empty string and status `done`; the repair pass only
re-queues documents whose stage is still `pending` and has no active job.

## OCR

Folio reads the embedded text on each PDF page with `pdftotext`. It uses that
text on pages with at least 20 letters or digits. It renders the remaining pages at 300 DPI
with `pdftoppm` and sends those images to the OCR provider. If text extraction
fails, it sends every page to the provider. Folio combines the results in page
order, so scanned pages contribute searchable text even in a mostly digital PDF.

| `ocr.provider` | What it does | Needs |
| --- | --- | --- |
| `tesseract` (default) | Local `tesseract` CLI, reports confidence | `tesseract-ocr` plus language packs |
| `ocrmypdf` | Local OCRmyPDF CLI, extracts a text sidecar from rendered pages | `ocrmypdf`, Tesseract language packs, and `poppler-utils` for PDFs |
| `openai-vision` | OpenAI-compatible chat completions with image input | `ai.openai_api_key` (optional with a custom base URL) |
| `anthropic-vision` | Anthropic messages API with image input | `ai.anthropic_api_key` |
| `ollama-vision` | Ollama `/api/chat` with images | Ollama server, e.g. `llama3.2-vision` |
| `none` | Skip OCR (`ocr_status = skipped`) | nothing |

Settings: `ocr.provider`, `ocr.model` (empty = provider default),
`ocr.languages` (tesseract style, e.g. `deu+eng`), `ocr.base_url`.

## Embeddings

Text is split into overlapping windows (1500 characters, 200 overlap) and each
chunk is embedded. Chunks are stored with the model id and dimensions. Search
embeds the query with the active model, compares only vectors of that model and
returns the best chunk score per document.

| `embeddings.provider` | What it does |
| --- | --- |
| `none` (default) | Semantic search off (`embed_status = skipped`) |
| `openai` | `POST {base_url}/embeddings`; works with OpenAI, LocalAI, vLLM, LM Studio |
| `ollama` | `POST {base_url}/api/embed`, default model `nomic-embed-text` |
| `fastembed` | Local ONNX models; build with `--features fastembed` |

Settings: `embeddings.provider`, `embeddings.model`, `embeddings.base_url`.

When the active model changes, vectors of other models are flagged stale and
ignored by search. The "Re-embed all" button on `/settings` queues every
document again.

Other settings: `processing.auto_ocr`, `processing.auto_embed`,
`ai.openai_api_key`, `ai.anthropic_api_key` (empty values fall back to
`config.toml`) and `ai.request_timeout_secs` (default 300). A provider request
that takes longer than that fails the job with an error naming the provider;
the job then follows the normal retry policy. Raise it for slow CPU-only
vision models. Set pipeline defaults through `config.toml` or environment variables,
for example `FOLIO_OCR__PROVIDER=none` and `FOLIO_PROCESSING__AUTO_EMBED=false`.
Saved database values edited on `/settings` override startup configuration.

## Adding a provider

1. Create `crates/folio-infra/src/ai/ocr/<name>.rs` (or `embeddings/<name>.rs`)
   with a `build(&ProviderContext) -> Result<Arc<dyn OcrService>>` function
   (`EmbeddingService` for embeddings). For engines that read one image at a
   time, implement `PageEngine` and wrap it in `PageOcr`; PDF rendering is then
   handled for you.
2. Add one line to `builtin()` in the sibling `mod.rs`:
   `.register("<name>", <name>::build)`.

The id appears in the settings page automatically. Implement `health()`; the
"Test connection" button calls it. Embedding providers must return a stable
`model_id()` (convention: `<provider>:<model>`), because it is stored with every
vector.

## Vector storage

Vectors live behind the `SearchService` port. The SQLite adapter stores them as
blobs in `document_chunks` and scores them in process; other backends can
implement the port with a native vector index.
