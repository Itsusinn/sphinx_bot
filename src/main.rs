use anyhow::Result;
use dotenvy::dotenv;
use std::env;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
use turingram::client::Client;
use turingram::types::UpdateKind;
use turingram::methods::SendMessage;

mod client;
mod methods;

use methods::get_updates::GetUpdates;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    info!("Starting Sphinx Bot...");

    let token = env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN must be set");
    let executor = client::reqwest::Executor::new();
    let bot_client = Client::new(executor, token);
    
    let mut offset: Option<u32> = None;
    
    loop {
        let req = GetUpdates {
            offset,
            limit: Some(10),
            timeout: Some(10),
        };
        
        match bot_client.execute(req).await {
            Ok(updates) => {
                for update in updates {
                    offset = Some(update.id + 1);
                    match update.kind {
                        UpdateKind::Message(message) => {
                            if let turingram::types::MessageKind::Text { text, .. } = message.kind {
                                info!("Received text: {}", text);
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
                }
            }
            Err(e) => {
                tracing::error!("Error getting updates: {:?}", e);
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }
    }
}