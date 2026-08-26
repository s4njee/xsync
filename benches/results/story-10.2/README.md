# Story 10.2 List Benchmark

The ignored benchmark test `server::tests::list_first_page_100k_entry_benchmark`
creates 100,000 empty entries and requests a 100-entry first page. It verifies
that only one page is returned and that the page is non-final, while printing
the first-page latency. Run it with:

```sh
cargo test -p xsync-core list_first_page_100k_entry_benchmark -- --ignored --nocapture
```

Observed on the local macOS benchmark host: `100/100000` entries in `848.625us`
(filesystem setup excluded from the timed section).
