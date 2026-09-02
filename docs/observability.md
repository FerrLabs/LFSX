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
| `lfsx_transfers_in_flight` | gauge | uploads and downloads holding a transfer slot right now, against `LFSX_MAX_CONCURRENT_TRANSFERS` |

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

## Traces

Metrics say a push was slow; the trace says where the time went. Set `LFSX_OTLP_ENDPOINT` to the
HTTP traces URL of an OTLP collector and every request becomes a span, with the storage and forge
calls as children: the batch resolver's per-object fan-out, the forge permission lookup, the
codec work on the way past. Outbound forge calls carry W3C trace context, so anything on the path
that participates lands in the same trace.

```bash
LFSX_OTLP_ENDPOINT=http://collector:4318/v1/traces
```

Unset, the layer is not installed at all, which is the default and costs nothing. Metrics stay
Prometheus either way; there is no OTLP metrics export and no reason for one.

## Audit trail

Every privileged mutation lands on the `lfsx::audit` tracing target as one event naming who acted.
The trail is a log stream, not a store: the server has no database on purpose, so durability
belongs to wherever you ship logs. Route it on its own without turning anything else up:

```bash
RUST_LOG=lfsx::audit=info
```

What is on it, each with the actor (the forge login of the token that asked, or `anonymous` with
auth disabled) and the namespace:

| Event | Also carries |
|---|---|
| a retain sweep unlinked objects | `swept`, `bytes`, `within_grace` |
| a repository was folded into the shared store (dedupe) | `adopted`, `linked`, `reclaimed`, `refused` |
| stored objects were rewritten compressed | `compressed`, `before`, `after` |
| a lock was force-opened over its owner | `path`, `owner` |
| a stale lock was taken over | `path`, `previous_owner`, `untouched_for_seconds` |
| a bucket upload was adopted (pre-signed verify) | `oid`, `bytes` |

Dry runs are reads and stay off the trail. The actor is resolved before the mutation runs, so a
privileged operation that cannot be attributed is refused rather than performed anonymously.
