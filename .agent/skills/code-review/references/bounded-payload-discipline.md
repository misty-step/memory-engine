# Bounded-Payload Discipline

Any response that advertises a bounded collection, pagination window, top-N
list, truncation flag, or true total must push the relevant bound into the data
query or split the total from the bounded fetch; it must not load an unbounded
set and trim it in memory.

Use this reference when a local `review-patterns.md` entry points at capped
payloads, preloads, nested includes, relation lists, pagination, top-N read
models, summary totals, or `*_truncated` fields. The examples name Ecto,
Prisma, and Drizzle because they are common Spellbook consumer stacks, but the
rule is the same for any ORM or query builder.

## Antipattern

```text
fetch every related row -> slice/take in memory -> return "top N"
```

This shape keeps the API payload small while letting database work, network
transfer, memory, and latency grow with the uncapped row count. It also tempts
summary code to compute `total_count` from the bounded list instead of the true
matching set.

## Decision Tree

1. Does the caller only need the bounded list?
   Use an in-query limit/order/cursor and return only those rows.
2. Does the caller also need a true total, `has_more`, or `truncated` flag?
   Use a separate count or existence query plus a bounded fetch.
3. Is the cap per parent relation, not just a top-level page?
   Use the ORM's nested relation limit if it supports one; otherwise issue a
   dedicated bounded query per explicit parent set or a ranked SQL query.
4. Is the collection already known to be tiny by schema or invariant?
   Document that invariant near the read model. Do not rely on current
   production size as the bound.

Memory slicing is acceptable only after the data layer has already enforced the
candidate set's upper bound. It is not the mechanism that makes an unbounded
read safe.

## Shape A: In-Query Bound

Use this when the response only needs the returned rows and does not need a
true total for the full matching set.

### Elixir + Ecto

Prefer a query that limits the associated rows before loading them.

```elixir
signal_query =
  from s in IncidentSignal,
    where: s.incident_id == ^incident_id,
    order_by: [desc: s.inserted_at],
    limit: ^max_signals

signals = Repo.all(signal_query)
```

For association-shaped code, Ecto documents both query preloads and loading an
association query with `Ecto.assoc/3`; keep the `limit` on that query, not after
`Repo.preload/2` returns every child row.

```elixir
signals =
  incident
  |> Ecto.assoc(:signals)
  |> order_by([s], desc: s.inserted_at)
  |> limit(^max_signals)
  |> Repo.all()
```

### TypeScript + Prisma

Put `take` and a deterministic `orderBy` in the Prisma query. Prisma documents
`skip`/`take` for pagination and relation queries support nested filtering and
ordering; the bound belongs in the query object, not after the promise resolves.

```typescript
const signals = await prisma.incidentSignal.findMany({
  where: { incidentId },
  orderBy: { insertedAt: "desc" },
  take: maxSignals,
});
```

For nested reads, bound the relation at the include/select site when that is
the shape the API needs.

```typescript
const incident = await prisma.incident.findUnique({
  where: { id: incidentId },
  include: {
    signals: {
      orderBy: { insertedAt: "desc" },
      take: maxSignals,
    },
  },
});
```

### TypeScript + Drizzle

Drizzle's query APIs expose `limit`/`offset` at the top level and for nested
relations. Use those instead of calling `.slice(0, max)` on a full result.

```typescript
const signals = await db.query.incidentSignals.findMany({
  where: eq(incidentSignals.incidentId, incidentId),
  orderBy: desc(incidentSignals.insertedAt),
  limit: maxSignals,
});
```

## Shape B: True Total Plus Bounded Fetch

Use this when the response needs both a bounded list and a true total,
`has_more`, or `truncated` flag. Two cheap queries are better than one
unbounded query.

```text
total_count = count matching rows
items = fetch matching rows ordered with limit max + 1 when has_more is needed
return items[0:max], total_count, has_more/truncated
```

Rule of thumb:

- Use `count + limit` when the contract exposes `total_count`.
- Use `limit max + 1` when the contract only needs `has_more` or `truncated`.
- Use cursor pagination when users page through a growing feed or timeline.
- Use offset pagination only for shallow page jumps where the database cost is
  acceptable.

### Elixir + Ecto

```elixir
base =
  from s in IncidentSignal,
    where: s.incident_id == ^incident_id

total = Repo.aggregate(base, :count)

signals =
  base
  |> order_by([s], desc: s.inserted_at)
  |> limit(^max_signals)
  |> Repo.all()

%{signals: signals, total_count: total, truncated: total > length(signals)}
```

### TypeScript + Prisma

```typescript
const where = { incidentId };

const [total, signals] = await prisma.$transaction([
  prisma.incidentSignal.count({ where }),
  prisma.incidentSignal.findMany({
    where,
    orderBy: { insertedAt: "desc" },
    take: maxSignals,
  }),
]);

return {
  signals,
  totalCount: total,
  truncated: total > signals.length,
};
```

### TypeScript + Drizzle

```typescript
const totalRows = await db
  .select({ count: count() })
  .from(incidentSignals)
  .where(eq(incidentSignals.incidentId, incidentId));

const signals = await db.query.incidentSignals.findMany({
  where: eq(incidentSignals.incidentId, incidentId),
  orderBy: desc(incidentSignals.insertedAt),
  limit: maxSignals,
});
```

## Telemetry Test Pattern

Bounded read models should have one behavior test that proves database work is
constant in row count. The exact API differs by stack; the shape is stable:

1. Attach a query observer to the ORM, adapter, or database client.
2. Create fixtures at small and larger cardinalities, such as 5, 50, and 500
   children for the same parent shape.
3. Call the read model with the same cap.
4. Assert that query count stays constant and returned rows never exceed the
   cap.
5. If the response exposes a total or truncation flag, assert it reflects the
   full fixture cardinality, not the returned list length.

Example assertions:

```text
rows_returned <= max
query_count(N=5) == query_count(N=50) == query_count(N=500)
total_count == N when the contract exposes a true total
truncated == (N > max)
```

This test catches both failure modes: N+1 query growth and unbounded reads that
only look safe because the final JSON payload is capped.

## Enforcement

- **Review pattern:** link this file from the repo's local `P-NN` entry for
  capped payloads or preload-then-take findings.
- **Static check:** add a per-repo lint when the stack exposes a reliable
  syntax signal, such as an Ecto preload followed by `Enum.take/2`.
- **Runtime test:** attach query telemetry or database-client query logging and
  assert constant query count over scaled fixtures.
- **Contract test:** when a response has `total_count`, `has_more`, or
  `truncated`, assert those fields against the full fixture set.

## Official References

- Ecto association and preload docs:
  https://hexdocs.pm/ecto/Ecto.html
- Prisma pagination docs:
  https://www.prisma.io/docs/orm/prisma-client/queries/pagination
- Prisma relation queries docs:
  https://www.prisma.io/docs/orm/prisma-client/queries/relation-queries
- Drizzle select, limit, and pagination docs:
  https://orm.drizzle.team/docs/select
- Drizzle relational query limit docs:
  https://orm.drizzle.team/docs/rqb-v2
