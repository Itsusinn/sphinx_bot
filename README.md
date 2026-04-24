# Sphinx Bot

A Telegram group verification bot built with Rust and [turingram](https://github.com/EAimTY/turingram), running on Cloudflare Workers.

New members are restricted (suspended) upon joining and must pass a verification question to gain full access. Wrong answers result in a 1-week ban.

## Configuration

### Secrets (set via `wrangler secret`)

| Secret | Required | Description |
|--------|----------|-------------|
| `TELEGRAM_BOT_TOKEN` | ✅ | Bot token from [@BotFather](https://t.me/botfather) |
| `MONITORED_GROUPS` | ✅ | Comma-separated group IDs to monitor (e.g. `-1001234567890,-1009876543210`) |

Set a secret:
```powershell
# PowerShell
'-1001234567890' | npx wrangler secret put MONITORED_GROUPS
```
```bash
# Bash
echo "-1001234567890" | npx wrangler secret put MONITORED_GROUPS
```

### Question Bank (`config.toml`)

Edit `config.toml` in the project root to configure verification questions. It is embedded at compile time via `include_bytes!`.

```toml
[[question]]
text = "What is the capital of France?"
options = ["London", "Paris", "Berlin", "Madrid"]
correct = [1]

[[question]]
text = "Which are programming languages?"
options = ["Python", "HTML", "Rust", "CSS", "JavaScript"]
correct = [0, 2, 4]
```

- `correct` accepts an **array** of indices (0-based), supporting multiple correct answers.
- A random question is selected for each verification attempt.
- After modifying `config.toml`, **rebuild and redeploy** for changes to take effect.

### Group ID

Send `/chatid` in your group to get its ID for `MONITORED_GROUPS`.

## Setup

### Prerequisites

- Node.js & npm
- Rust & Cargo (with `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`)
- `worker-build` (`cargo install worker-build`)

### 1. Install Dependencies

```bash
npm install
```

### 2. Deploy to Cloudflare

```bash
# Login
npx wrangler login

# Set secrets
npx wrangler secret put TELEGRAM_BOT_TOKEN
npx wrangler secret put MONITORED_GROUPS

# Deploy
npm run deploy
```

### 3. Set Webhook

After deploying, set the Telegram webhook to your Worker URL:

```bash
curl -X POST "https://api.telegram.org/bot<TELEGRAM_BOT_TOKEN>/setWebhook" \
  -d "url=https://sphinx-bot.your-subdomain.workers.dev"
```

Verify it's working:
```bash
curl "https://api.telegram.org/bot<TELEGRAM_BOT_TOKEN>/getWebhookInfo"
```

### 4. Grant Bot Admin Permissions

Add the bot as an administrator in your group with these permissions:

- **Restrict members** (`can_restrict_members`)
- **Manage tags** (`can_manage_tags`)
- **Delete messages** (optional, for auto-delete of suspended user messages)

### Local Development

```bash
echo "TELEGRAM_BOT_TOKEN=your-token" > .dev.vars
npm run dev
```

## Architecture

```
User joins group
  → restricted + tag = "suspending"
  → welcome message with button
  → user clicks → opens bot private chat via /start verify_{group_id}
  → bot asks random question from config.toml
  → correct answer → unrestrict + clear tag
  → wrong answer → banned for 1 week
```
