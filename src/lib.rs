use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use turingram::client::Error as TgramError;
use turingram::client::{Client, worker_0_8::Executor};
use turingram::methods::{AnswerCallbackQuery, SendMessage};
use turingram::types::{
    ChatType, InlineKeyboardButton, InlineKeyboardButtonKind, MessageKind, ReplyMarkup, True,
    Update, UpdateKind,
};
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

fn pick_question<'a>(
    questions: &'a [QuestionItem],
    user_id: i64,
) -> Option<(usize, &'a QuestionItem)> {
    if questions.is_empty() {
        return None;
    }
    let idx = (user_id as usize).wrapping_add(Date::now().as_millis() as usize) % questions.len();
    Some((idx, &questions[idx]))
}

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let token = match env.var("TELEGRAM_BOT_TOKEN") {
        Ok(t) => t.to_string(),
        Err(_) => return Response::error("TELEGRAM_BOT_TOKEN missing", 500),
    };

    let kv = match env.kv("WAIT_AUTH_KV") {
        Ok(kv) => kv,
        Err(e) => {
            console_error!("KV binding error: {:?}", e);
            return Response::error("WAIT_AUTH_KV binding missing", 500);
        }
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
                    handle_text(
                        &bot,
                        chat_id,
                        &text,
                        msg_id,
                        from_id,
                        &kv,
                        &monitored_groups,
                        &questions,
                    )
                    .await;
                }
                MessageKind::Other(raw) => {
                    handle_service_msg(&bot, chat_id, &raw, &kv, &monitored_groups).await;
                }
            }
        }
        UpdateKind::CallbackQuery(query) => {
            handle_callback(&bot, &query, &questions).await;
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
    kv: &KvStore,
    groups: &HashSet<i64>,
    questions: &[QuestionItem],
) {
    // If in a monitored group and sender is suspended, delete message and remove user
    if groups.contains(&chat_id) {
        if let Some(uid) = from_id {
            let kv_key = format!("wait_auth:{}:{}", chat_id, uid);
            if let Ok(Some(entry)) = kv.get(&kv_key).text().await {
                if entry.contains("\"pending\"") {
                    let _ = bot
                        .execute(DeleteMessage {
                            chat_id,
                            message_id: msg_id,
                        })
                        .await;
                    let until = Date::now().as_millis() / 1000 + 7 * 24 * 3600;
                    let _ = bot
                        .execute(BanChatMember {
                            chat_id,
                            user_id: uid,
                            until_date: Some(until),
                        })
                        .await;
                    return;
                }
            }
        }
    }

    if text.starts_with('/') {
        let parts: Vec<&str> = text.split_whitespace().collect();
        match parts[0] {
            "/start" => {
                let payload = parts.get(1).copied();
                handle_start(bot, chat_id, kv, payload, questions).await;
            }
            "/help" => {
                let _ = bot
                    .execute(SendMessage {
                        chat_id,
                        text: "🤖 Sphinx Bot\n\n组内验证流程：\n• 加入群组后，点击验证按钮\n• 私聊 bot 输入 /verify 完成验证\n\n命令：\n/start — 开始\n/help — 帮助\n/chatid — 获取当前群组 ID\n/status — 查看验证状态"
                            .to_string(),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await;
            }
            "/verify" => handle_verify(bot, chat_id, kv).await,
            "/chatid" => handle_chatid(bot, chat_id).await,
            "/status" => handle_status(bot, chat_id, kv).await,
            _ => {
                let _ = bot
                    .execute(SendMessage {
                        chat_id,
                        text: format!("未知命令：{}\n发送 /help 查看可用命令", parts[0]),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await;
            }
        }
        return;
    }
}

/// Handle service messages — detect new members.
async fn handle_service_msg(
    bot: &Client<Executor>,
    chat_id: i64,
    raw: &Value,
    kv: &KvStore,
    monitored: &HashSet<i64>,
) {
    if !monitored.contains(&chat_id) {
        return;
    }

    let Some(new_members) = raw.get("new_chat_members").and_then(|v| v.as_array()) else {
        return;
    };

    if new_members.is_empty() {
        return;
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

        let now_ms = Date::now().as_millis();
        let kv_key = format!("wait_auth:{}:{}", chat_id, user_id);
        let kv_value = serde_json::json!({
            "status": "pending",
            "joined_at_ms": now_ms,
            "first_name": first_name,
            "username": username,
        })
        .to_string();

        // Expire KV entry after 1 hour (3600 seconds)
        if let Err(e) = kv
            .put(&kv_key, kv_value.as_str())
            .and_then(|b| Ok(b.expiration_ttl(3600)))
            .and_then(|b| Ok(b.execute()))
        {
            console_error!("KV put error for {}: {:?}", kv_key, e);
        }

        // Suspend new member — restrict all permissions and set tag
        let until = Date::now().as_millis() / 1000 + 3600;
        let _ = bot
            .execute(RestrictChatMember {
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
            .await;
        let _ = bot
            .execute(SetChatMemberTag {
                chat_id,
                user_id,
                tag: Some("suspending".to_string()),
            })
            .await;

        let display_name = if !username.is_empty() {
            format!("@{}", username)
        } else {
            first_name.to_string()
        };

        let button = InlineKeyboardButton {
            text: "🔐 验证身份".to_string(),
            kind: InlineKeyboardButtonKind::Url {
                url: format!("https://t.me/wit_sphinx_bot?start=verify_{}", chat_id),
            },
        };
        let markup = ReplyMarkup::InlineKeyboard {
            inline_keyboard: vec![vec![button]],
        };

        let _ = bot
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
            .await;
    }
}

/// Verify a user in a group — unrestrict and clear tag.
async fn verify_user(bot: &Client<Executor>, group_id: i64, user_id: i64) {
    let _ = bot
        .execute(RestrictChatMember {
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
        .await;

    let _ = bot
        .execute(SetChatMemberTag {
            chat_id: group_id,
            user_id,
            tag: Some(String::new()),
        })
        .await;
}

/// Handle inline button clicks (callback queries).
async fn handle_callback(
    bot: &Client<Executor>,
    query: &turingram::types::CallbackQuery,
    questions: &[QuestionItem],
) {
    let data = match &query.data {
        Some(d) => d,
        None => return,
    };

    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() < 2 || (parts[0] != "verify" && parts[0] != "verify_pm" && parts[0] != "answer")
    {
        return;
    }

    let user_id = query.from.id;

    //
    // answer:{group_id}:{option_index} — answer to verification question
    //
    if parts[0] == "answer" {
        if parts.len() != 4 {
            return;
        }
        let group_id: i64 = match parts[1].parse() {
            Ok(id) => id,
            Err(_) => return,
        };
        let q_idx: usize = match parts[2].parse() {
            Ok(i) => i,
            Err(_) => return,
        };
        let opt_idx: usize = match parts[3].parse() {
            Ok(i) => i,
            Err(_) => return,
        };

        if let Some(q) = questions.get(q_idx) {
            if q.correct.contains(&opt_idx) {
                verify_user(bot, group_id, user_id).await;
                let _ = bot
                    .execute(AnswerCallbackQuery {
                        callback_query_id: query.id.clone(),
                        text: Some("✅ 回答正确！验证成功！".to_string()),
                        show_alert: true,
                    })
                    .await;
                let _ = bot
                    .execute(SendMessage {
                        chat_id: user_id,
                        text: "✅ 回答正确！你的限制已被解除，欢迎加入群组！\n\nCorrect answer! You are now verified and unrestricted."
                            .to_string(),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await;
            } else {
                // Wrong answer — ban user for 1 week
                let until = Date::now().as_millis() / 1000 + 7 * 24 * 3600;
                let _ = bot
                    .execute(BanChatMember {
                        chat_id: group_id,
                        user_id,
                        until_date: Some(until),
                    })
                    .await;
                let _ = bot
                    .execute(AnswerCallbackQuery {
                        callback_query_id: query.id.clone(),
                        text: Some("❌ 回答错误，你已被移出群组，一周后可重新验证。".to_string()),
                        show_alert: true,
                    })
                    .await;
                let _ = bot
                    .execute(SendMessage {
                        chat_id: user_id,
                        text: "回答错误，你已被移出群组，一周后可重新加入验证。\n\nWrong answer. You were removed from the group. You can rejoin and retry in one week."
                            .to_string(),
                        parse_mode: None,
                        entities: None,
                        reply_parameters: None,
                        reply_markup: None,
                    })
                    .await;
            }
        }
        return;
    }

    let group_id: i64 = match parts[1].parse() {
        Ok(id) => id,
        Err(_) => return,
    };

    //
    // verify_pm:{group_id} — button clicked in private chat, do actual verification
    //
    if parts[0] == "verify_pm" {
        let _ = bot
            .execute(AnswerCallbackQuery {
                callback_query_id: query.id.clone(),
                text: Some("⏳ 验证中...".to_string()),
                show_alert: false,
            })
            .await;

        verify_user(bot, group_id, user_id).await;
        let _ = bot
            .execute(AnswerCallbackQuery {
                callback_query_id: query.id.clone(),
                text: Some("✅ 验证成功！".to_string()),
                show_alert: true,
            })
            .await;
        let _ = bot
            .execute(SendMessage {
                chat_id: user_id,
                text: "✅ 验证成功！你已在群组中完成身份验证。\n\nVerification successful! You are now verified in the group."
                    .to_string(),
                parse_mode: None,
                entities: None,
                reply_parameters: None,
                reply_markup: None,
            })
            .await;

        return;
    }

    //
    // verify:{group_id} — button clicked in group, redirect to private chat
    //
    let _ = bot
        .execute(AnswerCallbackQuery {
            callback_query_id: query.id.clone(),
            text: Some("📩 请查看私聊消息完成验证...".to_string()),
            show_alert: false,
        })
        .await;

    let pm_button = InlineKeyboardButton {
        text: "✅ 点击验证身份".to_string(),
        kind: InlineKeyboardButtonKind::CallbackData {
            callback_data: format!("verify_pm:{}", group_id),
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
            let _ = bot
                .execute(AnswerCallbackQuery {
                    callback_query_id: query.id.clone(),
                    text: Some(
                        "⚠️ 请先私聊 @wit_sphinx_bot 发送 /start，然后重新点击验证按钮。"
                            .to_string(),
                    ),
                    show_alert: true,
                })
                .await;
        }
        Err(e) => {
            console_error!("Failed to send private msg: {:?}", e);
        }
    }
}

/// /start in private chat — initiate user session or verify via deep-link.
async fn handle_start(
    bot: &Client<Executor>,
    chat_id: i64,
    kv: &KvStore,
    payload: Option<&str>,
    questions: &[QuestionItem],
) {
    // Handle deep-link verification: /start verify_{group_id}
    if let Some(payload) = payload {
        if let Some(group_id_str) = payload.strip_prefix("verify_") {
            if let Ok(group_id) = group_id_str.parse::<i64>() {
                // Check membership via Telegram API instead of KV
                let is_member = match bot
                    .execute(GetChatMember {
                        chat_id: group_id,
                        user_id: chat_id,
                    })
                    .await
                {
                    Ok(v) => {
                        let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
                        status != "left" && status != "kicked"
                    }
                    Err(_) => false,
                };
                if !is_member {
                    let _ = bot
                        .execute(SendMessage {
                            chat_id,
                            text:
                                "⏳ 没有待处理的验证请求。\n\n请先加入群组，然后重新点击验证按钮。"
                                    .to_string(),
                            parse_mode: None,
                            entities: None,
                            reply_parameters: None,
                            reply_markup: None,
                        })
                        .await;
                    return;
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
                    let _ = bot
                        .execute(SendMessage {
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
                        .await;
                } else {
                    // No question — verify immediately
                    verify_user(bot, group_id, chat_id).await;
                    let _ = bot
                        .execute(SendMessage {
                            chat_id,
                            text: "✅ 验证成功！你已在群组中完成身份验证。\n\nVerification successful! You are now verified in the group."
                                .to_string(),
                            parse_mode: None,
                            entities: None,
                            reply_parameters: None,
                            reply_markup: None,
                        })
                        .await;
                }
                return;
            }
        }
    }

    // Check for pending verifications
    let prefix = "wait_auth:";
    let pending_keys: Vec<String> = match kv.list().prefix(prefix.to_string()).execute().await {
        Ok(r) => r
            .keys
            .iter()
            .filter(|k| {
                let parts: Vec<&str> = k.name.split(':').collect();
                parts.len() == 3 && parts[2] == chat_id.to_string()
            })
            .map(|k| k.name.clone())
            .collect(),
        Err(_) => Vec::new(),
    };

    if pending_keys.is_empty() {
        let _ = bot
            .execute(SendMessage {
                chat_id,
                text: "👋 你好！我是 Sphinx Bot。\n\n如果你刚刚加入了某个受监控的群组，请点击群组内的验证按钮，然后在此私聊中完成验证。\n\n发送 /help 查看帮助。"
                    .to_string(),
                parse_mode: None,
                entities: None,
                reply_parameters: None,
                reply_markup: None,
            })
            .await;
        return;
    }

    // Build verification buttons for each pending group
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for key in &pending_keys {
        let parts: Vec<&str> = key.split(':').collect();
        if let Some(gid) = parts.get(1) {
            buttons.push(vec![InlineKeyboardButton {
                text: format!("✅ 验证群组 {}", gid),
                kind: InlineKeyboardButtonKind::CallbackData {
                    callback_data: format!("verify_pm:{}", gid),
                },
            }]);
        }
    }

    let markup = ReplyMarkup::InlineKeyboard {
        inline_keyboard: buttons,
    };

    let _ = bot
        .execute(SendMessage {
            chat_id,
            text: "👋 你好！检测到你有待验证的请求，请选择要验证的群组：".to_string(),
            parse_mode: None,
            entities: None,
            reply_parameters: None,
            reply_markup: Some(markup),
        })
        .await;
}

/// /chatid — get current chat ID.
async fn handle_chatid(bot: &Client<Executor>, chat_id: i64) {
    let text = if chat_id < 0 {
        format!(
            "📋 当前群组 ID：{}\n\n提示：该 ID 可用于 MONITORED_GROUPS 配置。",
            chat_id
        )
    } else {
        format!("📋 当前会话 ID：{}", chat_id)
    };
    let _ = bot
        .execute(SendMessage {
            chat_id,
            text,
            parse_mode: None,
            entities: None,
            reply_parameters: None,
            reply_markup: None,
        })
        .await;
}

/// /verify in private chat — check if user has pending verification.
async fn handle_verify(bot: &Client<Executor>, chat_id: i64, kv: &KvStore) {
    // Check all wait_auth entries for this user across groups
    // We need to list KV with prefix
    let prefix = format!("wait_auth:");

    let list_resp = match kv.list().prefix(prefix.clone()).execute().await {
        Ok(r) => r,
        Err(e) => {
            console_error!("KV list error: {:?}", e);
            let _ = bot
                .execute(SendMessage {
                    chat_id,
                    text: "❌ 系统错误，请稍后再试。".to_string(),
                    parse_mode: None,
                    entities: None,
                    reply_parameters: None,
                    reply_markup: None,
                })
                .await;
            return;
        }
    };

    // Filter to find entries for this user
    let pending_entries: Vec<&str> = list_resp
        .keys
        .iter()
        .filter(|k| {
            // key format: wait_auth:{group_id}:{user_id}
            let parts: Vec<&str> = k.name.split(':').collect();
            parts.len() == 3 && parts[0] == "wait_auth" && parts[2] == chat_id.to_string()
        })
        .map(|k| k.name.as_str())
        .collect();

    if pending_entries.is_empty() {
        let _ = bot
            .execute(SendMessage {
                chat_id,
                text: "📭 没有待处理的验证请求。\n\n如果你刚加入群组，请点击群组内的验证按钮。"
                    .to_string(),
                parse_mode: None,
                entities: None,
                reply_parameters: None,
                reply_markup: None,
            })
            .await;
        return;
    }

    // Trigger verification for each pending entry
    for key in &pending_entries {
        let parts: Vec<&str> = key.split(':').collect();
        let group_id: i64 = parts[1].parse().unwrap_or(0);

        verify_user(bot, group_id, chat_id).await;
        let _ = bot
            .execute(SendMessage {
                chat_id,
                text: format!("✅ 你已在群组 {} 中完成验证！", group_id),
                parse_mode: None,
                entities: None,
                reply_parameters: None,
                reply_markup: None,
            })
            .await;
    }
}

/// /status in private chat — check verification status.
async fn handle_status(bot: &Client<Executor>, chat_id: i64, kv: &KvStore) {
    let prefix = "wait_auth:";
    let list_resp = match kv.list().prefix(prefix.to_string()).execute().await {
        Ok(r) => r,
        Err(e) => {
            console_error!("KV list error: {:?}", e);
            return;
        }
    };

    let user_entries: Vec<String> = list_resp
        .keys
        .iter()
        .filter(|k| {
            let parts: Vec<&str> = k.name.split(':').collect();
            parts.len() == 3 && parts[2] == chat_id.to_string()
        })
        .map(|k| k.name.clone())
        .collect();

    if user_entries.is_empty() {
        let _ = bot
            .execute(SendMessage {
                chat_id,
                text: "📭 没有相关验证记录。".to_string(),
                parse_mode: None,
                entities: None,
                reply_parameters: None,
                reply_markup: None,
            })
            .await;
        return;
    }

    let mut lines = vec!["📋 你的验证状态：".to_string()];
    for key in &user_entries {
        let parts: Vec<&str> = key.split(':').collect();
        let gid = parts.get(1).unwrap_or(&"?");

        if let Ok(Some(val)) = kv.get(key).text().await {
            let status = if val.contains("\"verified\"") {
                "✅ 已验证"
            } else {
                "⏳ 待验证"
            };
            lines.push(format!("  • 群组 {} → {}", gid, status));
        }
    }

    let _ = bot
        .execute(SendMessage {
            chat_id,
            text: lines.join("\n"),
            parse_mode: None,
            entities: None,
            reply_parameters: None,
            reply_markup: None,
        })
        .await;
}
