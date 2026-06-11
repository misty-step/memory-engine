# Generation eval receipt

- Provider: openrouter/google/gemini-3.1-flash-lite (prompt-principled)
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 4 | 100% | 100% | 100% | 0% | yes | 100% | 642/515 | $0.0009 | 1503ms |
| nato-alphabet | enumerable-list | 5 | 100% | 100% | 100% | 0% | yes | 100% | 610/523 | $0.0009 | 953ms |
| http-caching | technical-doc | 4 | 100% | 100% | 100% | 0% | yes | 100% | 671/675 | $0.0012 | 879ms |
| rubicon | narrative-history | — | — | — | — | — | — | — | — | — | FAILED: The model's drafts could not be read; please try again. |
| sourdough | how-to | 5 | 100% | 100% | 100% | 0% | yes | 100% | 673/692 | $0.0012 | 926ms |
| gdpr-basis | regulatory | 5 | 100% | 100% | 80% | 0% | yes | 100% | 660/620 | $0.0011 | 983ms |
| hope-feathers | verbatim-verse | 4 | 100% | 100% | 100% | 0% | yes | 100% | 603/420 | $0.0008 | 1040ms |
| pythagorean | math-concept | 4 | 100% | 100% | 100% | 0% | yes | 67% | 660/540 | $0.0010 | 903ms |
| curie | biography | 4 | 100% | 100% | 100% | 0% | yes | 50% | 661/491 | $0.0009 | 1302ms |
| git-branching | product-doc | 5 | 100% | 100% | 100% | 0% | yes | 100% | 650/615 | $0.0011 | 5288ms |
| spacing-effect | long-science-prose | 5 | 100% | 100% | 80% | 0% | yes | 50% | 845/728 | $0.0013 | 1447ms |
| water-boiling | tiny-fact | 3 | 100% | 100% | 67% | 0% | yes | 100% | 576/233 | $0.0005 | 994ms |

## Totals

- Provider failures: 1/12 sources
- Mean provenance: 100% · mean answerability: 93% · mean key-term coverage: 88% · count-in-range: 11/11
- Total cost: $0.0109 · mean per source: $0.0010
- Latency p50: 994ms · p95: 5288ms
