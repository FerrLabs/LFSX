# Objects in Google Cloud Storage

`LFSX_STORAGE=gcs` keeps the objects and the locks in a Cloud Storage bucket through the JSON API,
authenticated by a service account key or by the pod's own identity on GKE. Everything
[Objects in a bucket](buckets.md) says about the layout, deduplication, collection and the local
cache holds here too.

```bash
LFSX_STORAGE=gcs
LFSX_GCS_BUCKET=studio-lfs
```

The bucket has to exist, and the server reports not ready until it can list it.

## Credentials

`LFSX_GCS_CREDENTIALS` decides how the server authenticates:

| Value | What it does |
|---|---|
| unset | the metadata server, which on GKE is workload identity: annotate the Kubernetes service account with `iam.gke.io/gcp-service-account` and grant that account `roles/storage.objectAdmin` on the bucket |
| a path | a service account key file, the JSON Google hands out. The server trades a signed assertion for an access token, and it is the only credential that can sign a download URL for a client |
| `none` | no credentials at all, for an emulator such as fake-gcs-server |

Tokens are cached until five minutes before they expire. `LFSX_GCS_ENDPOINT` overrides
`https://storage.googleapis.com`, for an emulator or a private endpoint.

## Why not `LFSX_STORAGE=s3` against the interoperability API

Cloud Storage answers the S3 API at `https://storage.googleapis.com` with HMAC keys, and the startup
probes would tell you whether it refuses the second conditional write locks depend on. It is
untested here, and HMAC keys are long-lived secrets tied to a service account, which is what
workload identity exists to remove. The native client takes the pod's identity.

## What differs from S3

**Locks** are a write with `ifGenerationMatch=0`, which succeeds only if no object exists under the
name and answers `412` otherwise. The startup probe still writes one key twice and requires the
second to be refused.

**Large objects** above 256 MiB go up as a resumable upload in chunks of 64 MiB, each naming its
range, so the object appears whole or not at all. An abandoned session expires on its own after a
week.

**Redirects.** `LFSX_S3_PRESIGN=true` hands downloads to the bucket through a V4 signed URL, which
needs a service account key; with the metadata server, downloads keep coming through this server and
the log says so. Uploads always come through the server: a signed write URL is only safe if the
store refuses a body that does not hash to the object, and Cloud Storage can bind an MD5 or a CRC32C
to a write but not the SHA-256 an oid is. `LFSX_S3_CACHE_DIR` and `LFSX_S3_CACHE_MAX_BYTES` apply as
they do for a bucket.

The URL signing follows Google's V4 conformance vectors, which the unit tests check byte for byte.

## Testing against fake-gcs-server

```bash
docker run -d --name gcs -p 4443:4443 fsouza/fake-gcs-server -scheme http -port 4443 \
  -public-host 127.0.0.1:4443
curl -X POST http://127.0.0.1:4443/storage/v1/b -H 'content-type: application/json' \
  -d '{"name":"lfsx-test"}'
LFSX_TEST_GCS_ENDPOINT=http://127.0.0.1:4443 cargo test --test bucket
```

The bucket suite runs unchanged, two replicas racing for the same lock included. It skips the tests
about clients uploading straight to the store, and the two about signed downloads, since an emulator
holds no service account key.
