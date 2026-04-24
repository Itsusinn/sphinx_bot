use turingram::client::{Client, worker_0_8::Executor};
use turingram::methods::SendMessage;
use turingram::types::{Update, UpdateKind, MessageKind};
use worker::*;

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let token = match env.var("TELEGRAM_BOT_TOKEN") {
        Ok(t) => t.to_string(),
        Err(_) => return Response::error("TELEGRAM_BOT_TOKEN missing", 500),
    };
    
    let bot_client = Client::new(Executor::new(), token);
    let mut req = req;
    
    if req.method() != Method::Post {
        return Response::ok("Sphinx bot worker is running");
    }
    
    let update: Update = match req.json().await {
        Ok(u) => u,
        Err(_) => return Response::error("Bad request", 400),
    };
    
    match update.kind {
        UpdateKind::Message(message) => {
            if let MessageKind::Text { text, .. } = message.kind {
                console_log!("Received text: {}", text);
                let reply = SendMessage {
                    chat_id: message.chat.id,
                    text: format!("Echo: {}", text),
                    parse_mode: None,
                    entities: None,
                    reply_parameters: None,
                    reply_markup: None,
                };
                let _ = bot_client.execute(reply).await;
            }
        }
        _ => {}
    }
    
    Response::ok("OK")
}
