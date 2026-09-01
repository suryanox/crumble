# Crumble

Crumble is a database built from first principles in Rust.

The goal is to understand how a database actually works by implementing its core components rather than treating the database as a black box.

The architecture is intentionally inspired by compiler design: SQL is transformed into an intermediate representation, optimized through passes, and eventually executed by the database engine.

## Why Crumble?

The goal isn't to build the next PostgreSQL. The goal is to understand the engineering decisions, tradeoffs, algorithms, and abstractions that make databases work.

Every subsystem is built incrementally, measured, tested, and documented. If I can't explain why the code exists, it doesn't belong in Crumble.

## Architecture

* [Architecture](docs/architecture/README.md)
* [Design](docs/design/README.md)
* [Decisions & tradeoffs](docs/decisions/tradeoffs.md)
* [Guide](docs/guide.md)
* [Status & Features](STATUS.md) — what's built, what's next

## Development Philosophy

Crumble is **not a vibe-coded project**. AI-assisted implementation is intentionally not used for the core implementation. The purpose is to learn by: understanding, designing, implementing, testing, studying what went wrong, and iterating.

## License

Crumble is licensed under the [MIT License](LICENSE).