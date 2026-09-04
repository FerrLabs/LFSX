# Performance

Numbers from `bench/throughput.sh`, run on a GitHub-hosted `ubuntu-latest` runner: Linux 6.17,
4 cores, 16 GiB, loopback, local disk. Rerun it yourself with `bash bench/throughput.sh`, or read
the [Benchmark workflow](../.github/workflows/bench.yml) which publishes a table on every change to
the storage path.

| Measure | Result | Across runs |
|---|---|---|
| Upload, 1 GiB single object | 150 MiB/s | 117 to 273 |
| Download, 1 GiB single object | 194 MiB/s | 131 to 194 |
| 1000 objects of 64 KiB, sequential | 7.5 ms per object | 1.3 to 16 ms |
| Resident memory, idle → peak | 6 MiB → 7 MiB | reproduced every run |

**Read the throughput rows as an order of magnitude, not a measurement.** The spread in the last
column is what four consecutive runs of the same commit produced on these runners: the small-object
row moved by a factor of twelve and upload by nearly two, so a single figure from one run says more
about which machine the job landed on than about this server. Comparing two commits by one run each
would be reading noise.

The memory row is the one that reproduces, and it is the one worth having. A gigabyte moves through
the process and the resident set grows by a megabyte, which is what "nothing is buffered" means in
practice rather than as a claim.

It has to be said that this row used to be measured wrong. The harness launched the server through
`cargo run` and then read the resident size of that pid, which is cargo's: cargo stays alive as the
parent and hands the work to a child, so the number was the wrapper's, flat whatever the server did,
and it was published as proof of exactly the thing it could not see. The harness runs the binary
directly now, and the figures above are the first ones measured on the server itself.

Upload is the direction that does more work: every byte is hashed and the object is flushed to disk
before it is acknowledged, which buys the guarantee that an accepted object is on disk and matches
its digest. Whether that costs measurable throughput is not something these runs can say.

The small-object row is per-request overhead rather than bandwidth: at 64 KiB the transfer itself is
a fraction of a millisecond, so what is left is the cost of accepting, verifying, fsyncing and
renaming one object. It is also the row the runner disturbs most, since it is the one bound by fsync
on a disk shared with whatever else the machine is doing.

The image ships a musl binary rather than the glibc one this table was first measured with. Both
were run side by side for four samples and neither is consistently ahead: musl was slower on one
upload and faster on the next, faster on downloads, and the spread within each build was larger than
the gap between them. If musl's allocator costs anything at these sizes, it is smaller than what
these runners can resolve.

No comparison against another implementation yet. Doing it honestly means driving both servers with
the same client rather than curl, since their object endpoints differ, and that harness does not
exist here, an unfair benchmark against a competitor is worse than none.
