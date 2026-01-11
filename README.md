# Discord Thread Migrator

A Discord bot that migrates messages from one forum thread to another forum channel or existing thread.

## Features

- Migrate messages to a forum channel (bot creates new thread)
- Migrate messages to an existing thread
- Skip first N messages with `skip` option
- Include "Moved by" attribution
- Preserves message content, attachments, embeds, and author avatars
- Only migrates non-bot messages
- Role-based access control
- Prevents concurrent migrations

## Setup

```bash
cargo build --release
```

Configure environment variables:
```
DISCORD_TOKEN=your_bot_token
DISCORD_GUILD_ID=your_server_id
DISCORD_ROLE_ID=role_allowed_to_use_bot
```

## Usage

Run `/mv` from a forum thread with:

| Option | Description |
|--------|-------------|
| `channel` | Target forum channel (bot creates new thread) |
| `thread` | Existing thread to send messages to |
| `skip` | Number of messages to skip from start (default: 0) |
| `with_author` | Include "Moved by" message (default: true) |

## Examples

```
/mv channel:#general           # Create new thread in #general
/mv thread:#existing-thread    # Send to existing thread
/mv channel:#general skip:5    # Skip first 5 messages
/mv thread:#x with_author:false  # No attribution
```
