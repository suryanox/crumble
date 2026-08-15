# tradeoffs.md — why we did things this way

notes to self. not a spec, just the reasoning or someone reading code can learn.

---

## sqlparser vs writing our own parser
not writing a SQL parser. that's a solved problem, zero learning value for a DB
project specifically. using `sqlparser`, keep its AST as-is in crumble-sql. only
rule: AST types don't leak past lowering (lower.rs).

## why lower() is a free fn not a struct
no state to carry between calls. struct with zero fields is just ceremony.
if a schema/catalog needs to get threaded through later (for real type
checking), that's when it becomes struct.

## Scan/Filter/Project as separate tree nodes
mirrors how the query actually gets processed conceptually: get rows, keep
some, pick columns.

## logical IR vs physical IR? why two types that currently look identical
Scan(logical) -> SeqScan(physical) is the only physical strategy we have, so
right now they're basically the same shape. doesn't matter. the split exists
so that when IndexScan shows up, only `to_physical.rs` changes — optimizer and
everything upstream never finds out an index was used.

## constant fold before pushdown
pushdown (move Filter closer to Scan) is a no-op with one table. Filter is
already directly above Scan, nothing to push past. would've been writing code
against a tree shape that can't exist yet. wait for joins.

## fold returns Option not Result
a pass should never be able to fail. worst case: don't fold, leave the expr
alone. Result would imply optimization can break a valid query, which is wrong.

---

## Row is a struct not bare Vec<Value>
because MVCC. row will need a row-id + version fields at some point.
wrap it in one place Row instead of hunting down every Vec<Value> call
site later when that day comes.

## slot_count / free_space_offset live INSIDE the page bytes
first version had them as separate Rust struct fields next to a
[u8; PAGE_SIZE] — wrong, caught it myself. a page has to serialize to disk as
literally just those bytes. if the header lives outside the array, there's
nothing to write to disk that reconstructs slot_count after a restart. moved
both into bytes[0..4].

## why u16 for slot_count/free_space_offset, not usize
PAGE_SIZE is 4096, fits in u16 (max 65535). usize is 8 bytes on 64-bit,
u16 is 2. smaller header = more room for actual data. also usize would let
an offset be "bigger than the page," which is nonsense. the type itself
should rule that out.

## slotted page layout.. slots grow forward, rows grow backward
slot directory starts right after the header, grows toward the end of the
page as rows get added. row bytes get written starting from the END of the
page, growing backward. they meet somewhere in the middle/ that gap is free
space. lets you insert/delete without shifting existing row bytes around.

## heap file addressing: page N at byte offset N * PAGE_SIZE
no lookup table needed. pages never move once allocated, so it's just
arithmetic. simplest possible thing that works.

## sync_data() after every page write (later removed, see buffer pool)
fsync forces the OS to actually commit to disk instead of leaving it in page
cache. without it "written" doesn't mean "safe from a crash." this was the
correct-but-slow baseline before write-back existed.

---

## crumble-buffer is its own crate, not part of crumble-storage
tried to keep Page/PageStore inside crumble-storage and just have Table use
a BufferPool from a new crate. doesn't work — Table needs BufferPool
(storage -> buffer) but BufferPool needs Page (buffer -> storage). cycle.
cargo won't build it. moved Page/PageStore OUT of storage, into buffer.
one direction only now.

## write-through first, write-back after WAL existed
write-back (cache dirty, flush later) is only safe once something can
recover an unflushed write after a crash. that's literally the WAL's job.
built write-through first (correct, slow, syncs every write) as the honest
baseline, upgraded once WAL was there to lean on.

## LRU eviction MUST flush a dirty page before dropping it
this is the one line that's the entire difference between "cache" and
"silently deletes your data." easy to forget since dropping a HashMap entry
looks harmless.

---

## crumble-wal doesn't depend on crumble-storage
first version: WalRecord::Insert held a crumble_storage::Row directly.
broke same as the buffer pool — storage needs wal (Table uses WalWriter),
wal needed storage (for Row type). cycle again. fixed by logging raw
Vec<u8> instead. Table encodes Row->bytes itself before logging.
wal shouldn't know what a Row even is.

## WAL logs physical writes, not SQL statements
redo log, not statement log. record = "insert these exact bytes on this
exact page." replay is dumb — no re-parsing, re-planning, re-lowering SQL
on recovery. simpler, and matches what actually needs to be durable.

