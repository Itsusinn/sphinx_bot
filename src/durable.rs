use std::time::Duration;

use turingram::client::{Client, worker_0_8::Executor};
use worker::*;

use crate::{BanChatMember, GetChatMember, SetChatMemberTag};

/// Durable Object that watches a user after joining a group.
/// Sets an alarm for 2 minutes later. When the alarm fires,
/// checks if the user still has the "suspending" tag. If so,
/// clears the tag and bans for 1 week.
#[durable_object]
pub struct UserWatchDO {
    state: State,
    env: Env,
}

impl DurableObject for UserWatchDO {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    /// Receive scheduling request: ?chat_id=xxx&user_id=xxx
    async fn fetch(&self, req: Request) -> Result<Response> {
        let url = req.url()?;
        let chat_id: i64 = url
            .query_pairs()
            .find(|(k, _)| k == "chat_id")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let user_id: i64 = url
            .query_pairs()
            .find(|(k, _)| k == "user_id")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);

        if chat_id == 0 || user_id == 0 {
            return Response::error("chat_id and user_id required", 400);
        }

        self.state.storage().put("chat_id", chat_id).await?;
        self.state.storage().put("user_id", user_id).await?;

        // Set alarm for 2 minutes from now
        self.state.storage().set_alarm(Duration::from_secs(120)).await?;

        console_log!(
            "UserWatchDO: alarm set for user {} in group {} (+2min)",
            user_id,
            chat_id
        );
        Response::ok("Scheduled")
    }

    /// Alarm fired — check user's tag and ban if still suspending
    async fn alarm(&self) -> Result<Response> {
        let chat_id: i64 = self.state.storage().get("chat_id").await?.unwrap_or(0);
        let user_id: i64 = self.state.storage().get("user_id").await?.unwrap_or(0);

        if chat_id == 0 || user_id == 0 {
            console_log!("UserWatchDO: no target stored, skipping");
            return Response::ok("No target");
        }

        let token = match self.env.var("TELEGRAM_BOT_TOKEN") {
            Ok(t) => t.to_string(),
            Err(e) => {
                console_error!("UserWatchDO: TELEGRAM_BOT_TOKEN missing: {:?}", e);
                return Response::error("token missing", 500);
            }
        };

        let bot = Client::new(Executor::new(), token.trim().to_string());

        let member: serde_json::Value = match bot.execute(GetChatMember { chat_id, user_id }).await
        {
            Ok(v) => v,
            Err(e) => {
                console_error!(
                    "UserWatchDO: failed to check user {} in {}: {:?}",
                    user_id,
                    chat_id,
                    e
                );
                return Response::ok("Check failed");
            }
        };

        let tag = member.get("tag").and_then(|t| t.as_str()).unwrap_or("");
        if tag == "suspending" {
            let now_ms = Date::now().as_millis() as u64;
            let until = now_ms / 1000 + 7 * 24 * 3600;

            // Clear tag first, then ban
            let _ = bot
                .execute(SetChatMemberTag {
                    chat_id,
                    user_id,
                    tag: Some(String::new()),
                })
                .await;

            if let Err(e) = bot
                .execute(BanChatMember {
                    chat_id,
                    user_id,
                    until_date: Some(until),
                })
                .await
            {
                console_error!(
                    "UserWatchDO: failed to ban user {} in {}: {:?}",
                    user_id,
                    chat_id,
                    e
                );
            } else {
                console_log!(
                    "UserWatchDO: banned user {} in group {} (still suspending after 2min)",
                    user_id,
                    chat_id
                );
            }
        } else {
            console_log!(
                "UserWatchDO: user {} in group {} verified within 2min, skipping",
                user_id,
                chat_id
            );
        }

        Response::ok("Done")
    }
}
