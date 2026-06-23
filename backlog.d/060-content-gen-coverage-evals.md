# Content-type, coverage, and shape evals for generation

Priority: P2 · Status: pending · Estimate: M

## Goal

The generation bench can't tell a comprehensive verbatim-recitation set from six
conceptual trivia cards. It measures distractor quality and keep-rate (055/058),
not whether the cards **fit the content**. Dogfood 2026-06-23: the Apostles
Creed — a text to memorize line by line — produced 6 recognition MCQs ("God the
Father is creator of which two realms?") and the suite would happily green-light
it. Add eval dimensions that go **red** on today's output so they can drive the
generation fix (061). Per the eval-suite-growth principle, this dogfooded bug
becomes a deterministic eval.

## Oracle

- [ ] Fixtures: a verbatim/sequential text (a creed or short poem), an enumerable
      set (the NATO phonetic alphabet), and an existing prose-concept source as a
      regression guard against over-applying the new rules.
- [ ] **Classification eval:** creed → verbatim/sequential; NATO → enumerable
      set; prose → conceptual. Goes red on the current misclassification (creed
      treated as conceptual).
- [ ] **Coverage eval:** for memorize-the-whole-thing input, the card set covers
      every line/element in `[1..N]` with no gaps — creed → ≥1 card per line;
      NATO → all 26 letters. This **inverts** the conceptual "fewer, better"
      rule and must be explicit. Currently red.
- [ ] **Shape eval:** verbatim → cloze / next-line recall, not 4-option
      recognition trivia; set → production recall, not guess-from-4.
- [ ] **Directionality / anti-bloat eval:** for a paired-associate set, cards
      exist for the **non-derivable** direction only. NATO = letter→word
      (arbitrary, must memorize); word→letter is just the word's first letter
      (Bravo→B is free) and must **not** generate cards. Assert the redundant
      direction is absent.
- [ ] The suite is the comparison artifact and is **red on current generation**.
      Deterministic graders where possible (count, coverage `[1..N]`,
      direction); a model judge only for shape/classification, sized per the 058
      rigor doctrine (CI, judge ≠ generator family).

## Notes

Build this **before** 061 (verification system first): the eval proves the
creed/NATO fix instead of us eyeballing it. Connects to the graduated-difficulty
vision (recognition → recall → free recall by mastery). Keep grading
deterministic wherever the property is structural — coverage and directionality
are countable, not judgment calls.

## Children

1. Fixtures: creed/poem, NATO, prose-concept (with expected classification + N).
2. Classification grader.
3. Coverage `[1..N]` grader.
4. Card-shape grader (judge where needed).
5. Directionality / anti-bloat grader.
6. Wire into the bench receipt; confirm red on current output.
