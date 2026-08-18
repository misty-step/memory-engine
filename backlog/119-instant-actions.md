---
id: 119
status: ready
priority: p0
type: performance
---

# Instant keep, skip, snooze, and hatches

## Outcome

Keep, reject, skip, snooze, snooze-concept, reveal, reference, and bridge
acknowledge on tap. Skip and snooze leave a short confirmation of what
happened. The review surface does not go blank while the server works.

## Why now

2026-08-17 phone dogfood: Keep still feels like a load. Skip and snooze
succeed with no banner. Bridge leaves every overflow control enabled while
an indeterminate bar loops. Only `/app/submit` and `/app/next` use in-place
fetch (`app.js` `isInPlaceActionForm`). Draft keep/edit/reject post a full
HTML document.

## Acceptance

- [ ] Keep, reject, skip, snooze, snooze-concept, reveal, reference, and
      bridge use the same in-place fetch path as submit/next. JS-off still
      posts the form.
- [ ] Keep does not flash a full workspace reload. The pending draft leaves
      and the due count updates in place.
- [ ] Skip confirmation names that the card stays due later this session.
      Snooze confirmation names the delay. The message is transient, not a
      new page.
- [ ] While a hatch is in flight, sibling overflow controls disable. Bridge
      does not leave a looping bar with live buttons.
- [ ] Server still owns durable state. Pending labels never invent a grade.

## Dependencies

None.

## Proof

Phone keep of one draft and skip + snooze of one NATO card on
`https://scry.study`. Before/after timing on the same device.

## Non-goals

No auto-advance after grade. No new toast library. No automatic remediation
packs.