## length-prefix on every WAL record
without it: if a crash happens mid-write of a record, replay has no way to
know where a torn/half-written record ends and garbage starts. length
written first means replay can detect "not enough bytes here, stop" instead
of misreading garbage as the next record.

## fsync BEFORE append() returns, not after
this is the actual point of a WAL. append() only returns Ok once the write
is physically durable. whoever calls it (Table::insert) MUST wait for that
Ok before touching anything else — that ordering is what makes "log before
apply" mean anything.

## LSN = byte offset in the file before writing the record
free — file is append-only, so "how many bytes exist before this write" is
naturally unique and always increasing. no counter to maintain separately.

## page_lsn stamped in the page header — the annoying bug this fixes
buffer pool can flush a page on its own via LRU eviction, independent of any
explicit checkpoint. so by the time of a crash, SOME pages might already be
durably on disk even though their WAL records are still sitting in the log.
blind full replay would reinsert those rows AGAIN — duplicate data. fix:
stamp the LSN into the page itself on every write (same atomic disk write).
on replay, if page's stamped LSN >= record's LSN, skip — it's already there.
found this by literally asking "wait what if eviction flushes mid-crash" —
worth remembering to always ask that about anything that flushes
independently of the "planned" checkpoint path.

---

## CREATE TABLE stores column names only, no types
no type system exists. Value is decided by what actually gets inserted, not
declared ahead of time. `age INT` gets parsed but the INT part is thrown
away right now. real gap, not fixed yet.

## catalog.json uses serde_json, not bincode
different job than pages/WAL. tiny, written rarely (only on CREATE TABLE),
and being able to `cat` it while debugging is actually useful. not every
file needs to be binary just because pages are.

## catalog didn't persist schema at first — real bug I hit
created a table, inserted rows, restarted the REPL, SELECT said "table not
found" even though the .tbl/.wal files were sitting right there on disk.
Catalog::open just built an empty HashMap every time — never scanned for
existing tables. data was durable, the FACT that the table existed wasn't.
fixed by adding catalog.json (name -> columns) + reopening every known
table (each running its own WAL replay) on Catalog::open.

---

## DELETE uses a tombstone bit, not real byte removal
pages are append-only — insert_row only ever writes forward from the free
space boundary. nothing physically removes bytes. added a 5th byte per slot
(0=dead, 1=live). get_row checks it, returns None for dead slots. actual
space reclaim (compaction) — not built, deliberately, that's a separate
future problem.

## UPDATE = delete + insert, not in-place mutation
in-place would mean handling "new value is bigger than old value, doesn't
fit in the same slot" — real complexity. delete+insert reuses two paths that
are ALREADY proven crash-safe (full WAL/LSN coverage) for free. tradeoff:
costs 2 WAL records instead of 1 per update. fine for now.

## SET only takes literals, not expressions
`age = 41` works, `age = age + 1` doesn't yet. same reasoning as INSERT
VALUES only taking literals — real expression eval in SET is its own
feature, not squeezing it in here.

---

## indexing: secondary, not clustered — real reason, not just "easier"
clustered = rows physically stored in key order = insert has to find sorted
position + maybe split pages. that's not additive, it's a different Table
core entirely — and it would invalidate the WAL/LSN crash-recovery proof,
which was built and TESTED against "pages are an append-only heap." secondary
index = new structure that points at existing (page,slot) locations,
Table doesn't change at all. clustered stays on the roadmap as its own
future milestone, not something to sneak in as part of "adding an index."

## B+tree not plain B-tree
only leaves hold real data (key -> row location). internal nodes hold only
routing keys + child pointers, no data. standard shape — postgres/innodb/
sqlite all do this. not a simplification, it's the actual normal design.

## index keys: only Int and String for now
Float excluded — NaN breaks total ordering, can't cleanly impl Ord. Bool
excluded — 2 distinct values, indexing it is basically useless.

## index nodes reuse crumble_buffer::Page directly
didn't invent a new byte format. Page is already "container of variable-
length byte blobs with slot indirection" which is exactly what a B+tree node
needs. crumble-index depends on crumble-buffer only — NOT crumble-storage,
same cycle-avoidance as the buffer pool split.