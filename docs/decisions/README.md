# tradeoffs — why i did things this way

notes to self. not a spec, just the reasoning so I don't forget it later.

---

## sqlparser instead of writing our own parser
not writing a SQL parser ourselves. that's a solved problem, zero learning
value for a DB project specifically. use sqlparser, keep its AST as is in
crumble-sql. one rule: AST types never leak past lowering. if crumble-ir or
crumble-exec ever import sqlparser::ast directly, that's a bug.

## lower() is a free function not a struct
no state to carry between calls. a struct with zero fields is just
ceremony. if a schema or catalog ever needs threading through, that's when
it becomes a real struct with a field. not before.

## Scan, Filter, Project as separate tree nodes
mirrors how a query actually gets processed: get rows, keep some, pick
columns. recursive tree, so Box is needed or the enum has no fixed size.

## logical IR and physical IR are separate types even though they look the same right now
Scan becomes SeqScan, the only physical strategy that exists at first, so
they're basically identical shapes. doesn't matter. the split is what lets
IndexScan get added later without touching the optimizer or anything
upstream of it.

## constant fold before predicate pushdown
pushdown moves Filter closer to Scan, but with one table Filter is already
directly above Scan — nothing to push past. would've been writing code
against a tree shape that couldn't exist yet. wait for joins.

## fold returns Option not Result
a pass should never be able to fail. worst case: don't fold, leave it
alone. Result would imply optimization can break a valid query, wrong.

## no Visitor trait, just match
match on an enum already is a visitor, compiler checked, exhaustive. only
worth a Fold trait once a second thing walks the same type the same way.
fold_plan walks LogicalPlan, execute walks PhysicalPlan — different types,
no real duplication, no trait needed yet.

---

## Row is a struct not a bare Vec of Value
because MVCC needs a row id and version fields eventually. wrap it now in
one place instead of hunting down every call site later.

## slot_count and free_space_offset live inside the page bytes, not next to them
first version had them as separate struct fields sitting beside a raw byte
array — wrong, caught it myself. a page has to serialize to disk as is
literally just those bytes. if the header lives outside the array there's
nothing on disk to reconstruct it from after a restart.

## u16 for page offsets, not usize
PAGE_SIZE is 4096, fits in two bytes. usize is eight bytes on a 64 bit
machine — smaller header, more room for actual data. also usize would let
an offset be bigger than the page, which is nonsense. the type should rule
that out on its own.

## slotted page layout, slots grow forward, rows grow backward
slot directory starts right after the header and grows toward the end of
the page. row bytes get written starting from the end, growing backward.
they meet in the middle — that gap is free space. lets you delete a row
without shifting every other row's bytes.

## HEADER_SIZE and SLOT_SIZE as named constants, not hardcoded numbers
paid off the moment the tombstone flag got added, slot size going from four
bytes to five. every offset calculation that referenced the constant just
worked. only had to touch the lines that needed the new byte.

## bincode pinned to exactly 2.0.1
real bincode is dead, the maintainer had a harassment situation and
stopped. version 3.0.0 on crates.io is not real code, it's a deliberate
tombstone that fails to compile on purpose so nobody accidentally depends
on abandoned code. 2.0.1 is the last real release, pinned exact since there
will never be a real 2.0.2.

## heap file addressing, page N at byte offset N times PAGE_SIZE
no lookup table needed. pages never move once written, so it's pure
arithmetic. simplest thing that works.

## truncate false when opening the page file
almost used truncate true, the common default. that wipes the file to zero
bytes on every open — would've deleted everything on every restart. one
flag, huge consequence, no warning if you get it wrong.

---

## crumble-buffer is its own crate, not folded into crumble-storage
tried keeping Page and PageStore inside storage and having Table pull in a
BufferPool from a new crate. doesn't compile — Table needs BufferPool so
storage depends on buffer, but BufferPool needs Page so buffer depends on
storage. a cycle. moved Page and PageStore out of storage entirely, into
buffer. one direction only now.

## write through first, write back only after WAL existed
write back means cache dirty pages, flush later. only safe once something
can recover an unflushed write after a crash — that's the WAL's whole job.
built write through first as the honest, slower, correct baseline, upgraded
once WAL was there to lean on.

## LRU eviction has to flush a dirty page before dropping it
this one line is the entire difference between a cache and something that
silently deletes your data. easy to forget since dropping a hashmap entry
looks completely harmless.

---

