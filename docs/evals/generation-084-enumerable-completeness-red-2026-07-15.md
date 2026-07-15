# Enumerable completeness eval receipt

Card `memory-engine-084` adds `014-us-presidents.json`, a 47-member ordinal
fixture (including Grover Cleveland at ordinals 22/24 and Donald J. Trump at
45/47). The fixture records ordinal identity, so repeated people in the
historical sequence are not mistaken for duplicate ordinal cards.

The fixture's current source basis is the White House presidential walk, which
describes 47 plaques and identifies the sequence from George Washington through
Donald J. Trump: <https://www.whitehouse.gov/walk-of-fame/>.

## Current red generation receipt

Command:

```text
cargo run -p memory-engine-bench -- generation --out /tmp/memory-engine-084-generation-red.md
```

Provider: `fixture/fake-model` · corpus: 14 sources · provider failures: 0.

| source | expected | observed | covered | missing | duplicate | invented | misassigned | reversed | order | direction | pass |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- | --- |
| us-presidents-ordinal | 47 | 6 | 0 | 47 | 0 | 0 | 0 | 6 | NO | NO | NO |

This is an honest generator red: the deterministic fake provider emits six
reverse-direction fact cards from this source, not 47 ordinal-to-member cards.
The eval instrument itself is green: it reports the failure without changing
generator policy, and its focused tests pass.

## Instrument verification

```text
cargo test -p memory-engine-bench generation:: -- --nocapture
34 passed; 0 failed; 23 filtered out

cargo test -p memory-engine-bench generation::enumerable -- --nocapture
6 passed; 0 failed; 51 filtered out

cargo test -p memory-engine-bench generation::tests::score_source_uses_production_gate_to_filter_duplicates_before_scoring -- --nocapture
1 passed; 0 failed; 56 filtered out
```

The focused scorer tests assert missing, duplicate, invented, ordinal
misassignment, reversed, and order-corrupt output. The fully correct 47-card
fixture passes even though Grover Cleveland and Donald J. Trump each occur at
two ordinals. The renderer integration test asserts that conceptual sources
without a content-fit score render `N/A` in each content-fit column. Conceptual
sources remain non-applicable rather than failing an enumerable-set check.
