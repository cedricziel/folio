# Scan text in review

In **Review a scan**, select **Read text** below a captured page. Edit or copy the
recognized text in the preview, or explicitly use its first line as the scan
title. The title remains editable in scan review. Closing the preview discards
its text edits. Cancel returns to scan review, where you can file manually while
Vision finishes stopping. Read another page by opening its own preview.

Folio sends the scan through the existing image-based PDF upload to the chosen
vault. Recognition runs locally. The chosen title uses the existing upload title
field. Preview text, table cells, dates, amounts and references stay out of the
upload. Server OCR and indexing continue to follow server settings.

## Evaluation

[Apple's request documentation](https://developer.apple.com/documentation/vision/recognizedocumentsrequest)
and [WWDC25 session](https://developer.apple.com/videos/play/wwdc2025/272/) describe
the document hierarchy, including transcripts, paragraphs and tables with cell
ranges. The probe inspects those results on four synthetic, disposable documents.
Run it on a Mac from the repository root:

```sh
probe_dir=$(mktemp -d)
xcrun swiftc -swift-version 6 -parse-as-library \
  apple/scripts/scan-recognition-probe.swift -o "$probe_dir/probe"
for fixture in letter receipt table poor; do
  /usr/bin/time -l "$probe_dir/probe" "$fixture"
done
rm -r "$probe_dir"
```

Measured on an M1 Pro with macOS 27.2.1, using the installed Xcode 27.0 SDK
and Swift 6.4. Each fixture ran three times in a separate process. RSS is the
process peak across all three requests, including the fixture and frameworks.

| Fixture | Vision request time | Peak RSS | Observed output |
| --- | --- | --- | --- |
| German letter, 1200 × 1600 | 52.64 s first run, 168–189 ms afterward | 108.5 MiB | All words, heading and reference were correct. Vision joined two source lines into one paragraph. Two dates offered no single filing date. |
| English receipt, 680 × 1300 | 220–372 ms | 92.0 MiB | All text was present, with all six item and amount cells in order. The heading was the merchant name. |
| English inventory, 1200 × 1600 | 225–306 ms | 109.4 MiB | All 12 table cells were in the expected rows and columns. The transcript put cells on separate lines. |
| Low-contrast, skewed German letter, 1200 × 1600 | 194–265 ms | 97.5 MiB | Vision kept the heading and body but added a vertical bar to the reference. |

The first request spent time waiting for Apple Neural Engine model compilation.
The request reported 33 language identifiers, including English and German.
The probe checked output quality in those two languages. Camera captures and iOS
latency, peak memory and cancellation still need device verification.

## Scope

Show the transcript for one page at a time on request. Keep the original capture
available for manual filing and let the user decide which text belongs in the
title. The letter and receipt results show why a document heading can help but
still needs context. Defer date and reference suggestions, structured table export,
Spotlight and transfer integration, and searchable PDF generation until each
has its own quality and persistence contract.

The scan review starts one request at a time. Closing the preview cancels its task
and drops any late result. The reading path limits the longest edge of the
recognition raster to 2,048 pixels and the retained transcript to 16 KiB of UTF-8.
Vision manages its own working memory and may take time to return after
cancellation. PDF rendering continues to use the captured page images.

For optional title suggestions (#203), `ScanTextRecognizer.recognize(_:)` returns a
plain Swift string for one upright `CGImage`. It returns an empty string for a
blank transcript and throws on failure or cancellation. A caller must set its own
model input budget and keep human acceptance explicit.
