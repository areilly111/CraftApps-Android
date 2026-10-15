# pdfcraft-audit

Layer L4 (may use `edit`): verification tooling, the faux-redaction audit.

```rust
let findings: Vec<Finding> = audit_redactions(&doc)?;  // page, kind, rect, covered_text
```

It reports two kinds of finding:

- **faux redaction**: an opaque filled rectangle painted *after* text it overlaps, dark enough
  against the text's colour (WCAG contrast below 3) to hide it, while the text is still
  extractable underneath;
- **unapplied mark**: a Redact annotation that was marked but never applied.

A properly applied redaction removes the text, so it never flags. Page backgrounds, table
shading and highlights don't either: they're painted before the text, or light against it.

Limits, by design and documented rather than hidden:

- only `re` rectangles closed by a fill count as covers (not arbitrary paths, shadings or
  patterns), and a translucent one (`/ca` below 1) is not flagged;
- covers inside form XObjects are found, but only text drawn directly in the page content is
  checked under them;
- annotation rectangles are compared in unrotated user space, so `/Rotate` pages may misalign.

It fails closed: an undecodable content stream, malformed `/Annots`, form XObjects nested deeper
than 32 or in a cycle, or a walk past the operator bound is an error, never "no findings".

Surfaces: the `redact_audit` automation tool and `pdfcraft-cli`.
