use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use turingram::client::Error as TgramError;
use turingram::client::{Client, worker_0_8::Executor};
use turingram::methods::{AnswerCallbackQuery, SendMessage};
use turingram::types::{
    InlineKeyboardButton, InlineKeyboardButtonKind, MessageKind, ParseMode, ReplyMarkup, True,
    Update, UpdateKind,
};
mod auth_hub;
mod durable;

use auth_hub::AuthHub;
use durable::UserWatchDO;

use worker::*;

#[derive(Debug, Serialize)]
struct ChatPermissions {
    #[serde(skip_serializing_if = "Option::is_none")]
    can_send_messages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_send_audios: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_send_documents: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_send_photos: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_send_videos: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_send_video_notes: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_send_voice_notes: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_send_polls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_send_other_messages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_add_web_page_previews: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_change_info: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_invite_users: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_pin_messages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_manage_topics: Option<bool>,
}

#[derive(Debug, Serialize)]
struct RestrictChatMember {
    chat_id: i64,
    user_id: i64,
    permissions: ChatPermissions,
    #[serde(skip_serializing_if = "Option::is_none")]
    until_date: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    use_independent_chat_permissions: Option<bool>,
}

impl turingram::methods::Method for RestrictChatMember {
    type Response = True;
    const NAME: &str = "restrictChatMember";
}

#[derive(Debug, Serialize)]
struct SetChatMemberTag {
    chat_id: i64,
    user_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

impl turingram::methods::Method for SetChatMemberTag {
    type Response = True;
    const NAME: &str = "setChatMemberTag";
}

#[derive(Debug, Serialize)]
struct GetChatMember {
    chat_id: i64,
    user_id: i64,
}

impl turingram::methods::Method for GetChatMember {
    type Response = Value;
    const NAME: &str = "getChatMember";
}

#[derive(Debug, Serialize)]
struct BanChatMember {
    chat_id: i64,
    user_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    until_date: Option<u64>,
}

impl turingram::methods::Method for BanChatMember {
    type Response = True;
    const NAME: &str = "banChatMember";
}

#[derive(Debug, Serialize)]
struct UnbanChatMember {
    chat_id: i64,
    user_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    only_if_banned: Option<bool>,
}

impl turingram::methods::Method for UnbanChatMember {
    type Response = True;
    const NAME: &str = "unbanChatMember";
}

#[derive(Debug, Serialize)]
struct DeleteMessage {
    chat_id: i64,
    message_id: u32,
}

impl turingram::methods::Method for DeleteMessage {
    type Response = True;
    const NAME: &str = "deleteMessage";
}

#[derive(Deserialize)]
struct QuestionItem {
    text: String,
    options: Vec<String>,
    correct: Vec<usize>,
}

#[derive(Deserialize)]
struct Config {
    question: Vec<QuestionItem>,
}

fn load_questions() -> Vec<QuestionItem> {
    let bytes = include_bytes!("../config.toml");
    let text = std::str::from_utf8(bytes).expect("config.toml must be valid UTF-8");
    let config: Config = toml::from_str(text).expect("Failed to parse config.toml");
    config.question
}

fn pick_question(questions: &[QuestionItem], user_id: i64) -> Option<(usize, &QuestionItem)> {
    if questions.is_empty() {
        return None;
    }
    let idx = (user_id as usize).wrapping_add(Date::now().as_millis() as usize) % questions.len();
    Some((idx, &questions[idx]))
}

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: worker::Context) -> worker::Result<Response> {
    let token = match env.var("TELEGRAM_BOT_TOKEN") {
        Ok(t) => t.to_string(),
        Err(_) => return Response::error("TELEGRAM_BOT_TOKEN missing", 500),
    };



    let monitored_groups: HashSet<i64> = match env.var("MONITORED_GROUPS") {
        Ok(v) => v
            .to_string()
            .split(',')
            .filter_map(|s| s.trim().parse::<i64>().ok())
            .collect(),
        Err(_) => {
            console_log!("MONITORED_GROUPS not set — monitoring disabled");
            HashSet::new()
        }
    };

    let bot_username = match env.secret("BOT_USERNAME") {
        Ok(v) => v.to_string(),
        Err(_) => return Response::error("BOT_USERNAME missing", 500),
    };

    let questions = load_questions();
    let bot = Client::new(Executor::new(), token.trim().to_string());
    let mut req = req;

    if req.method() != Method::Post {
        return Response::ok("Sphinx bot worker is running");
    }

    let update: Update = match req.json().await {
        Ok(u) => u,
        Err(e) => {
            console_log!("JSON parse error: {:?}", e);
            return Response::error("Bad request", 400);
        }
    };

    match update.kind {
        UpdateKind::Message(msg) => {
            let chat_id = msg.chat.id;
            let msg_id = msg.id;
            let from_id = msg.from.as_ref().map(|u| u.id);

            match msg.kind {
                MessageKind::Text { text, .. } => {
                    if let Err(e) = handle_text(
                        &bot,
                        chat_id,
                        &text,
                        msg_id,
                        from_id,
                        &env,
                        &monitored_groups,
                        &questions,
                        &bot_username,
                    )
                    .await
                    {
                        console_error!("handle_text error: {:?}", e);
                    }
                }
                MessageKind::Other(raw) => {
                    if let Err(e) = handle_service_msg(
                        &bot,
                        chat_id,
                        &raw,
                        &monitored_groups,
                        &bot_username,
                        &env,
                    )
                    .await
                    {
                        console_error!("handle_service_msg error: {:?}", e);
                    }
                }
            }
        }
        UpdateKind::CallbackQuery(query) => {
            if let Err(e) = handle_callback(&bot, &query, &questions, &bot_username, &env).await {
                console_error!("handle_callback error: {:?}", e);
            }
        }
        _ => {
            console_log!("Other update kind — ignored");
        }
    }

    Response::ok("OK")
}

