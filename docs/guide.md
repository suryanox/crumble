# guide.md — what the pieces actually are

quick definitions, not a tutorial. if you already know what a B-tree is,
skip most of this — it's here for when I (or someone else) forgets.

## page
fixed-size chunk of bytes, PAGE_SIZE = 4096. the unit everything moves in —
disk reads/writes, buffer pool caching, B+tree nodes. never a partial page.
4096 because it's the classic OS page size (matches how most filesystems
and disks actually do I/O under the hood) — not a magic number, just the
standard default.

## slot
one row's location inside a page: `{ offset, length, live }`. pages don't
store rows in a flat list — they store a small directory of slots up front,
and the actual row bytes wherever there's free space. lets you delete a row
(flip `live` to false) without shifting every other row's bytes around.

## slotted page layout
slot directory grows forward from right after the header. row bytes grow
backward from the end of the page. they meet somewhere in the middle — the
gap between them is free space. insert = write bytes from the back, add a
slot at the front. this is the standard layout basically every disk-based
DB uses (postgres included), not something we invented.

## page_index
which page, as a number. page 0 is bytes `[0..4096)` of the file, page 1 is
`[4096..8192)`, and so on. no lookup table — just `page_index * PAGE_SIZE`
to find it on disk. pages never move once written, so this always works.

## slot (as in "row location")
when something needs to point at a specific row (an index entry, a WAL
delete record), it's always `(page_index, slot)` — page + which slot in
that page. this pair is our version of postgres's `ctid`.

## page header
first few bytes of every page: `slot_count`, `free_space_offset`,
`page_lsn`. lives INSIDE the page's own bytes, not as separate fields
next to it — that's what makes a page literally serializable to disk as-is,
no separate metadata file needed to reconstruct it.

## tombstone
a slot marked dead (not physically removed). pages are append-only, can't
reclaim the space yet — DELETE just flips a bit. get_row skips dead slots.
real space reclaim = compaction, not built.

## buffer pool
in-memory cache of pages, keyed by page_index. avoids hitting disk for a
page that's already in memory. write-back: writes go to the cache first,
disk write happens later (on eviction or explicit flush), not immediately.

## dirty page
a cached page that's been written to but not yet flushed to disk. buffer
pool has to flush dirty pages before evicting them, or the write is lost.

## WAL (write-ahead log)
append-only file of "what changed," written and fsync'd BEFORE the actual
page write happens. if the process crashes after the WAL write but before
the page write, replay the WAL on restart to recover what was lost. this is
the entire reason write-back (fast, deferred page writes) is safe at all.

## LSN (log sequence number)
a number identifying "how far into the WAL" a record is. ours = byte offset
in the WAL file right before that record was written. always increasing,
free to compute (append-only file).

## page_lsn
the LSN of the last WAL record that's reflected in a given page, stamped
into the page's own header on every write. on WAL replay: if a page's
stamped LSN is already >= a record's LSN, skip that record — it's already
durably on disk (probably via buffer pool eviction flushing it
independently of any checkpoint). without this, replay would duplicate
writes that were already safe.

## B+tree
sorted tree of pages for fast lookup by key. only LEAF pages hold real
data (key -> row location). INTERNAL pages hold only routing keys + child
pointers, no data — that's the "+" in B+tree, and it's the standard
industry design (postgres, innodb, sqlite all do this), not a simplification.

## secondary index
a B+tree that's separate from the table's own storage — maps
key -> (page_index, slot) pointing back at the real row. table storage
itself doesn't change. opposite of a clustered index, where the table
*is* the tree, ordered by key. we only have secondary indexes right now.

## root page
the B+tree's entry point, always page 0 of the index file, forever — never
moves. when the root needs to split, its contents move to two new pages
and page 0 gets overwritten with a fresh internal node. means there's
never a separate "where's the root" pointer to keep in sync.