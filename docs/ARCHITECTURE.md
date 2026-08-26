# ghlinks architecture diagrams

The canonical diagram sources are the `.mmd` files in this folder, not
this document. This file is a short guide to what each diagram shows and
how to render them to SVG/PNG/PDF — it does not duplicate their content.
Edit the `.mmd` files when the architecture changes; there is nothing here
that needs to change in step with them.

## Diagrams

| File | Shows |
|---|---|
| [`architecture.mmd`](architecture.mmd) | Module-level data flow: input file → `classify.rs` → `github.rs` / `discovery.rs` → `model.rs` → `main.rs` → `report.json`. |
| [`pipeline.mmd`](pipeline.mmd) | The same flow with concurrency made explicit — per-link tasks spawned via `futures::stream`, fanning out to GitHub and Hacker News collection in parallel before results are assembled. |
| [`sequence.mmd`](sequence.mmd) | A single end-to-end run as a sequence diagram: CLI invocation through to `report.json` being written, including the optional (`--skip-external`) discovery branch. |

There is deliberately no class diagram in this set. A hand-maintained
class diagram tends to drift from the actual struct/enum definitions
faster than anything else in a doc set — see `model.rs`, `github.rs`, and
`classify.rs` directly for the authoritative shape of `RepoData`,
`GistData`, `LinkKind`, and related types; their own doc-comments explain
the non-obvious parts (e.g. why `LinkKind` has more than four variants,
why `github.rs` is GraphQL-first).

## Rendering

Each `.mmd` file renders independently with [`mermaid-cli`](https://github.com/mermaid-js/mermaid-cli)
(`mmdc`):

```bash
npm install -g @mermaid-js/mermaid-cli

mmdc -i architecture.mmd -o architecture.svg
mmdc -i architecture.mmd -o architecture.png

mmdc -i pipeline.mmd -o pipeline.svg
mmdc -i sequence.mmd -o sequence.svg
```

For a PDF, render to SVG first and convert (`mmdc` doesn't emit PDF
directly):

```bash
mmdc -i architecture.mmd -o architecture.svg
# then, e.g. via a converter you already have (rsvg-convert, Inkscape, etc.)
rsvg-convert -f pdf -o architecture.pdf architecture.svg
```

GitHub's own Markdown renderer also renders fenced ` ```mermaid ` code
blocks natively, so pasting a diagram's contents into a fenced block in
any Markdown file (README, a PR description, an issue) works without
running `mmdc` at all — useful for one-off viewing, not a substitute for
keeping the `.mmd` files themselves current.