/// Handle text messages: commands or plain echo.
async fn handle_text(
    bot: &Client<Executor>,
    chat_id: i64,
    text: &str,
    msg_id: u32,
    from_id: Option<i64>,
    env: &Env,
    groups: &HashSet<i64>,
    questions: &[QuestionItem],
    bot_username: &str,
) -> Result<()> {
    // If in a monitored group and sender is suspended, delete message and remove user
    if groups.contains(&chat_id)
        && let Some(uid) = from_id
        && let Ok(member) = bot
            .execute(GetChatMember {
                chat_id,
                user_id: uid,
            })
            .await
        && member.get("tag").and_then(|t| t.as_str()).unwrap_or("") == "suspending"
    {
        bot.execute(DeleteMessage {
            chat_id,
            message_id: msg_id,
        })
        .await?;
        let until = Date::now().as_millis() / 1000 + 7 * 24 * 3600;
        bot.execute(BanChatMember {
            chat_id,
            user_id: uid,
            until_date: Some(until),
        })
        .await?;
        return Ok(());
    }

    if text.starts_with('/') {
        let parts: Vec<&str> = text.split_whitespace().collect();
        match parts[0] {
            "/start" => {
                let payload = parts.get(1).copied();
                handle_start(bot, chat_id, env, payload, questions, bot_username).await?;
            }
            "/help" => {
                bot.execute(SendMessage {
                        chat_id,
                        text: "🤖 Sphinx Bot\n\n组内验证流程：\n• 加入群组后，点击验证按钮\n• 私聊 bot 输入 /verify 完成验证\n\n命令：\n/start — 开始\n/help — 帮助\n/chatid — 获取当前群组 ID\n/status — 查看验证状态"
                            .to_string(),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await?;
            }
            "/verify" => handle_verify(bot, chat_id, env, bot_username).await?,
            "/chatid" => handle_chatid(bot, chat_id).await?,
            "/status" => handle_status(bot, chat_id, env).await?,
            "/unban" => {
                if chat_id > 0 {
                    bot.execute(SendMessage {
                        chat_id,
                        text: "❌ 请在群组内使用此命令。".to_string(),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await?;
                    return Ok(());
                }

                let sender_id = match from_id {
                    Some(id) => id,
                    None => return Ok(()),
                };

                let is_admin = match bot
                    .execute(GetChatMember {
                        chat_id,
                        user_id: sender_id,
                    })
                    .await
                {
                    Ok(member) => {
                        let status = member.get("status").and_then(|s| s.as_str()).unwrap_or("");
                        status == "administrator" || status == "creator"
                    }
                    Err(_) => false,
                };

                if !is_admin {
                    bot.execute(SendMessage {
                        chat_id,
                        text: "❌ 只有管理员可以使用此命令。".to_string(),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await?;
                    return Ok(());
                }

                if let Some(target_id) = parts.get(1).and_then(|s| s.parse::<i64>().ok()) {
                    bot.execute(UnbanChatMember {
                        chat_id,
                        user_id: target_id,
                        only_if_banned: Some(true),
                    })
                    .await?;
                    bot.execute(SendMessage {
                        chat_id,
                        text: format!(
                            "✅ 已解除对用户 {} 的封禁。他们现在可以重新加入群组。",
                            target_id
                        ),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await?;
                } else {
                    bot.execute(SendMessage {
                        chat_id,
                        text: "⚠️ 请提供要解封的用户 ID，例如：/unban 123456789".to_string(),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await?;
                }
            }
            _ => {
                bot.execute(SendMessage {
                    chat_id,
                    text: format!("未知命令：{}\n发送 /help 查看可用命令", parts[0]),
                    parse_mode: None,
                    entities: None,
                    reply_parameters: None,
                    reply_markup: None,
                })
                .await?;
            }
        }
        return Ok(());
    }
    Ok(())
}

/// AuthHub helpers.
async fn auth_stub(env: &Env) -> anyhow::Result<Stub> {
    Ok(env
        .durable_object("AUTH_HUB")?
        .id_from_name("global")?
        .get_stub()?)
}

async fn auth_add_pending(env: &Env, user_id: i64, group_id: i64) -> anyhow::Result<()> {
    let stub = auth_stub(env).await?;
    let url = format!(
        "https://do/add_pending?user_id={}&group_id={}",
        user_id, group_id
    );
    let req = Request::new(&url, Method::Post)?;
    stub.fetch_with_request(req).await?;
    Ok(())
}

async fn auth_remove_pending(env: &Env, user_id: i64, group_id: i64) -> anyhow::Result<()> {
    let stub = auth_stub(env).await?;
    let url = format!(
        "https://do/remove_pending?user_id={}&group_id={}",
        user_id, group_id
    );
    let req = Request::new(&url, Method::Post)?;
    stub.fetch_with_request(req).await?;
    Ok(())
}

async fn auth_get_pending(env: &Env, user_id: i64) -> anyhow::Result<Vec<i64>> {
    let stub = auth_stub(env).await?;
    let url = format!("https://do/get_pending?user_id={}", user_id);
    let req = Request::new(&url, Method::Get)?;
    let mut resp = stub.fetch_with_request(req).await?;
    let text = resp.text().await?;
    Ok(serde_json::from_str(&text)?)
}

/// Handle service messages — detect new members.
/// Schedule a Durable Object alarm to check the user after 2 minutes.
async fn schedule_user_watch(
    env: &Env,
    chat_id: i64,
    user_id: i64,
    msg_id: u32,
) -> Result<()> {
    let stub = env
        .durable_object("USER_WATCH_DO")?
        .id_from_name(&format!("watch_{}_{}", chat_id, user_id))?
        .get_stub()?;

    let url = format!(
        "https://do/schedule?chat_id={}&user_id={}&msg_id={}",
        chat_id, user_id, msg_id
    );
    let req = Request::new(&url, Method::Post)?;
    stub.fetch_with_request(req).await?;
    Ok(())
}

/// Handle service messages — detect new members.
async fn handle_service_msg(
    bot: &Client<Executor>,
    chat_id: i64,
    raw: &Value,
    monitored: &HashSet<i64>,
    bot_username: &str,
    env: &Env,
) -> Result<()> {
    if !monitored.contains(&chat_id) {
        return Ok(());
    }

    let Some(new_members) = raw.get("new_chat_members").and_then(|v| v.as_array()) else {
        return Ok(());
    };

    if new_members.is_empty() {
        return Ok(());
    }

    for member in new_members {
        let user_id = match member.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => continue,
        };

        // Skip bots
        if member
            .get("is_bot")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            console_log!("Skipping bot user {}", user_id);
            continue;
        }

        let first_name = member
            .get("first_name")
            .and_then(|v| v.as_str())
            .unwrap_or("User");
        let username = member
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Register pending auth in AuthHub
        if let Err(e) = auth_add_pending(env, user_id, chat_id).await {
            console_error!("auth_add_pending error for user {} in {}: {:?}", user_id, chat_id, e);
        }

        // Suspend new member — restrict all permissions and set tag
        let until = Date::now().as_millis() / 1000 + 3600;
        bot.execute(RestrictChatMember {
            chat_id,
            user_id,
            permissions: ChatPermissions {
                can_send_messages: Some(false),
                can_send_audios: Some(false),
                can_send_documents: Some(false),
                can_send_photos: Some(false),
                can_send_videos: Some(false),
                can_send_video_notes: Some(false),
                can_send_voice_notes: Some(false),
                can_send_polls: Some(false),
                can_send_other_messages: Some(false),
                can_add_web_page_previews: Some(false),
                can_change_info: Some(false),
                can_invite_users: Some(false),
                can_pin_messages: Some(false),
                can_manage_topics: Some(false),
            },
            until_date: Some(until),
            use_independent_chat_permissions: None,
        })
        .await?;
        bot.execute(SetChatMemberTag {
            chat_id,
            user_id,
            tag: Some("suspending".to_string()),
        })
        .await?;

        let display_name = if !username.is_empty() {
            format!("@{}", username)
        } else {
            first_name.to_string()
        };

        let button = InlineKeyboardButton {
            text: "🔐 验证身份".to_string(),
            kind: InlineKeyboardButtonKind::Url {
                url: format!("https://t.me/{}?start=verify_{}", bot_username, chat_id),
            },
        };
        let markup = ReplyMarkup::InlineKeyboard {
            inline_keyboard: vec![vec![button]],
        };

        let welcome_msg = bot
            .execute(SendMessage {
                chat_id,
                text: format!(
                    "👋 欢迎 {} 加入群组！\n\n请点击下方按钮进行身份验证。\nWelcome! Please verify your identity.",
                    display_name
                ),
                parse_mode: None,
                entities: None,
                reply_parameters: None,
                reply_markup: Some(markup),
            })
            .await?;

        // Schedule alarm to check user after 2 minutes
        if let Err(e) = schedule_user_watch(env, chat_id, user_id, welcome_msg.id).await {
            console_error!(
                "Failed to schedule watch for user {} in group {}: {:?}",
                user_id,
                chat_id,
                e
            );
        }
    }
    Ok(())
}

/// Verify a user in a group — unrestrict and clear tag.
async fn verify_user(bot: &Client<Executor>, group_id: i64, user_id: i64) -> Result<()> {
    bot.execute(RestrictChatMember {
        chat_id: group_id,
        user_id,
        permissions: ChatPermissions {
            can_send_messages: Some(true),
            can_send_audios: Some(true),
            can_send_documents: Some(true),
            can_send_photos: Some(true),
            can_send_videos: Some(true),
            can_send_video_notes: Some(true),
            can_send_voice_notes: Some(true),
            can_send_polls: Some(true),
            can_send_other_messages: Some(true),
            can_add_web_page_previews: Some(true),
            can_change_info: Some(true),
            can_invite_users: Some(true),
            can_pin_messages: Some(true),
            can_manage_topics: Some(true),
        },
        until_date: None,
        use_independent_chat_permissions: None,
    })
    .await?;

    bot.execute(SetChatMemberTag {
        chat_id: group_id,
        user_id,
        tag: Some(String::new()),
    })
    .await?;
    Ok(())
}

/// Handle inline button clicks (callback queries).
async fn handle_callback(
    bot: &Client<Executor>,
    query: &turingram::types::CallbackQuery,
    questions: &[QuestionItem],
    bot_username: &str,
    env: &Env,
) -> Result<()> {
    let data = match &query.data {
        Some(d) => d,
        None => return Ok(()),
    };

    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() < 2 || (parts[0] != "verify" && parts[0] != "answer" && parts[0] != "unban") {
        return Ok(());
    }

    let user_id = query.from.id;

    //
    // answer:{group_id}:{option_index} — answer to verification question
    //
    if parts[0] == "answer" {
        if parts.len() != 4 {
            return Ok(());
        }
        let group_id: i64 = match parts[1].parse() {
            Ok(id) => id,
            Err(_) => return Ok(()),
        };
        let q_idx: usize = match parts[2].parse() {
            Ok(i) => i,
            Err(_) => return Ok(()),
        };
        let opt_idx: usize = match parts[3].parse() {
            Ok(i) => i,
            Err(_) => return Ok(()),
        };

        if let Some(q) = questions.get(q_idx) {
            if q.correct.contains(&opt_idx) {
                verify_user(bot, group_id, user_id).await?;
                // Clean up AuthHub pending entry
                let _ = auth_remove_pending(env, user_id, group_id).await;
                bot.execute(AnswerCallbackQuery {
                    callback_query_id: query.id.clone(),
                    text: Some("✅ 回答正确！验证成功！".to_string()),
                    show_alert: true,
                })
                .await?;
                bot.execute(SendMessage {
                        chat_id: user_id,
                        text: "✅ 回答正确！你的限制已被解除，欢迎加入群组！\n\nCorrect answer! You are now verified and unrestricted."
                            .to_string(),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await?;
            } else {
                // Wrong answer — ban user for 1 week
                let until = Date::now().as_millis() / 1000 + 7 * 24 * 3600;
                bot.execute(BanChatMember {
                    chat_id: group_id,
                    user_id,
                    until_date: Some(until),
                })
                .await?;
                bot.execute(AnswerCallbackQuery {
                    callback_query_id: query.id.clone(),
                    text: Some("❌ 回答错误，你已被移出群组，一周后可重新验证。".to_string()),
                    show_alert: true,
                })
                .await?;
                bot.execute(SendMessage {
                        chat_id: user_id,
                        text: "回答错误，你已被移出群组，一周后可重新加入验证。\n\nWrong answer. You were removed from the group. You can rejoin and retry in one week."
                            .to_string(),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await?;

                // Notify group about the wrong answer, with an unban button
                let unban_btn = InlineKeyboardButton {
                    text: format!("🔓 解封用户 {}", user_id),
                    kind: InlineKeyboardButtonKind::CallbackData {
                        callback_data: format!("unban:{}:{}", group_id, user_id),
                    },
                };
                bot.execute(SendMessage {
                        chat_id: group_id,
                        text: format!(
                            "❌ 用户 <a href=\"tg://user?id={}\">{}</a> 验证回答错误，已被移出群组并封禁一周。",
                            user_id, user_id
                        ),
                        parse_mode: Some(ParseMode::Html),
                        entities: None,
                        reply_parameters: None,
                        reply_markup: Some(ReplyMarkup::InlineKeyboard {
                            inline_keyboard: vec![vec![unban_btn]],
                        }),
                    })
                    .await?;
            }
        }
        return Ok(());
    }

    let group_id: i64 = match parts[1].parse() {
        Ok(id) => id,
        Err(_) => return Ok(()),
    };

    //
    // unban:{group_id}:{target_id} — admin clicked unban button
    //
    if parts[0] == "unban" {
        if parts.len() != 3 {
            return Ok(());
        }
        let target_id: i64 = match parts[2].parse() {
            Ok(id) => id,
            Err(_) => return Ok(()),
        };

        let is_admin = match bot
            .execute(GetChatMember {
                chat_id: group_id,
                user_id,
            })
            .await
        {
            Ok(member) => {
                let status = member.get("status").and_then(|s| s.as_str()).unwrap_or("");
                status == "administrator" || status == "creator"
            }
            Err(_) => false,
        };

        if !is_admin {
            bot.execute(AnswerCallbackQuery {
                callback_query_id: query.id.clone(),
                text: Some("❌ 只有管理员可以执行此操作。".to_string()),
                show_alert: true,
            })
            .await?;
            return Ok(());
        }

        bot.execute(UnbanChatMember {
            chat_id: group_id,
            user_id: target_id,
            only_if_banned: Some(true),
        })
        .await?;

        bot.execute(AnswerCallbackQuery {
            callback_query_id: query.id.clone(),
            text: Some(format!("✅ 已解除对用户 {} 的封禁。", target_id)),
            show_alert: true,
        })
        .await?;

        bot.execute(SendMessage {
            chat_id: group_id,
            text: format!(
                "✅ 管理员已解除对用户 <a href=\"tg://user?id={}\">{}</a> 的封禁。",
                target_id, target_id
            ),
            parse_mode: Some(ParseMode::Html),
            entities: None,
            reply_parameters: None,
            reply_markup: None,
        })
        .await?;

        return Ok(());
    }

    //
    // verify:{group_id} — button clicked in group, redirect to private chat
    //
    bot.execute(AnswerCallbackQuery {
        callback_query_id: query.id.clone(),
        text: Some("📩 请查看私聊消息完成验证...".to_string()),
        show_alert: false,
    })
    .await?;

    let pm_button = InlineKeyboardButton {
        text: "✅ 点击验证身份".to_string(),
        kind: InlineKeyboardButtonKind::Url {
            url: format!("https://t.me/{}?start=verify_{}", bot_username, group_id),
        },
    };
    let pm_markup = ReplyMarkup::InlineKeyboard {
        inline_keyboard: vec![vec![pm_button]],
    };

    let result = bot
        .execute(SendMessage {
            chat_id: user_id,
            text: "👋 你好！\n\n请点击下方按钮完成在群组中的身份验证。\n\nPlease click the button below to verify your identity."
                .to_string(),
            parse_mode: None,
            entities: None,
            reply_parameters: None,
            reply_markup: Some(pm_markup),
        })
        .await;

    match result {
        Ok(_) => {
            console_log!("Private verify prompt sent to user {}", user_id);
        }
        Err(TgramError::Api {
            error_code: 403, ..
        }) => {
            console_log!("Cannot send to user {} (hasn't started bot)", user_id);
            bot.execute(AnswerCallbackQuery {
                callback_query_id: query.id.clone(),
                text: Some(format!(
                    "⚠️ 请先私聊 @{} 发送 /start，然后重新点击验证按钮。",
                    bot_username
                )),
                show_alert: true,
            })
            .await?;
        }
        Err(e) => {
            console_error!("Failed to send private msg: {:?}", e);
        }
    };
    Ok(())
}

/// /start in private chat — initiate user session or verify via deep-link.
async fn handle_start(
    bot: &Client<Executor>,
    chat_id: i64,
    env: &Env,
    payload: Option<&str>,
    questions: &[QuestionItem],
    bot_username: &str,
) -> Result<()> {
    // Handle deep-link verification: /start verify_{group_id}
    if let Some(payload) = payload
        && let Some(group_id_str) = payload.strip_prefix("verify_")
        && let Ok(group_id) = group_id_str.parse::<i64>()
    {
        // Check membership and tag via Telegram API instead of KV
        let mut needs_verify = false;
        let mut is_member = false;

        if let Ok(v) = bot
            .execute(GetChatMember {
                chat_id: group_id,
                user_id: chat_id,
            })
            .await
        {
            let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
            is_member = status != "left" && status != "kicked";

            let tag = v.get("tag").and_then(|t| t.as_str()).unwrap_or("");
            needs_verify = tag == "suspending";
        }

        if !is_member {
            bot.execute(SendMessage {
                chat_id,
                text: "⏳ 没有待处理的验证请求。\n\n请先加入群组，然后重新点击验证按钮。"
                    .to_string(),
                parse_mode: None,
                entities: None,
                reply_parameters: None,
                reply_markup: None,
            })
            .await?;
            return Ok(());
        }

        if !needs_verify {
            bot.execute(SendMessage {
                            chat_id,
                            text: "✅ 你已完成验证或无需验证。\n\nYou do not need verification or have already been verified."
                                .to_string(),
                            parse_mode: None,
                            entities: None,
                            reply_parameters: None,
                            reply_markup: None,
                        })
                        .await?;
            return Ok(());
        }

        if let Some((q_idx, q)) = pick_question(questions, chat_id) {
            // Send verification question
            let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
            for (i, opt) in q.options.iter().enumerate() {
                buttons.push(vec![InlineKeyboardButton {
                    text: opt.clone(),
                    kind: InlineKeyboardButtonKind::CallbackData {
                        callback_data: format!("answer:{}:{}:{}", group_id, q_idx, i),
                    },
                }]);
            }
            bot.execute(SendMessage {
                chat_id,
                text: format!(
                    "请回答以下问题以完成验证：\n\n{}\n\nPlease answer to complete verification:",
                    q.text
                ),
                parse_mode: None,
                entities: None,
                reply_parameters: None,
                reply_markup: Some(ReplyMarkup::InlineKeyboard {
                    inline_keyboard: buttons,
                }),
            })
            .await?;
        } else {
            // No question — verify immediately
            verify_user(bot, group_id, chat_id).await?;
            bot.execute(SendMessage {
                            chat_id,
                            text: "✅ 验证成功！你已在群组中完成身份验证。\n\nVerification successful! You are now verified in the group."
                                .to_string(),
                            parse_mode: None,
                            entities: None,
                            reply_parameters: None,
                            reply_markup: None,
                        })
                        .await?;
        }
        return Ok(());
    }

    // Check for pending verifications via AuthHub
    let pending_groups = auth_get_pending(env, chat_id).await.unwrap_or_default();

    if pending_groups.is_empty() {
        bot.execute(SendMessage {
                chat_id,
                text: "👋 你好！我是 Sphinx Bot。\n\n如果你刚刚加入了某个受监控的群组，请点击群组内的验证按钮，然后在此私聊中完成验证。\n\n发送 /help 查看帮助。"
                    .to_string(),
                parse_mode: None,
                entities: None,
                reply_parameters: None,
                reply_markup: None,
            })
            .await?;
        return Ok(());
    }

    // Build verification buttons for each pending group
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for &gid in &pending_groups {
        buttons.push(vec![InlineKeyboardButton {
            text: format!("✅ 验证群组 {}", gid),
            kind: InlineKeyboardButtonKind::Url {
                url: format!("https://t.me/{}?start=verify_{}", bot_username, gid),
            },
        }]);
    }

    let markup = ReplyMarkup::InlineKeyboard {
        inline_keyboard: buttons,
    };

    bot.execute(SendMessage {
        chat_id,
        text: "👋 你好！检测到你有待验证的请求，请选择要验证的群组：".to_string(),
        parse_mode: None,
        entities: None,
        reply_parameters: None,
        reply_markup: Some(markup),
    })
    .await?;
    Ok(())
}

/// /chatid — get current chat ID.
async fn handle_chatid(bot: &Client<Executor>, chat_id: i64) -> Result<()> {
    let text = if chat_id < 0 {
        format!(
            "📋 当前群组 ID：{}\n\n提示：该 ID 可用于 MONITORED_GROUPS 配置。",
            chat_id
        )
    } else {
        format!("📋 当前会话 ID：{}", chat_id)
    };
    bot.execute(SendMessage {
        chat_id,
        text,
        parse_mode: None,
        entities: None,
        reply_parameters: None,
        reply_markup: None,
    })
    .await?;
    Ok(())
}

/// /verify in private chat — check if user has pending verification.
async fn handle_verify(
    bot: &Client<Executor>,
    chat_id: i64,
    env: &Env,
    bot_username: &str,
) -> Result<()> {
    // Check pending verifications via AuthHub
    let pending_groups = auth_get_pending(env, chat_id).await.unwrap_or_default();

    if pending_groups.is_empty() {
        bot.execute(SendMessage {
            chat_id,
            text: "📭 没有待处理的验证请求。\n\n如果你刚加入群组，请点击群组内的验证按钮。"
                .to_string(),
            parse_mode: None,
            entities: None,
            reply_parameters: None,
            reply_markup: None,
        })
        .await?;
        return Ok(());
    }

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for &group_id in &pending_groups {
        buttons.push(vec![InlineKeyboardButton {
            text: format!("✅ 去验证群组 {}", group_id),
            kind: InlineKeyboardButtonKind::Url {
                url: format!("https://t.me/{}?start=verify_{}", bot_username, group_id),
            },
        }]);
    }

    bot.execute(SendMessage {
        chat_id,
        text: "你有待处理的验证请求，请点击下方按钮进行验证：".to_string(),
        parse_mode: None,
        entities: None,
        reply_parameters: None,
        reply_markup: Some(ReplyMarkup::InlineKeyboard {
            inline_keyboard: buttons,
        }),
    })
    .await?;
    Ok(())
}

/// /status in private chat — check verification status.
async fn handle_status(bot: &Client<Executor>, chat_id: i64, env: &Env) -> Result<()> {
    // Fetch pending groups via AuthHub
    let pending_groups = auth_get_pending(env, chat_id).await.unwrap_or_default();

    if pending_groups.is_empty() {
        bot.execute(SendMessage {
            chat_id,
            text: "📭 没有相关验证记录。".to_string(),
            parse_mode: None,
            entities: None,
            reply_parameters: None,
            reply_markup: None,
        })
        .await?;
        return Ok(());
    }

    let mut lines = vec!["📋 你的验证状态：".to_string()];
    for &gid in &pending_groups {
        lines.push(format!("  • 群组 {} → ⏳ 待验证", gid));
    }

    bot.execute(SendMessage {
        chat_id,
        text: lines.join("\n"),
        parse_mode: None,
        entities: None,
        reply_parameters: None,
        reply_markup: None,
    })
    .await?;
    Ok(())
}
