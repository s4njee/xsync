# T8.1 crossover table

Status: blocked pending T0.6 shaped-transfer evidence.

Existing Story 8.1 evidence establishes the direction of the effect: compression greatly
reduces wire bytes for the compressible corpus and is correctly skipped for the incompressible
corpus. It does not establish the link speed where compression or deduplication becomes worth
its CPU cost.

T0.6 provides opt-in support for cold-cache labels and SSH bandwidth shaping, but no retained
50, 100, or 1000 Mbit transfer report exists in this checkout. The required documentation audit
for regime-qualified multipliers is also outstanding.

## Plain-English result

We know when compression saves bytes, but not yet when it makes the whole transfer faster. Run
the shaped congress and manga measurements, record their latency and cache state, then publish
the crossover as a range rather than a universal number.
