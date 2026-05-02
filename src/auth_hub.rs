use worker::*;

/// Single-instance Durable Object that tracks pending auth for each user.
/// Stores a JSON array of group_ids per user under key `user_idx:{user_id}`.
/// Replaces WAIT_AUTH_KV to avoid free-plan write limits.
#[durable_object]
pub struct AuthHub {
    state: State,
}

impl DurableObject for AuthHub {
    fn new(state: State, _env: Env) -> Self {
        Self { state }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        let url = req.url()?;

        let user_id: i64 = url
            .query_pairs()
            .find(|(k, _)| k == "user_id")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        if user_id == 0 {
            return Response::error("user_id required", 400);
        }

        let action = url
            .path_segments()
            .and_then(|s| s.last())
            .unwrap_or("");

        let key = format!("user_idx:{}", user_id);

        match action {
            "add_pending" => {
                let group_id: i64 = url
                    .query_pairs()
                    .find(|(k, _)| k == "group_id")
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(0);
                if group_id == 0 {
                    return Response::error("group_id required", 400);
                }

                let mut groups: Vec<i64> = self.state.storage().get(&key).await?.unwrap_or_default();
                if !groups.contains(&group_id) {
                    groups.push(group_id);
                }
                self.state.storage().put(&key, groups).await?;
                Response::ok("OK")
            }
            "remove_pending" => {
                let group_id: i64 = url
                    .query_pairs()
                    .find(|(k, _)| k == "group_id")
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(0);
                if group_id == 0 {
                    return Response::error("group_id required", 400);
                }

                let mut groups: Vec<i64> = self.state.storage().get(&key).await?.unwrap_or_default();
                groups.retain(|g| *g != group_id);
                if groups.is_empty() {
                    self.state.storage().delete(&key).await?;
                } else {
                    self.state.storage().put(&key, groups).await?;
                }
                Response::ok("OK")
            }
            "get_pending" => {
                let groups: Vec<i64> = self.state.storage().get(&key).await?.unwrap_or_default();
                let json = serde_json::to_string(&groups).unwrap_or_else(|_| "[]".to_string());
                Response::ok(&json)
            }
            _ => Response::error("unknown action", 404),
        }
    }
}
