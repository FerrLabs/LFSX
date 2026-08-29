# Observability

`/metrics` serves the Prometheus text format, unauthenticated like the probes: an orchestrator
scraping it has no forge token, and refusing it would only mean the one moment you need numbers is
the one moment you cannot get them.

| Metric | Kind | Answers |
|---|---|---|
| `lfsx_requests_total{route,status}` | counter | what is being served, and what is failing |
| `lfsx_request_duration_seconds{route}` | histogram | how long transfers take |
| `lfsx_uploaded_bytes_total`, `lfsx_downloaded_bytes_total` | counter | throughput in and out |
| `lfsx_object_size_bytes` | histogram | what people are actually storing |
| `lfsx_rejections_total{cause}` | counter | why requests are refused, by cause rather than by status |
| `lfsx_objects_stored`, `lfsx_store_bytes` | gauge | how full the disk is getting, counting shared objects once |
| `lfsx_store_scans` | gauge | how often the expensive walk behind those two actually ran |

Routes are labelled by their template, never by the path, so the object id can never turn into a
label and the series count stays bounded whatever you store.

Those two count what the disk holds, not what the repositories logically hold: an object shared by
three projects is one set of bytes and is counted once. The per-repository page reports logical
size instead, since "this project uses 3 GiB of assets" is the useful answer there even when some
of it is shared.

The two disk gauges are measured by walking the store, so they are computed at most once a minute
and reused in between, and concurrent scrapes queue behind a single walk rather than each starting
their own, which is what keeps an unauthenticated endpoint from being a lever on a large disk.
`lfsx_store_scans` is how you check that: it should climb about once a minute under load, not once
per request.

`lfsx_downloaded_bytes_total` counts bytes as they are streamed, so a client that disconnects
halfway is not recorded as a full download.
