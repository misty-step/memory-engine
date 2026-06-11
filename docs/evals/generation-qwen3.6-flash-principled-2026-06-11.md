# Generation eval receipt

- Provider: openrouter/qwen/qwen3.6-flash (prompt-principled)
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | — | — | — | — | — | — | — | — | — | FAILED: The model's drafts could not be read; please try again. |
| nato-alphabet | enumerable-list | — | — | — | — | — | — | — | — | — | FAILED: The model's drafts could not be read; please try again. |
| http-caching | technical-doc | — | — | — | — | — | — | — | — | — | FAILED: The model's drafts could not be read; please try again. |
| rubicon | narrative-history | — | — | — | — | — | — | — | — | — | FAILED: The model's drafts could not be read; please try again. |
| sourdough | how-to | 0 | 100% | 0% | 0% | 0% | NO | 0% | 418/2699 | $0.0031 | 698ms |
| gdpr-basis | regulatory | — | — | — | — | — | — | — | — | — | FAILED: The model's drafts could not be read; please try again. |
| hope-feathers | verbatim-verse | 7 | 100% | 100% | 100% | 0% | NO | 67% | 340/3855 | $0.0044 | 810ms |
| pythagorean | math-concept | — | — | — | — | — | — | — | — | — | FAILED: The model's drafts could not be read; please try again. |
| curie | biography | — | — | — | — | — | — | — | — | — | FAILED: The model's drafts could not be read; please try again. |
| git-branching | product-doc | 0 | 100% | 0% | 0% | 0% | NO | 0% | 383/2932 | $0.0034 | 675ms |
| spacing-effect | long-science-prose | 0 | 100% | 0% | 0% | 0% | NO | 0% | 585/2628 | $0.0031 | 665ms |
| water-boiling | tiny-fact | — | — | — | — | — | — | — | — | — | FAILED: The model's drafts could not be read; please try again. |

## Totals

- Provider failures: 8/12 sources
- Mean provenance: 25% · mean answerability: 25% · mean key-term coverage: 17% · count-in-range: 0/4
- Total cost: $0.0140 · mean per source: $0.0035
- Latency p50: 675ms · p95: 810ms
