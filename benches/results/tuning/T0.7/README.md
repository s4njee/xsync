# T0.7 results — real-corpus mutation states

The live `congress-10k` corpus was exercised on the local APFS same-volume route with five
repetitions each for:

- `no-op-second-sync`
- `content-churn`
- `delete`

All three matrix cells passed. Every repetition passed the full destination oracle, and all
reports retained the pinned source manifest digest
`f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6`.

The runner records the deterministic mutation selection in each report. The source corpus was
never mutated; churn and delete changed only disposable destination trees.

Evidence:

- `/tmp/xsync-t0.7-no-op-second-sync-out/`
- `/tmp/xsync-t0.7-content-churn-out/`
- `/tmp/xsync-t0.7-delete-out/`

Plain English: these runs prove that the benchmark can represent the three important follow-up
states after an initial copy—nothing changed, some file contents changed, and some files were
removed—without silently changing the source dataset. Remote mutation measurements remain
deferred until remote destination preparation exists.