## crumble-wal does not depend on crumble-storage
first version logged a real Row directly. broke the same way the buffer
pool did — storage needs wal for Table to use WalWriter, wal needed storage
for the Row type. another cycle. fixed by logging raw bytes instead. Table
encodes Row to bytes itself before handing it to the log.

## WAL logs physical writes, not SQL statements
a redo log, not a statement log. a record says insert these exact bytes on
this exact page. replay is dumb on purpose — no re-parsing, no re-planning,
just mechanical reapplication.

## length prefix on every WAL record
without it, a crash mid write leaves a torn record and replay has no way to
tell where it ends and garbage begins. writing the length first means
replay can detect not enough bytes here, stop — instead of misreading
garbage as the next record.

## fsync before append returns, not after
this is the actual point of a WAL. append only returns once the write is
physically durable. whoever calls it has to wait for that before touching
anything else, or the ordering guarantee means nothing.

## LSN is just the byte offset in the file before the write
free, since the file only ever grows. no separate counter to maintain.

## page_lsn stamped into the page header, and why
buffer pool can flush a page on its own through eviction, independent of
any checkpoint. so by the time of a crash some pages might already be
durable while their WAL records are still sitting in the log. blind full
replay would reinsert those rows again. fix: stamp the LSN into the page
itself on the same write. on replay, if the page's stamped LSN is already
at or past a record's LSN, skip it. found this by asking what if eviction
flushes mid crash — worth remembering to ask that about anything that
flushes independently of the planned checkpoint path.

## the index gets its own WAL too, same discipline
BTree writes through a buffer pool exactly like Table does, so it needed
the same durability story or it would've been the one part of the system
that lied about being crash safe. difference from Table's WAL: every BTree
write replaces a whole page's contents (splits, rewritten leaves), never a
single row insert, so the record type is the raw new page bytes, not an
incremental row. everything else — length prefix, fsync before return, LSN
stamping, replay skipping already durable pages — is identical.

---

## catalog stores column names only, at first, no types
no type system existed yet. Value was decided by whatever got inserted, not
declared ahead of time. fixed later — see typed schema below.

## catalog.json uses serde_json, not bincode
different job than pages and WAL. small, written rarely, worth being
readable while debugging. not every file has to be binary just because
pages are.

## catalog didn't persist schema at first, a real bug I hit
created a table, inserted rows, restarted, select said table not found even
though the files were sitting right there on disk. Catalog::open just built
an empty map every time, never scanned for existing tables. the data was
durable, the fact that the table existed wasn't. fixed by adding
catalog.json and reopening every known table on startup.

---

## DELETE uses a tombstone bit, not real removal
pages are append only, nothing physically removes bytes. added a live flag
per slot. get_row checks it, skips dead slots. actual space reclaim is a
separate, unbuilt problem.

## UPDATE is delete plus insert, not in place mutation
in place would mean handling a new value that doesn't fit the old slot's
size — real complexity. delete plus insert reuses two paths that are
already proven crash safe, for free. cost: two WAL records instead of one
per update. acceptable.

## SET only takes literals, not expressions
age equals 41 works, age equals age plus 1 doesn't yet. same reasoning as
insert values only taking literals.

---

## indexing is secondary, not clustered, and that's a real reason not just easier
clustered means rows physically live in key order, which means insert has
to find a sorted position and maybe split pages — not additive, a different
Table core entirely, and it would invalidate the whole WAL and LSN crash
recovery proof already built and tested against an append only heap.
secondary index is just a structure pointing at existing locations, Table
never changes. clustered storage stays a real, separate, future milestone.

## B+tree, not a plain B-tree
only leaf pages hold real data, key to row location. internal pages hold
only routing keys and child pointers, no data. this is the standard shape,
postgres, innodb, sqlite all do this. not a simplification, the normal
design.

## index keys limited to Int and String at first, later Null too
Float excluded, NaN breaks total ordering, can't cleanly implement Ord.
Bool excluded, only two distinct values, not worth indexing.

## index nodes reuse crumble_buffer's Page directly
didn't invent a new byte format. Page is already a container of variable
length byte blobs with slot indirection, exactly what a tree node needs.
crumble-index depends on crumble-buffer only, not crumble-storage, same
cycle avoidance as the buffer pool split.

## leaf pages are linked, real B+tree range scans
without a pointer from one leaf to the next, answering something like age
greater than 30 means re descending from the root every time you cross a
leaf boundary. added a next_leaf pointer to every leaf's header, wired up
correctly on every split. find the starting leaf once, then just walk
forward. this is the actual textbook reason B+trees exist over plain
B-trees.

