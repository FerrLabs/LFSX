use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Storage {
    Local,
    // Endpoint, bucket and credentials all have to be there: a bucket the server
    // cannot reach is a server that answers every push with an error, and
    // discovering that on the first upload rather than at boot is the wrong
    // order.
    Bucket {
        dialect: Dialect,
        // Whether a download is redirected to the bucket instead of streamed
        // through this server. Off by default: the streamed path is the one
        // that counts bytes, serves ranges and holds the ceiling, and an
        // operator should choose to give those up rather than discover it.
        presign: bool,
        // A local copy of what the bucket holds, so a second reader does not
        // pay the round trip again. None is no cache at all, which is what a
        // deployment that never set it keeps getting.
        cache: Option<DiskCache>,
        // Whether locks can be taken here. Not read from the environment: it
        // starts true and the startup probes turn it off, the same way they turn
        // `presign` off, when the store will not prove it can arbitrate between
        // two writers racing for the same key.
        locking: bool,
    },
}

#[derive(Debug, Clone)]
pub enum Dialect {
    S3 {
        endpoint: String,
        bucket: String,
        region: String,
        access_key: String,
        secret_key: String,
        path_style: bool,
    },
    Azure {
        endpoint: String,
        account: String,
        container: String,
        credential: AzureCredential,
    },
    Gcs {
        endpoint: String,
        bucket: String,
        credential: GcsCredential,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcsCredential {
    ServiceAccount(PathBuf),
    Metadata,
    Anonymous,
}

pub(super) fn gcs_credential(value: Option<&str>) -> GcsCredential {
    match value.filter(|value| !value.is_empty()) {
        None => GcsCredential::Metadata,
        Some("none") => GcsCredential::Anonymous,
        Some(path) => GcsCredential::ServiceAccount(path.into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AzureCredential {
    AccountKey(String),
    Sas(String),
    Identity,
}

pub(super) fn azure_credential(key: Option<&str>, sas: Option<&str>) -> AzureCredential {
    let key = key.filter(|key| !key.is_empty());
    let sas = sas.filter(|sas| !sas.is_empty());

    match (key, sas) {
        (Some(_), Some(_)) => panic!(
            "LFSX_AZURE_ACCOUNT_KEY and LFSX_AZURE_SAS_TOKEN are both set: pick one, or neither \
             to authenticate with the pod's managed or workload identity"
        ),
        (Some(key), None) => AzureCredential::AccountKey(key.to_owned()),
        (None, Some(sas)) => AzureCredential::Sas(sas.to_owned()),
        (None, None) => AzureCredential::Identity,
    }
}

impl Storage {
    pub(super) fn from_env() -> Self {
        let kind = std::env::var("LFSX_STORAGE").unwrap_or_default();
        if !matches!(kind.as_str(), "s3" | "azure" | "gcs") {
            return Self::Local;
        }

        let required = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| panic!("LFSX_STORAGE={kind} needs {name}"))
        };

        let dialect = if kind == "gcs" {
            Dialect::Gcs {
                endpoint: std::env::var("LFSX_GCS_ENDPOINT")
                    .ok()
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "https://storage.googleapis.com".into()),
                bucket: required("LFSX_GCS_BUCKET"),
                credential: gcs_credential(std::env::var("LFSX_GCS_CREDENTIALS").ok().as_deref()),
            }
        } else if kind == "azure" {
            let account = required("LFSX_AZURE_ACCOUNT");
            Dialect::Azure {
                endpoint: std::env::var("LFSX_AZURE_ENDPOINT")
                    .ok()
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| format!("https://{account}.blob.core.windows.net")),
                container: required("LFSX_AZURE_CONTAINER"),
                credential: azure_credential(
                    std::env::var("LFSX_AZURE_ACCOUNT_KEY").ok().as_deref(),
                    std::env::var("LFSX_AZURE_SAS_TOKEN").ok().as_deref(),
                ),
                account,
            }
        } else {
            Dialect::S3 {
                endpoint: required("LFSX_S3_ENDPOINT"),
                bucket: required("LFSX_S3_BUCKET"),
                region: std::env::var("LFSX_S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
                access_key: required("LFSX_S3_ACCESS_KEY"),
                secret_key: required("LFSX_S3_SECRET_KEY"),
                path_style: std::env::var("LFSX_S3_PATH_STYLE").as_deref() != Ok("false"),
            }
        };

        Self::Bucket {
            dialect,
            presign: std::env::var("LFSX_S3_PRESIGN").as_deref() == Ok("true"),
            cache: disk_cache(
                std::env::var("LFSX_S3_CACHE_DIR").ok().as_deref(),
                std::env::var("LFSX_S3_CACHE_MAX_BYTES").ok().as_deref(),
            ),
            locking: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskCache {
    pub dir: PathBuf,
    pub max_bytes: u64,
}

// A directory and a ceiling, together or not at all. A cache with nowhere to
// live is nothing, and one with no ceiling fills the disk the server also
// stages uploads on, which is a worse outage than the bucket round trips it
// was meant to save.
pub(super) fn disk_cache(dir: Option<&str>, max_bytes: Option<&str>) -> Option<DiskCache> {
    let dir = dir.filter(|dir| !dir.is_empty())?;

    let Some(max_bytes) = max_bytes
        .map(str::trim)
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|ceiling| *ceiling > 0)
    else {
        tracing::warn!(
            "LFSX_S3_CACHE_DIR is set without a usable LFSX_S3_CACHE_MAX_BYTES, so nothing is \
             cached: a cache with no ceiling would fill the volume this server stages uploads on"
        );
        return None;
    };

    Some(DiskCache {
        dir: PathBuf::from(dir),
        max_bytes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    File(PathBuf),
    Command(String),
}

// One source or none. Both is a configuration that says two things, and which
// of them the operator trusts with the store is not a guess this server makes.
pub(super) fn encryption_key(file: Option<&str>, command: Option<&str>) -> Option<KeySource> {
    let file = file.filter(|path| !path.is_empty());
    let command = command.filter(|hook| !hook.is_empty());

    match (file, command) {
        (None, None) => None,
        (Some(path), None) => Some(KeySource::File(PathBuf::from(path))),
        (None, Some(hook)) => Some(KeySource::Command(hook.to_owned())),
        (Some(_), Some(_)) => panic!(
            "LFSX_ENCRYPTION_KEY_FILE and LFSX_ENCRYPTION_KEY_COMMAND are both set: they are two \
             answers to where the keys live, and picking one for you is how the wrong keys get used"
        ),
    }
}
