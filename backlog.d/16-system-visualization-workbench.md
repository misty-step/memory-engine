---
shaping: true
ticket: 16-system-visualization-workbench
slice: 4
status: ready
priority: medium
estimate: M
depends_on: [15-service-interface-prototype]
oracles:
  - bun run ci
  - test -f docs/architecture/memory-engine.map.json
  - test -f docs/architecture/workbench.html
---

# System visualization workbench — architecture and workflow map

## Goal

Create a repo-local, versioned visualization workbench that makes the
memory-engine boundary easy to inspect: kernel surfaces, service commands,
workflow paths, state transitions, and extraction candidates.

## Non-Goals

- No changes to `src/` runtime behavior.
- No production web app, hosted service, auth, telemetry, or persistence.
- No new runtime dependency for the published `memory-engine` package.
- No attempt to auto-generate an authoritative architecture from code alone.
- No generic workflow editor or semantic orchestration DSL.

## Oracle

- [ ] `docs/architecture/memory-engine.map.json` defines the canonical
      visualization model: nodes, edges, view tags, source-file references, and
      stable ids.
- [ ] `docs/architecture/workbench.html` renders the JSON model as an
      interactive local artifact with at least these selectable views:
      architecture boundary, service command lifecycle, review state flow, and
      extraction decision map.
- [ ] The workbench links graph nodes back to source files, tests, or backlog
      tickets where useful.
- [ ] The README or architecture notes explain how to update the JSON model and
      what kinds of diagrams remain better represented as Mermaid, D2, or
      Structurizr.
- [ ] `bun run ci` exits 0.

## Notes

- Sequence this after ticket 15. The service command envelope should exist
  before the workbench tries to visualize it.
- Use the workbench as a decision aid for extraction, onboarding, and review,
  not as a second source of truth for runtime behavior.
- A thin JSON graph plus static HTML is the preferred first pass. It keeps the
  artifact diffable and avoids introducing a frontend build into the package.
- C4/Structurizr is a good fit for stable context/container/component views, but
  less expressive for interactive use-case and state exploration.
- D2 is useful for polished static or lightly interactive diagrams with
  tooltips/links, but it should not become the primary graph data model.
- Cytoscape.js or React Flow are plausible renderers for the static workbench.
  Prefer Cytoscape.js if graph filtering, clustering, and model-level
  navigation matter more than editable node-based UI.
- XState/Stately belongs only if the service prototype exposes real executable
  state machines or if model-based path tests become part of the acceptance
  story.

## Research

- C4 model: https://c4model.com/
- Structurizr: https://docs.structurizr.com/
- Structurizr file types: https://docs.structurizr.com/workspaces/file-types
- D2 interactive diagrams: https://d2lang.com/tour/interactive/
- Cytoscape.js: https://js.cytoscape.org/
- React Flow: https://reactflow.dev/learn/concepts/terms-and-definitions
- Stately Inspector: https://stately.ai/docs/inspector
- XState graph paths: https://stately.ai/docs/graph
- Madge: https://www.npmjs.com/package/madge
