# xsync compression sampling spike

- Schema: `xsync.compression-probe.report.v1`
- zstd: `1.5.7`
- Rule: select zstd level 3 when bounded sample output is <= 95% of input

| Corpus | Sample | Selected files | Logical | Raw wire | Adaptive wire | Ratio |
|---|---:|---:|---:|---:|---:|---:|
| short-files | 65536 | 32 | 2097152 | 2097600 | 2144 | 0.00102x |
| short-files | 262144 | 32 | 2097152 | 2097600 | 2144 | 0.00102x |
| short-files | 1048576 | 32 | 2097152 | 2097600 | 2144 | 0.00102x |
| compressible | 65536 | 256 | 268435456 | 268439040 | 39424 | 0.00015x |
| compressible | 262144 | 256 | 268435456 | 268439040 | 39424 | 0.00015x |
| compressible | 1048576 | 256 | 268435456 | 268439040 | 39424 | 0.00015x |
| incompressible | 65536 | 0 | 268435456 | 268439040 | 268439040 | 1.00000x |
| incompressible | 262144 | 0 | 268435456 | 268439040 | 268439040 | 1.00000x |
| incompressible | 1048576 | 0 | 268435456 | 268439040 | 268439040 | 1.00000x |
| mixed | 65536 | 4102 | 35361530 | 35476386 | 18255325 | 0.51458x |
| mixed | 262144 | 4102 | 35361530 | 35476386 | 18255325 | 0.51458x |
| mixed | 1048576 | 4102 | 35361530 | 35476386 | 18255325 | 0.51458x |
