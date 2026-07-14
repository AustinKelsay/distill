# Distill CLI

Thin Rust CLI over the public Library Fixture journey.

## Usage

```bash
cargo run -p distill-cli -- --home /tmp/distill-home --fixture /path/to/fixture
cargo run -p distill-cli -- --home /tmp/distill-home --fixture /path/to/fixture --format json
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Fixture journey succeeded |
| `1` | Library or runtime failure |
| `2` | Usage or invalid arguments |

The CLI does not implement product policy; it only validates paths, calls `Library::run_fixture_journey`, and prints stable human or JSON output.
