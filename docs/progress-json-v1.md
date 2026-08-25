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
- `cloud_placeholders`: `files`, `bytes`, `detection_available`
- `action`: `path`, `action` (`create`, `update`, or `delete`)
- `transferred`: `path`, `bytes`, `physical_bytes`, `method`
- `skipped`/`deleted`: `path`
- `warning`/`failed`: `path`, `message`

The `done` event contains the complete transfer summary: logical, physical, and wire bytes;
transferred/skipped/deleted/failed counts; local workers and streams; clone/copy counts; resume and
retransmission counters; transport identity; negotiated wire version; mapped options; unavailable
guarantees; checksum/compression algorithms; and the selection reason.

Phase timing is represented by paired `phase` events. Consumers record the timestamp on the
`started: true` event and subtract it from the matching `started: false` event. Absent compression
values are JSON `null`. New fields may be ignored; changes to existing field meanings require a
new schema version.
