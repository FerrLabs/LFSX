use serde::{Deserialize, Serialize};

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
}
