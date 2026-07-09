# Architecture Workbench

Refs-Powder: memory-engine-016

This folder is the repo-local system visualization workbench for `memory-engine`.
It is intentionally static and diffable:

- `memory-engine.map.json` is the canonical graph model.
- `workbench.html` is a dependency-free local viewer for that model.

## Update Workflow

1. Edit `docs/architecture/memory-engine.map.json` directly.
2. Keep IDs stable:
   - never recycle an old `node.*` or `edge.*` id for a different meaning;
   - add new IDs for new concepts and remove IDs only when the concept is gone.
3. For every node/edge change, update all of:
   - `viewTags` for relevant views;
   - `summary` text that describes invariant intent;
   - `refs` pointing to concrete source/test/doc/Powder evidence.
4. Run a local parse/smoke check:
   - `python3 -m json.tool docs/architecture/memory-engine.map.json >/dev/null`
   - `test -f docs/architecture/workbench.html`
5. Open the viewer:
   - `cd docs/architecture && python3 -m http.server 4173`
   - visit `http://localhost:4173/workbench.html`

## Invariants

- The map is a decision aid, not runtime truth.
- Runtime behavior is still defined by the Rust crates and tests.
- No code generation, no workflow DSL, and no dependency additions for this artifact.

## When To Use Other Diagram Tools

Use this workbench when you need interactive, linkable, repo-versioned architecture views.

Use Mermaid when:

- you need a short sequence/state chart inline in Markdown;
- the diagram is small and mostly prose-adjacent.

Use D2 when:

- you need polished static diagrams for docs/reviews;
- you want concise text-first layout control and simple interactive links.

Use Structurizr (C4) when:

- you need stable context/container/component views across teams;
- architecture governance matters more than ad hoc flow exploration.