## index maintenance lives in the executor, not in Table, postgres style
Table stays completely ignorant that indexes exist, same as a heap access
method never knowing about pg_index. the executor calls insert, gets back
where the row landed, then separately checks if an index covers that column
and updates it. Table::insert had to start returning the page and slot it
used instead of throwing that away, purely so the executor could pass it on.

---

## typed schema, ColumnDef properly instead of a parallel type list
could have kept columns as a plain list of names and added a second parallel
list of types next to it. rejected — two lists that must always agree but
aren't enforced by the type system is exactly the bug class that already
bit this project more than once, page_count as both a field and a stale
method, a forgotten match arm, a shadowed variable. ColumnDef makes name and
type structurally one entry, can't drift apart. one time ripple cost across
call sites, beats a standing risk that compounds with every future change.

## crumble-ir and crumble-storage each define their own ColumnType, not shared
same reasoning as Literal versus Value already being two separate types.
translated at the exec boundary, same spot literal_to_value already lives.

## Float got added to ColumnType only after it broke a test
missed it the first time, only had Int, Bool, String. a test using a
Float column caught the gap immediately. good reminder that adding an enum
variant means checking every exhaustive match downstream, this has now
happened enough times in this project that it's basically expected.

## no fixed width int or float types like smallint or bigint
Value::Int is always i64, Value::Float always f64, no matter what SQL says.
real width support means new Value variants per width, rippling through
basically every crate, for a benefit that only matters once storage size is
actually being measured. silently aliasing smallint to the same i64 would
be worse than not supporting it, claims to respect a width while secretly
ignoring it. NULL was the more honest gap to close first.

---

## NULL is a real Value variant, not an Option wrapper or a sentinel
comparisons against a null operand always produce Null, not Bool, matching
real SQL, including the classic trap that col equals NULL always returns
zero rows — use IS NULL instead. AND and OR use real three valued truth
tables, false AND NULL is false, not NULL, because false already decides
the outcome no matter what the unknown side is.

## IndexKey::Null appended last in the enum on purpose
derived Ord orders by declaration position first, so putting Null last
makes every null sort after every real value for free, matching postgres.
that single ordering choice means IS NOT NULL reduces to a plain range scan
with an exclusive upper bound of Null, and IS NULL reduces to a plain
equality search for Null. no new tree operation needed for either, both
reuse already tested code.

## the planner refuses to rewrite col equals NULL into an index scan
even though NULL can be converted into an IndexKey now and could
technically be looked up, doing so would answer what's stored under the key
Null, which is a different question from is this unknown. rewriting it that
way would silently violate the three valued logic just built. always falls
through to Filter's null aware check instead.

---

## JOIN, inner only at first, nested loop, columns always qualified after a join
LEFT, RIGHT, FULL OUTER weren't built until NULL existed, since an outer
join is really just padding unmatched rows with NULL. nested loop, scan the
right side fully per left row, chosen as the honest correct baseline first,
same reasoning as SeqScan before IndexScan.

only one join supported, two tables, not chained three way joins.
qualifying columns as table dot column only works cleanly when both join
sides are plain scans. a three way join's outer join has another join as
its left side, which needs real column provenance tracking through nested
joins. solvable, just its own separate increment, not done.

column ambiguity solved by always qualifying every column after a join,
users.id, orders.id, instead of building real SQL name resolution where an
unqualified name has to resolve unambiguously or error. sidesteps a whole
subsystem on purpose.

table aliases needed their own real fix. TableFactor::Table carries an
alias field that was originally just thrown away. fixed by tracking two
separate strings per side of a join, the real table name for actually
reading data, and a qualifier, the alias if one exists otherwise the real
name, used everywhere columns get labeled. Scan always uses the real name,
Join's left_table and right_table fields always use the qualifier. the
whole thing composes correctly with zero new lookup logic anywhere else,
because a qualified column like u.id just lowers to a plain string with a
dot in it, and RowSet gets built using that same string. as long as both
sides agree on which string to use, it just matches.

## join stayed unaccelerated by any index for a while, a real gap, later closed
the optimizer rewrite that turns Filter over SeqScan into IndexScan never
looked inside a Join at all, confirmed by testing it directly, an index on
the join column sat there completely unused. fixed with index nested loop
join: for each row on the probing side, look the join key up in the index
instead of scanning the whole other table. O(n log m) instead of O(n
times m).

