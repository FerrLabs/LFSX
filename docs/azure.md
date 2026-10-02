# Objects in Azure Blob Storage

`LFSX_STORAGE=azure` keeps the objects and the locks in a Blob Storage container. Azure has no S3
API, so this is a native client rather than `LFSX_STORAGE=s3` pointed at a gateway, and everything
[Objects in a bucket](buckets.md) says about the layout, deduplication, collection and the local
cache holds here too.

```bash
LFSX_STORAGE=azure
LFSX_AZURE_ACCOUNT=studioassets
LFSX_AZURE_CONTAINER=lfs
LFSX_AZURE_ACCOUNT_KEY=…
```

The container has to exist. The server never creates one, and it reports not ready until it can
list the container, so a typo in the name shows up at boot rather than on the first push.

## Credentials

Set at most one of these. Neither means the pod's own identity.

| Variable | What it does |
|---|---|
| `LFSX_AZURE_ACCOUNT_KEY` | the storage account key. The server signs a short-lived SAS for every request with it, and it is the only credential that can sign a download URL for a client |
| `LFSX_AZURE_SAS_TOKEN` | a SAS you issued, container scope, with read, add, create, write, delete and list. It is never handed to a client: it covers the whole container, and a client should get one object |
| neither | managed identity, or workload identity on AKS when `AZURE_FEDERATED_TOKEN_FILE`, `AZURE_TENANT_ID` and `AZURE_CLIENT_ID` are set, which the workload identity webhook does. The identity needs `Storage Blob Data Contributor` on the container |

`LFSX_AZURE_ENDPOINT` overrides the blob endpoint, `https://<account>.blob.core.windows.net` by
default, for Azurite, a private endpoint or a sovereign cloud.

## What differs from S3

**Locks** use the same conditional write, `If-None-Match: *`, which Azure answers with `409` rather
than `412`. The startup probe still writes one key twice and requires the second to be refused, and
locking switches off rather than pretending if it is not.

**Large objects** go up as blocks above 5,000 MiB, Azure's single-request ceiling, in blocks of
64 MiB grown when fifty thousand of them would not cover the object, and are committed with one
block list. Uncommitted blocks expire on their own after a week, so an interrupted upload leaves
nothing to clean up.

**Redirects.** `LFSX_S3_PRESIGN=true` hands downloads to the container through a SAS scoped to one
blob, which needs the account key; with a SAS token or an identity, downloads keep coming through
the server and the log says so. Uploads always come through the server: a signed write URL is only
safe if the store refuses a body that does not hash to the object, and Azure can bind an MD5 to a
write but not the SHA-256 an oid is. `LFSX_S3_CACHE_DIR` and `LFSX_S3_CACHE_MAX_BYTES` apply as
they do for a bucket.

## Testing against Azurite

```bash
docker run -d --name azurite -p 10000:10000 mcr.microsoft.com/azure-storage/azurite \
  azurite-blob --blobHost 0.0.0.0 --loose
az storage container create --name lfsx-test --connection-string \
  "DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://127.0.0.1:10000/devstoreaccount1;"
LFSX_TEST_AZURE_ENDPOINT=http://127.0.0.1:10000/devstoreaccount1 cargo test --test bucket
```

The bucket suite runs unchanged, two replicas racing for the same lock included, and skips only the
tests about clients uploading straight to the store.
