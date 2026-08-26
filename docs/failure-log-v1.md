# Failure log schema v1

`--log-json FILE` appends one JSON object per line describing failures. `FILE`
may be `-`, meaning stderr.

It is independent of `--progress-json`: failures are captured during ordinary
human-readable runs too, and the file survives a terminal nobody was watching.
When `--progress-json` is also active, the same records appear in that stdout
stream, so a consumer of either sees the failure.

## Record

| Field | Always | Meaning |
|---|---|---|
| `schema_version` | yes | `1` |
| `type` | yes | `fatal`, `failed`, or `info` |
| `timestamp_unix_nanos` | yes | When the record was produced |
| `origin` | yes | `client` or `server` — which end produced it |
| `kind` | yes | Stable machine-readable family (see below) |
| `pid` | yes | Process that produced it, on that end |
| `message` | yes | Human-readable detail |
| `path` | no | Destination-relative path, when one entry is at fault |
| `host` | no | Remote authority, when the failure concerns a peer |

Optional fields are **omitted**, never null.

`type` distinguishes severity: `fatal` ended the run, `failed` is one entry
while unrelated work continued, `info` is lifecycle context carried so a JSON
sink stays a single parseable stream rather than a mix of JSON and bare text.

## `kind` values

Deliberately coarse, and deliberately separate from `message`: the message is
for a human and may be reworded, while `kind` is what a consumer routes on.
Grouping follows what an operator would *do* about it.

`protocol`, `io`, `scan`, `plan`, `path`, `remote`, `transport`,
`peer-disconnected`, `missing-remote-binary`, `bootstrap`, `remote-shell`,
`remote-flag-rejected`, `entry`, `warning`, `local`, `rsync`, `lifecycle`.

## Both ends

The remote `xs --server` is asked to emit records to its own stderr, which the
client already captures. The client routes those into the same sinks unchanged,
so one log holds both ends of a transfer and `origin` tells them apart. The
server's **stdout is the binary protocol** and can never carry diagnostics —
that is why `-` means stderr.

A single failure typically produces one `server` record naming the root cause
and one `client` record describing how the client saw it. Reading only the
client's view usually tells you a pipe broke; the server record tells you why.

## Older remotes

A remote whose argument parser does not know `--log-json` refuses it outright.
The client detects that, drops the flag for that host, and retries: a logging
preference never costs a transfer. Such a remote's diagnostics are relayed as
plain text exactly as before, so the feature degrades rather than breaking.
