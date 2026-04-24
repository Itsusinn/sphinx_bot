# Sphinx Bot

A Telegram bot built with Rust and [turingram](https://github.com/EAimTY/turingram), running on Cloudflare Workers.

## Setup & Deployment

### Prerequisites

- Node.js & npm
- Rust & Cargo (with `wasm32-unknown-unknown` target)
- `worker-build` installed (`cargo install worker-build`)

### Local Development

1. Install JS dependencies:
   ```bash
   npm install
   ```

2. Add your Telegram Bot Token for local dev:
   ```bash
   echo "TELEGRAM_BOT_TOKEN=your-token" > .dev.vars
   ```

3. Run locally:
   ```bash
   npm run dev
   ```
   *You can test it by sending a POST request to the local URL with a mock Telegram Update JSON.*

### Deployment to Cloudflare

1. Login to Wrangler:
   ```bash
   npx wrangler login
   ```

2. Add your Telegram Bot Token as a secret:
   ```bash
   npx wrangler secret put TELEGRAM_BOT_TOKEN
   ```

3. Deploy:
   ```bash
   npm run deploy
   ```

4. Set your Telegram Bot Webhook to the deployed Cloudflare Worker URL:
   ```bash
   curl "https://api.telegram.org/bot<YOUR_TOKEN>/setWebhook?url=<YOUR_WORKER_URL>"
   ```
