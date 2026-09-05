# Crumble — Status & Features

🚧 work in progress. checked = built and tested, not just started.

## Parsing / IR

- [x] SQL parsing (via `sqlparser`, not hand-rolled — see [tradeoffs](../decisions/README.md))
- [x] AST -> Logical IR lowering
- [x] Logical IR -> Physical IR
- [x] constant folding (optimizer pass)
- [ ] predicate pushdown (needs joins first, currently a no-op)
- [ ] cost-based optimization

## Storage

- [x] slotted page format (disk-backed, byte-exact)
- [x] heap file storage (append-only)
- [x] buffer pool (LRU, write-back)
- [x] WAL + crash recovery (LSN-stamped pages, idempotent replay)
- [x] tombstone deletes (no physical byte removal)
- [ ] page compaction / space reclaim
- [x] NULL support (three-valued logic, nullable schema, NULL-aware indexing)
- [x] typed schema (CREATE TABLE columns have an enforced type: INT/BOOL/TEXT/FLOAT)

## Query Execution

- [x] SELECT (seq scan, filter, project)
- [x] SELECT * (wildcard expansion happens post-execution, reuses whatever columns the input already produced — works correctly through joins for free)
- [x] INSERT
- [x] UPDATE (delete + insert under the hood)
- [x] DELETE
- [x] CREATE TABLE
- [x] table aliases (JOIN and single-table, including mixed aliased/unaliased sides)
- [x] JOIN (INNER, LEFT, RIGHT, FULL OUTER — nested loop baseline; all four accelerated via index nested loop join when an index exists on the join column)
- [ ] index-accelerated RIGHT/FULL OUTER join (needs seen-set tracking on the indexed side, not just lookups)
- [ ] subqueries
- [ ] aggregates (COUNT/SUM/GROUP BY etc)
- [x] aggregates: COUNT, SUM, AVG, MIN, MAX, GROUP BY (linear-scan grouping, not hashed — Value::Float can't cleanly implement Hash/Eq for NaN reasons)
- [x] HAVING clause (including aggregates referenced only in HAVING, not in SELECT)
- [ ] SELECT-list/GROUP BY validation (ungrouped non-aggregated columns aren't rejected)
- [x] DROP TABLE / DROP INDEX (with IF EXISTS, DROP TABLE cascades to dependent indexes)

## Indexing

- [x] B+tree (leaf pages hold data, internal pages route only)
- [x] CREATE INDEX + backfill on existing rows
- [x] index kept in sync on INSERT/UPDATE/DELETE
- [x] index has its own WAL (crash-safe, same pattern as table storage)
- [x] optimizer rewrite: `WHERE col = literal` -> IndexScan when an index exists
- [ ] range scans through the index (WHERE col > x) — currently only exact equality
- [ ] clustered/index-organized storage (secondary index only right now, see [tradeoffs](../decisions/README.md))

## Transactions

- [x] MVCC (xmin/xmax row versioning, snapshot-based visibility)
- [x] row-level locking (block-and-wait, matches Postgres's real default behavior)
- [x] deadlock detection (wait-for chain cycle check, self-abort on detection)
- [x] BEGIN/COMMIT/ROLLBACK (multi-statement transactions, autocommit when no explicit BEGIN)
- [ ] isolation levels (currently always READ COMMITTED-equivalent behavior, no REPEATABLE READ/SERIALIZABLE)
- [x] concurrent access (proven via real multi-threaded tests, not just claimed)

## Not Started

- [ ] concurrency (single-threaded end to end right now)
- [ ] benchmarking

Crumble is being built incrementally. APIs, architecture, and implementation details will change as the project evolves and new database concepts are explored.