started scoped to INNER and LEFT only, because an index lookup only tells
you found or not found per probe, never which keys were never probed at
all, which is exactly what detecting unmatched rows on the indexed side for
RIGHT or FULL OUTER needs. fixed properly rather than left as a gap: track
every matched page and slot on the right side in a set during the probing
loop, then, only when the join kind actually needs it, one full scan of the
right table afterward to find whatever wasn't in the set. still O(n log m)
for the probing plus O(m) once for the unmatched check, not O(n times m),
so it stays a real win even for RIGHT and FULL OUTER now.

right_table had to split into two fields for this, same real name versus
qualifier split as aliasing, because the rewrite needs the real table name
to actually query the catalog and the index, but the qualifier for naming
output columns, and by the time the rewrite runs those two things already
live in different places in the plan tree.

## MVCC — row versioning through the locking door, not the visibility door
went straight to row-level locking per your choice, skipping coarse-grained
entirely. turns out row-level LOCKS in real MVCC are implemented THROUGH row
versioning (xmax being set = the lock), not separately from it — so building
locks meant building xmin/xmax/transaction-id infrastructure anyway. same
foundation either way, just entered from a different side.

physical lock (Mutex<Table>, brief, per-operation) is a completely different
thing from the logical/transactional row lock (xmax + wait_for, held for a
whole transaction) — both needed, not alternatives. real bug caught by
writing the actual concurrent test: delete_at originally held the physical
mutex THROUGH the wait_for block, meaning one transaction waiting on another
blocked every other thread from touching the table at all — the coarse
lock we explicitly rejected, sneaking back in through a different door.
fixed by making delete_at a free function operating on Arc<Mutex<Table>>
directly instead of a method on an already-locked &mut Table, so it can
drop the physical lock before blocking and reacquire after.

## delete_at initially tombstoned the row physically — broke MVCC visibility
first version called page.delete_row(slot) (the tombstone bit) the instant
xmax got set. get_row skips dead slots, so the row became invisible to
EVERYONE immediately, including the next transaction trying to check who
holds it. found this via the concurrent test itself, not by inspection —
both block-and-wait tests failed with RowNotFound instead of the expected
conflict/success. real fix: xmax became a plain u64 sentinel (0 = unclaimed)
instead of Option<u64>, guaranteeing fixed serialized length, which makes
true in-place page overwrite safe (added Page::update_row for exactly this,
strict same-length-only). row stays fully readable with its real xmax the
whole time now, matching how postgres tuple headers actually work.

## deadlock detection — wait-for chain, not a general graph
delete_at only ever waits on ONE other transaction at a time, so the whole
wait-for structure is just chains, not an arbitrary graph — detecting a
cycle is just walking the chain from what you're about to wait on and
checking if it leads back to yourself. cycle check and edge insertion
happen under one unbroken lock hold, not two separate ones — otherwise two
threads could each see "no cycle yet" and insert edges that together form
one anyway.

## BEGIN/COMMIT/ROLLBACK kept sqlparser types out of main.rs
crumble-sql exposes a small transaction_control() helper instead of main.rs
matching on sqlparser::ast::Statement directly — same "AST types never
leak past lowering" rule, just extended to this one REPL-level check that
happens before lowering even starts.

## SELECT * expands AFTER execution, not at lowering time
lowering has no Catalog access on purpose (kept decoupled). Project already
gets the fully materialized RowSet before picking columns, and that RowSet
already knows its own columns — so Projection::All just reuses whatever's
already there instead of needing a schema lookup. free bonus: works
correctly through joins with zero extra code, since a join's RowSet is
already qualified (users.id, orders.total, ...) by the time Project sees it.

## fixed a real rollback/index consistency bug — eager index deletes were wrong
DELETE and the old-half of UPDATE used to call index.delete() immediately at
execution time. if that transaction later rolled back, the row itself
correctly became visible again (xmax reverts), but the index entry was
already gone forever — nothing re-added it. SeqScan and IndexScan could give
different answers for the same rolled-back DELETE. turns out the fix wasn't
adding an undo mechanism, it was removing code: postgres itself never
physically removes an index entry at DELETE time either, only VACUUM does,
much later, once no transaction could possibly still need the old version.
index reads stay correct in the meantime purely because they go through
row_at + is_visible, which is already the single source of truth for
whether a row counts. stopped calling index.delete() in both places — a
deleted/updated-away row's old index entry just becomes the same kind of
harmless garbage an aborted INSERT's entry already was, cleaned up later by
compaction (already a known, deferred gap).


