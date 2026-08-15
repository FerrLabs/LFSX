use serde::{Deserialize, Serialize};

use crate::locks::Lock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Upload,
    Download,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ObjectId {
    pub oid: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct BatchRequest {
    pub operation: Operation,
    #[serde(default)]
    pub transfers: Vec<String>,
    pub objects: Vec<ObjectId>,
}

#[derive(Debug, Deserialize)]
pub struct RetainRequest {
    pub oids: Vec<String>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize)]
pub struct DedupeRequest {
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct BatchResponse {
    pub transfer: &'static str,
    pub objects: Vec<ObjectSpec>,
}

#[derive(Debug, Serialize)]
pub struct ObjectSpec {
    #[serde(flatten)]
    pub id: ObjectId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Actions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ObjectError>,
}

#[derive(Debug, Default, Serialize)]
pub struct Actions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<Action>,
}

#[derive(Debug, Serialize)]
pub struct Action {
    pub href: String,
    pub expires_in: u32,
}

#[derive(Debug, Serialize)]
pub struct ObjectError {
    pub code: u16,
    pub message: String,
}

impl ObjectSpec {
    pub fn missing(id: ObjectId) -> Self {
        Self {
            id,
            actions: None,
            error: Some(ObjectError {
                code: 404,
                message: "object not found".into(),
            }),
        }
    }

    pub fn too_large(id: ObjectId, limit: u64) -> Self {
        Self {
            error: Some(ObjectError {
                code: 413,
                message: format!(
                    "object is {} bytes, over the {limit} byte limit this server accepts",
                    id.size
                ),
            }),
            id,
            actions: None,
        }
    }

    pub fn over_quota(id: ObjectId, used: u64, limit: u64) -> Self {
        Self {
            id,
            actions: None,
            error: Some(ObjectError {
                code: 507,
                message: format!(
                    "this repository holds {used} bytes of its {limit} byte budget — collect what \
                     it no longer references, or ask for more room"
                ),
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateLockRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ListLocksQuery {
    pub path: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct VerifyLocksRequest {}

#[derive(Debug, Deserialize)]
pub struct UnlockRequest {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct LockResponse {
    pub lock: Lock,
}

#[derive(Debug, Serialize)]
pub struct ListLocksResponse {
    pub locks: Vec<Lock>,
    pub next_cursor: &'static str,
}

#[derive(Debug, Serialize)]
pub struct VerifyLocksResponse {
    pub ours: Vec<Lock>,
    pub theirs: Vec<Lock>,
    pub next_cursor: &'static str,
}

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub objects: u64,
    pub bytes: u64,
    pub locks: usize,
}
