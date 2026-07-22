# Generation 061 live Mint receipt

- Observed: 2026-07-21
- Transport: Mint egress proxy to OpenRouter (runtime base URL omitted)
- Authorization: runtime Mint credential alias (value omitted)
- Secret handling: no credential bytes or raw model response text retained in this receipt
- Evidence: nonzero provider usage and cost for all 14 model-backed source calls; deterministic post-processing and receipt scoring ran on their returned candidates
- Production residual: this run exercised the bench harness through the runtime beta generation runner; the deployed Scry capture path, rendered learner review, grading, and phone review remain unexercised and unproved

# Generation eval receipt

- Provider: openrouter/google/gemini-3.5-flash (prompt-principled)
- Corpus: 14 sources

| source | category | accepted | rejected | failures | runtime | provenance | answerable | dup | count-ok | terms | shape | content-kind | content-cover | content-shape | direction | variants | cohesion | self-ref | tokens in/out | cost | latency |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| mitochondria | science-prose | 4 | 1 | 0 | 80% | 100% | 100% | 0% | yes | 100% | NO | NO | 0/0 (100%) | NO | yes | 0% | 100% | 100% | 3190/3097 | $0.0327 | 18715ms |
| nato-alphabet | enumerable-list | 26 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | yes | 26/26 (100%) | yes | yes | 0% | 100% | 100% | 1572/5056 | $0.0479 | 22839ms |
| http-caching | technical-doc | 7 | 2 | 0 | 78% | 100% | 100% | 0% | yes | 100% | yes | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 3303/7040 | $0.0683 | 37841ms |
| rubicon | narrative-history | 3 | 4 | 0 | 43% | 100% | 100% | 0% | yes | 75% | yes | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 3250/3601 | $0.0373 | 17843ms |
| sourdough | how-to | 6 | 1 | 0 | 86% | 100% | 100% | 0% | yes | 67% | NO | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 3268/4152 | $0.0423 | 20004ms |
| gdpr-basis | regulatory | 5 | 1 | 0 | 83% | 100% | 100% | 0% | yes | 100% | yes | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 3228/3083 | $0.0326 | 17741ms |
| hope-feathers | verbatim-verse | 8 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 1489/2657 | $0.0261 | 18199ms |
| pythagorean | math-concept | 4 | 3 | 0 | 57% | 100% | 100% | 0% | yes | 33% | yes | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 3366/5634 | $0.0558 | 29190ms |
| curie | biography | 7 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 1547/2596 | $0.0257 | 12139ms |
| git-branching | product-doc | 6 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 75% | yes | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 1536/2689 | $0.0265 | 12899ms |
| spacing-effect | long-science-prose | 4 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 75% | yes | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 1731/1951 | $0.0202 | 10499ms |
| water-boiling | tiny-fact | 3 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 1462/1421 | $0.0150 | 7872ms |
| apostles-creed | verbatim-sequential | 6 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | yes | yes | 6/6 (100%) | yes | yes | 0% | 100% | 100% | 1497/2621 | $0.0258 | 12547ms |
| us-presidents-ordinal | enumerable-ordinal | 47 | 0 | 0 | 100% | 100% | 100% | 0% | yes | 100% | NO | N/A | N/A | N/A | N/A | 0% | 100% | 100% | 2284/11394 | $0.1060 | 45376ms |

## Enumerable-set completeness

| source | expected | observed | covered | missing | duplicate | invented | misassigned | reversed | order | direction | pass |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- | --- |
| us-presidents-ordinal | 47 | 47 | 47 | 0 | 0 | 0 | 0 | 0 | yes | yes | yes |

## Totals

- Provider failures: 0/14 sources
- Mean provenance: 100% · mean answerability: 100% · mean key-term coverage: 88% · count-in-range: 14/14
- Intent shape matches: 10/14 sources
- Content fit matches: 2/3 sources · mean required-unit coverage 100%
- Bridge fixture: easier 100% · faithful 100% · duplicate 0% · pass
- Total cost: $0.5620 · mean per source: $0.0401
- Latency p50: 17843ms · p95: 45376ms
