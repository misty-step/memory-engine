---
id: 121
status: ready
priority: p1
type: feature
---

# Capture more opens create and always uses the model

## Outcome

From review, Capture more opens the create form. Create is one prompt and
one Create tap. The source always goes to the model.

## Why now

Overflow `Capture more` is `href="/"` (`render_escape_hatches`), so it dumps
to Home. Create still asks "Allow model help" vs "Keep local / Never send to
a model". Daily study always sends to the model.

## Acceptance

- [ ] Capture more navigates to `/app/create`, not Home.
- [ ] The capture form is title/body + Create. The permission select is gone
      from the PWA.
- [ ] Production capture is `model-eligible`. Local-only remains possible
      only as an explicit non-PWA/operator path if tests still need it.
- [ ] Creating still acknowledges immediately and generates in the
      background, with a clearer in-progress state than today's hint line.

## Dependencies

None. Pair with [122](122-edit-distractors.md) if cancel-from-edit also
needs a path back into review.

## Proof

From a live review card, Capture more lands on the create form. One capture
produces drafts without choosing a permission.

## Non-goals

No new capture types. No local-only consumer product path.
