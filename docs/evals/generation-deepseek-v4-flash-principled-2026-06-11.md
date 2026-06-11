# Generation eval receipt

- Provider: openrouter/deepseek/deepseek-v4-flash (prompt-principled)
- Corpus: 12 sources

| source | category | drafts | schema | provenance | answerable | dup | count-ok | terms | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 4 | 100% | 100% | 100% | 0% | yes | 100% | 359/470 | $0.0001 | 1125ms |
| nato-alphabet | enumerable-list | 8 | 100% | 100% | 100% | 0% | yes | 100% | 330/627 | $0.0002 | 194ms |
| http-caching | technical-doc | 7 | 100% | 100% | 100% | 0% | yes | 100% | 389/3326 | $0.0009 | 748ms |
| rubicon | narrative-history | 7 | 100% | 100% | 100% | 0% | yes | 75% | 373/746 | $0.0003 | 334ms |
| sourdough | how-to | 7 | 100% | 100% | 100% | 0% | yes | 100% | 394/839 | $0.0003 | 518ms |
| gdpr-basis | regulatory | 6 | 100% | 100% | 100% | 0% | yes | 100% | 376/4851 | $0.0018 | 1532ms |
| hope-feathers | verbatim-verse | 7 | 100% | 100% | 100% | 0% | NO | 67% | 315/652 | $0.0002 | 912ms |
| pythagorean | math-concept | 6 | 100% | 100% | 83% | 0% | yes | 100% | 373/753 | $0.0003 | 778ms |
| curie | biography | 7 | 100% | 100% | 100% | 0% | yes | 100% | 377/802 | $0.0002 | 183ms |
| git-branching | product-doc | 6 | 100% | 100% | 100% | 0% | yes | 100% | 366/667 | $0.0002 | 874ms |
| spacing-effect | long-science-prose | — | — | — | — | — | — | — | — | — | FAILED: The model provider's response could not be read. |
| water-boiling | tiny-fact | 3 | 100% | 100% | 100% | 0% | yes | 100% | 289/299 | $0.0001 | 3143ms |

## Totals

- Provider failures: 1/12 sources
- Mean provenance: 100% · mean answerability: 98% · mean key-term coverage: 95% · count-in-range: 10/11
- Total cost: $0.0046 · mean per source: $0.0004
- Latency p50: 778ms · p95: 3143ms
