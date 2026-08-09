# Crumble Architecture

Crumble is a layered database system with a compiler-inspired query pipeline.

The architecture is intentionally incremental. Components will be introduced as the project evolves.

## High-Level Architecture

```mermaid
flowchart TD
    SQL["SQL Query"]
    Parser["SQL Parser"]
    AST["AST"]
    IR["Query IR"]
    Opt["Optimizer"]
    Exec["Execution Engine"]

    Buffer["Buffer Pool"]
    MVCC["MVCC"]
    Index["Index Manager"]
    Storage["Storage Engine"]
    WAL["Write-Ahead Log"]
    Disk["Persistent Storage"]

    SQL --> Parser
    Parser --> AST
    AST --> IR
    IR --> Opt
    Opt --> Exec

    Exec --> Buffer
    Exec --> MVCC
    Exec --> Index

    Buffer --> Storage
    MVCC --> Storage
    Index --> Storage

    Storage --> WAL
    WAL --> Disk
    Storage --> Disk
```

## Query Pipeline

The query pipeline transforms SQL into an executable representation.

```mermaid
flowchart LR
    SQL["SQL"]
    Lexer["Lexer"]
    Parser["Parser"]
    AST["AST"]
    IR["Query IR"]
    Optimizer["Optimization Passes"]
    Plan["Physical Plan"]
    Executor["Executor"]

    SQL --> Lexer
    Lexer --> Parser
    Parser --> AST
    AST --> IR
    IR --> Optimizer
    Optimizer --> Plan
    Plan --> Executor
```

## Query IR

The Query IR is the central representation between query parsing and execution.

```mermaid
flowchart LR
    AST["AST"] --> IR["Logical IR"]
    IR --> P1["Optimization Pass"]
    P1 --> P2["Optimization Pass"]
    P2 --> P3["Optimization Pass"]
    P3 --> PIR["Physical IR"]
    PIR --> Executor["Executor"]
```

The goal is to make query transformations explicit and independently testable.

The IR is inspired by compiler intermediate representations, particularly the idea of transforming a program through a sequence of well-defined representations and optimization passes.

## Execution

Crumble should eventually support experimentation with different execution strategies.

```mermaid
flowchart TD
    IR["Physical IR"] --> Executor["Execution Engine"]

    Executor --> Interpreter["Interpreter"]
    Executor --> Vectorized["Vectorized Executor"]
    Executor --> JIT["JIT / Compiled Executor"]
```

The same query representation should be capable of being executed using different strategies.

This allows execution techniques to be compared without changing the SQL or storage layers.

## Storage

The storage subsystem is responsible for turning logical database operations into persistent data.

```mermaid
flowchart TD
    Executor["Executor"]
    Buffer["Buffer Pool"]
    Storage["Storage Engine"]
    Page["Pages"]
    WAL["Write-Ahead Log"]
    Disk["Disk"]

    Executor --> Buffer
    Buffer --> Storage
    Storage --> Page
    Page --> Disk

    Executor --> WAL
    WAL --> Disk
```

## Transactions and MVCC

Transactions introduce visibility and concurrency semantics over the storage layer.

```mermaid
flowchart TD
    Transaction["Transaction"]
    MVCC["MVCC"]
    Buffer["Buffer Pool"]
    Storage["Storage Engine"]
    WAL["WAL"]

    Transaction --> MVCC
    MVCC --> Buffer
    Buffer --> Storage
    Transaction --> WAL
```

The exact transaction and versioning model will evolve as the implementation progresses.

## Architectural Principles

### Separation of Concerns

Query processing, execution, concurrency, and storage should have clear boundaries.

### Explicit Representations

Important transformations should use explicit representations rather than being hidden inside large abstractions.

### Composable Optimization

Optimization should be implemented as independently testable passes over the Query IR.

### Experimentation

The architecture should allow alternative execution and storage strategies to be explored without rewriting unrelated components.

### Measure, Don't Assume

Performance-sensitive decisions should be validated through benchmarks and measurements.

### Incremental Complexity

Crumble will grow incrementally. Components should be introduced when their underlying concepts are understood and can be tested.

## Architecture Evolution

This document describes the current and intended architecture.

It is expected to change as Crumble evolves. Architectural changes should be accompanied by a corresponding design document explaining the motivation and tradeoffs behind the change.
