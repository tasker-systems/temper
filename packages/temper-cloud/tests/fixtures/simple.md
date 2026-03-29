# Getting Started

Welcome to the temper knowledge base documentation.

## Installation

Install temper using cargo:

```bash
cargo install temper
```

Then initialize your vault:

```bash
temper init
```

## Configuration

Temper reads configuration from `temper.toml` in your vault root.

### Search Settings

The search index uses HNSW for approximate nearest neighbor lookup.

### Sync Settings

Device sync uses a manifest-based approach with content-addressed hashing.
