# `--progress-json` schema v1

Each stdout line is one JSON object with `type`, `schema_version: 1`, and
`timestamp_unix_nanos`. Unknown fields are forward-compatible. `type` values are `phase`,
`started`, `planned`, `cloud_placeholders`, `action`,
`metrics`, `transferred`, `skipped`, `deleted`, `warning`, `failed`, and `done`.

Common fields:

- `started`: `local_workers`, `streams`
- `phase`: `name` (`scan`, `plan`, `transfer`, or `metadata`) and `started` (`true` or `false`)
- `metrics`: `queue_high_water`, `compression_algorithm`, and `compression_level`
- `planned`: `files`, `bytes`
- `cloud_placeholders`: `files`, `bytes`, `detection_available`, `detection_performed`
- `action`: `path`, `action` (`create`, `update`, or `delete`)
- `transferred`: `path`, `bytes`, `physical_bytes`, `method`
- `skipped`/`deleted`: `path`
- `warning`/`failed`: `path`, `message`

The `done` event contains the complete transfer summary: logical, physical, and wire bytes;
transferred/skipped/deleted/failed counts; local workers and streams; clone/copy counts; resume and
retransmission counters; transport identity; negotiated wire version; mapped options; unavailable
guarantees; checksum/compression algorithms; and the selection reason.

`cloud_placeholders` reports an inventory only when `detection_performed` is `true`.
Detection costs a process spawn per file, so it runs only under the `skip` and `error`
policies, whose outcome depends on the answer. Under the default `download` policy, on the
whole-tree clone fast path, and on every remote route, `detection_performed` is `false` and
`files`/`bytes` are zero because nothing was inspected — not because no placeholder exists.
Consumers must check `detection_performed` before treating the counts as an inventory.
`detection_available` remains a platform capability flag and is independent of it.

`detection_performed` was added additively; a consumer that does not read it sees the same
`files`/`bytes` semantics it saw before for the `skip` and `error` policies.

Phase timing is represented by paired `phase` events. Consumers record the timestamp on the
`started: true` event and subtract it from the matching `started: false` event. Absent compression
values are JSON `null`. New fields may be ignored; changes to existing field meanings require a
new schema version.

## Failure records

A run that ends in an error now emits a `fatal` record before exiting; before,
the error that ended the run was printed only as plain text and was the one
event missing from this stream. Records relayed from the remote server appear
here too, tagged `"origin":"server"`.

These records follow [failure-log-v1.md](failure-log-v1.md), which is also what
`--log-json` writes. They carry `origin`, `kind`, and `pid` in addition to the
common fields above.