## aggregates — linear scan grouping, not a hashmap
Value::Float can't cleanly implement Hash (NaN breaks the Eq it needs too) —
same reason Float was already excluded from IndexKey. rather than exclude
Float from GROUP BY entirely (a real, if unusual, SQL case), grouping does
a linear Vec scan instead of a HashMap — O(n * groups) instead of O(n), but
correct for every type. worth hashing properly later if grouping ever
becomes a hot path, not urgent now.

## no GROUP BY still means one implicit group, even over zero rows
SELECT COUNT(*) FROM empty_table must return one row (count=0), not zero
rows. handled as a special case: no GROUP BY + zero groups formed from the
input -> push one empty-key group before computing aggregates.

## SUM/AVG/MIN/MAX return NULL over an all-NULL or empty group, not 0
matches real SQL, same "surprising but correct" bucket as col = NULL always
being false. COUNT(*) counts every row including NULLs; COUNT(col) only
counts non-null values of that column — different rule for COUNT
specifically vs the other four.

## Aggregate produces a wide row, Project narrows it — no new projection logic
Aggregate always outputs every GROUP BY column plus every aggregate result,
regardless of SELECT list order. the existing Project node (unchanged) sits
on top and picks/reorders whatever the SELECT list actually asked for —
same Filter-then-Project composition pattern, Aggregate just slots in as
another thing Project can sit on.

## no validation that SELECT list columns are grouped or aggregated
real SQL rejects `SELECT name, SUM(age) FROM users GROUP BY city` (name is
ambiguous per group). we don't enforce this — trust valid SQL. named gap,
not silently wrong, just unchecked.

## HAVING supports aggregates not in the SELECT list too
went with the fuller scope after pushback — HAVING SUM(age) > 100 works even
without SUM(age) in the SELECT list. implemented as a small expr-tree walk
(collect_having_aggregates) that finds function calls not already computed,
injects them into Aggregate's list (so they get computed) without adding
them to output_columns (so they don't leak into results). HAVING itself
lowers to a plain Filter sitting on top of Aggregate — zero exec-layer
changes needed, since Filter's eval_expr already works generically against
any input's named columns, and Aggregate's RowSet already exposes group_by
columns + every aggregate's alias by name. same Filter-then-Project
composability the whole IR has leaned on all along.

## real bug found: test helper discarded its own seeding transaction id
half the aggregate/HAVING tests did `let (_dir, catalog, ..) = seeded_pets_catalog()`
then `catalog.tx_manager.begin()` — a FRESH xid, different from the one
that actually inserted the seed rows (which was never committed). is_visible
correctly hid all the seed data from that fresh reader. some tests still
"passed" by accident (SUM over invisible data returns NULL, which coincided
with the expected "empty group" answer; a loop with no row-count assertion
silently skips when there are zero rows). real lesson: a test with only
conditional per-row assertions inside a for-loop, no total count check, will
silently pass over zero rows — worth always asserting row count explicitly,
not just per-row content.

## found a real, serious bug just by walking through the architecture out loud
TransactionManager's commit/abort status only ever lived in memory. every
restart created a brand new empty manager. is_visible checks a transaction's
LIVE status — unknown xid means "not committed" means invisible. so every
row with a real (non-zero) xmin became permanently invisible after ANY
restart, forever. confirmed with a real test: insert+commit in one process,
quit, start fresh, the row is gone. this is exactly what postgres's CLOG
(pg_xact) exists to prevent — a durable record of every transaction's fate,
specifically so it survives a crash/restart.

fixed with a small, independent append-only log in crumble-tx itself (same
pattern as the page WAL — length-prefixed records, fsync before returning —
but NOT sharing crumble-wal's actual code, since WalRecord's shape is
table/index-specific and bolting transaction-log entries onto it would force
Table/BTree's replay to handle irrelevant variants).

logs begin() too, not just commit/abort — needed for two things: (1) knowing
the highest xid ever used, so the counter can resume past it instead of
resetting to 1 and risking a NEW transaction colliding with an OLD row's
xmin from a previous run, and (2) the actual crash-recovery rule: any
transaction found still "InProgress" after replaying the whole log never
got a chance to finish — whatever process owned it is gone, so it's treated
as aborted. matches postgres's real convention exactly.

scoped begin/commit/abort's log-write failures to panic rather than
propagating Result — threading Result through every call site across the
whole codebase for this would have been a huge ripple on top of an already
large fix. named explicitly, not hidden.

old data created before this fix has no log entries for its transactions at
all — after the fix, that data's xids get treated as "no record = aborted",
same as the crash-recovery rule. not a regression: that data was ALREADY
invisible (that's the bug), just confirms it stays that way rather than
silently reappearing wrong.