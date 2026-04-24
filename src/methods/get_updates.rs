use turingram::methods::Method;
use turingram::types::Update;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct GetUpdates {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
}

impl Method for GetUpdates {
    type Response = Vec<Update>;
    const NAME: &str = "getUpdates";
}
