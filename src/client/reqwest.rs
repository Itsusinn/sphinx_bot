use turingram::client::ClientExecutor;
use http::Uri;
use serde::{Deserialize, Serialize};
use std::future::Future;

pub struct Executor {
    client: reqwest::Client,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl ClientExecutor for Executor {
    type Error = reqwest::Error;

    async fn request<T, U>(&self, uri: Uri, payload: T) -> Result<U, Self::Error>
    where
        T: Serialize + Send,
        U: for<'a> Deserialize<'a>,
    {
        let url = uri.to_string();
        self.client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .json::<U>()
            .await
    }
}
