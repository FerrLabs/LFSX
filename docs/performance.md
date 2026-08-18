# Performance

Numbers from `bench/throughput.sh`, run on a GitHub-hosted `ubuntu-latest` runner — Linux 6.17,
4 cores, 16 GiB, loopback, local disk. Rerun it yourself with `bash bench/throughput.sh`, or read
the [Benchmark workflow](../.github/workflows/bench.yml) which publishes a table on every change to
the storage path.

| Measure | Result |
|---|---|
| Upload, 1 GiB single object | 117 MiB/s |
| Download, 1 GiB single object | 141 MiB/s |
| 1000 objects of 64 KiB, sequential | 1.4 ms per object, 45 MiB/s |
| Resident memory, idle → peak | 5 MiB → 6 MiB |

The memory row is the one worth looking at. A gigabyte moves through the process and its resident
set grows by one megabyte, which is what "nothing is buffered" means in practice rather than as a
claim. Upload is the slower direction because every byte is hashed and the object is flushed to
disk before it is acknowledged — that cost buys the guarantee that an accepted object is on disk
and matches its digest.

The small-object row is per-request overhead rather than bandwidth: at 64 KiB the transfer itself
is a fraction of a millisecond, so 1.4 ms is essentially what it costs to accept, verify, fsync and
rename one object. A Unity project pushing ten thousand small assets spends about fourteen seconds
of it.

No comparison against another implementation yet. Doing it honestly means driving both servers with
the same client rather than curl, since their object endpoints differ, and that harness does not
exist here — an unfair benchmark against a competitor is worse than none.
