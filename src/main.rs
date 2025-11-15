// vim: set sw=2 ts=2:
#![feature(if_let_guard)]
#![cfg_attr(debug_assertions, allow(warnings))]
#![cfg_attr(not(debug_assertions), deny(warnings, clippy::unwrap_used))]

use std::sync::Arc;

use serenity::all::{
	Channel,
	ChannelId,
	ChannelType,
	CommandInteraction,
	Context,
	CreateAttachment,
	CreateCommand,
	CreateCommandOption,
	CreateInteractionResponseFollowup,
	CreateWebhook,
	EventHandler,
	ExecuteWebhook,
	GatewayIntents,
	GuildChannel,
	GuildId,
	Interaction,
	Message,
	MessagePagination,
	PartialChannel,
	Ready,
	ResolvedOption,
	ResolvedValue,
	RoleId,
};
use serenity::async_trait;
use tokio::sync::RwLock;

const COMMAND_NAMES: &[&str] = &["mv"];
const MVT_MIGRATOR: &'static str = "MVT_MIGRATOR";

struct Handler {
	in_progress: Arc<RwLock<bool>>,
	guild_id: RwLock<GuildId>,
	role_id: RwLock<RoleId>,
}

async fn move_thread_to_forum_channel(ctx: &Context, command: &CommandInteraction, channel: &PartialChannel) -> String {
	let Some(ref source_channel) = command.channel else {
		return "No channel found".to_string();
	};

	let target_channel_id = channel.id;

	let wh = if let Some(wh) = target_channel_id
		.webhooks(ctx)
		.await
		.unwrap_or_default()
		.into_iter()
		.find(|e| e.name.as_ref().map(|e| e.as_str()).unwrap_or_default() == MVT_MIGRATOR)
	{
		wh
	} else {
		let webhook = CreateWebhook::new(MVT_MIGRATOR).name(MVT_MIGRATOR);

		target_channel_id.create_webhook(ctx, webhook).await.unwrap()
	};

	let mut messages = get_messages(ctx, source_channel.id).await.unwrap().into_iter();
	let messages_count = messages.len();

	let thread = {
		let message = { messages.next() };

		let Some(first_message) = message else {
			return "No messages found".to_string();
		};

		let (username, display_name) = {
			if let Some(ref member) = first_message.author.member
				&& let Some(ref member) = member.user
			{
				(member.name.to_string(), member.display_name().to_string())
			} else {
				let user = first_message.author.clone();
				(user.name.to_string(), user.display_name().to_string())
			}
		};

		let content = format!(
			r#"
{}
||OP: <@{}>||
"#,
			first_message.content, first_message.author.id
		);

		let mut files = vec![];

		for a in first_message.attachments {
			let Ok(attachment) = CreateAttachment::url(&ctx, &a.url).await else {
				continue;
			};
			files.push(attachment);
		}

		let ex = ExecuteWebhook::new()
			.thread_name(source_channel.name.clone().unwrap_or_else(|| "Thread".to_string()))
			.content(content)
			.embeds(first_message.embeds.into_iter().map(|e| e.into()).collect::<Vec<_>>())
			.username(format!("{} - ({})", display_name, username))
			.add_files(files)
			.avatar_url(first_message.author.avatar_url().unwrap());
		let x = wh.execute(ctx, true, ex).await.unwrap();

		let Some(message) = x else {
			return "No more messages found".to_string();
		};

		message.channel_id
	};

	{
		for message in messages {
			let (username, display_name) = {
				if let Some(ref member) = message.author.member
					&& let Some(ref member) = member.user
				{
					(&member.name.as_str(), member.display_name())
				} else {
					let user = &message.author;
					(&user.name.as_str(), user.display_name())
				}
			};

			if message.content.trim().is_empty() {
				continue;
			}

			let content = format!(
				r#"
{}
"#,
				message.content
			);

			let mut files = vec![];

			for a in message.attachments {
				let Ok(attachment) = CreateAttachment::url(&ctx, &a.url).await else {
					continue;
				};
				files.push(attachment);
			}

			let ex = ExecuteWebhook::new()
				.in_thread(thread)
				.content(content)
				.username(format!("{} - ({})", display_name, username))
				.embeds(message.embeds.into_iter().map(|e| e.into()).collect::<Vec<_>>())
				.avatar_url(message.author.avatar_url().unwrap());
			wh.execute(ctx, false, ex).await.unwrap();
		}
	}

	format!("Done sending {}", messages_count).to_string()
}

pub async fn get_messages(ctx: &Context, channel_id: ChannelId) -> Result<Vec<Message>, serenity::Error> {
	let mut page: Option<MessagePagination> = None;

	let mut out_messages: Vec<Message> = vec![];

	while let Ok(messages) = ctx.http.get_messages(channel_id, page, Some(100)).await {
		if messages.is_empty() {
			break;
		}

		out_messages.extend(messages.iter().filter(|e| !e.author.bot).cloned().collect::<Vec<_>>());

		match messages.last() {
			Some(message) => {
				page = Some(MessagePagination::Before(message.id));
			}
			None => break,
		}
	}

	out_messages.reverse();
	Ok(out_messages)
}

pub async fn move_thread(ctx: &Context, command: &CommandInteraction, options: &[ResolvedOption<'_>]) -> String {
	let Some(ref channel) = command.channel else {
		return "No channel".to_string();
	};

	match channel {
		PartialChannel {
			kind: ChannelType::PublicThread,
			parent_id: Some(parent_id),
			..
		} if let Ok(Channel::Guild(parent)) = parent_id.to_channel(ctx).await
			&& parent.kind == ChannelType::Forum => {}
		_ => return "Invoke the command from thread".to_string(),
	}

	let option = options.iter().find(|o| o.name == "channel");
	if let Some(option) = option {
		match option.value {
			ResolvedValue::Channel(target_channel) if target_channel.kind == ChannelType::Forum => {
				move_thread_to_forum_channel(ctx, command, target_channel).await
			}
			_ => "Not supported channel".to_string(),
		}
	} else {
		"No channel".to_string()
	}
}

#[async_trait]
impl EventHandler for Handler {
	async fn ready(&self, ctx: Context, ready: Ready) {
		println!("Logged in as {}", ready.user.name);

		let guild = { *self.guild_id.read().await };

		let commands = guild.get_commands(&ctx).await.unwrap_or(vec![]);

		for command in COMMAND_NAMES {
			let Some(registered_command) = commands.iter().find(|c| c.name.as_str() == *command) else {
				continue;
			};

			if (ctx.http.delete_global_command(registered_command.id).await).is_err() {
				guild.delete_command(&ctx, registered_command.id).await.ok();
			}
		}

		guild
			.set_commands(
				&ctx,
				vec![
					CreateCommand::new("mv")
						.add_option(CreateCommandOption::new(
							serenity::all::CommandOptionType::Channel,
							"channel",
							"The target channel you want to move this thread to",
						))
						.description("Move a thread to another channel"),
				],
			)
			.await
			.expect("Could not set commands");
	}

	async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
		match interaction {
			Interaction::Command(command) => {
				command.defer(&ctx).await.ok();
				let content = (async || match command.data.name.as_str() {
					"mv" => {
						let allowed_role_id = { *self.role_id.read().await };

						if let Some(ref member) = command.member
							&& !member.roles.iter().any(|e| e == &allowed_role_id)
						{
							return Some("Not allowed".to_string());
						}

						let already = {
							let mut in_progress = self.in_progress.write().await;
							if *in_progress {
								true
							} else {
								*in_progress = true;
								false
							}
						};

						if already {
							return Some("Already processing".to_string());
						}

						let s = move_thread(&ctx, &command, &command.data.options()).await;

						{
							let mut in_progress = self.in_progress.write().await;
							*in_progress = false;
						}

						Some(s)
					}
					_ => Some("not implemented :(".to_string()),
				})()
				.await;

				if let Some(content) = content {
					let builder = CreateInteractionResponseFollowup::new().content(content);
					if let Err(why) = command.create_followup(&ctx.http, builder).await {
						println!("Cannot respond to slash command: {why}");
					}
				}
			}
			_ => (),
		}
	}
}

#[tokio::main]
async fn main() {
	let token = std::env::var("DISCORD_TOKEN").expect("Expected `DISCORD_TOKEN` in the environment");
	let guild_id = std::env::var("DISCORD_GUILD_ID").expect("Expected `DISCORD_GUILD_ID` in the environment");
	let role_id = std::env::var("DISCORD_ROLE_ID").expect("Expected `DISCORD_ROLE_ID` in the environment");

	let event_handler = Handler {
		in_progress: Arc::new(RwLock::new(false)),
		guild_id: RwLock::new(
			GuildId::try_from(guild_id.parse::<u64>().expect("Expect a valid guild id")).expect("Expected a valid guild id"),
		),
		role_id: RwLock::new(
			RoleId::try_from(role_id.parse::<u64>().expect("Expect a valid role id")).expect("Expected a valid role id"),
		),
	};

	let client = serenity::Client::builder(token, GatewayIntents::all())
		.event_handler(event_handler)
		.await;

	if let Err(why) = client.expect("Could not start client").start().await {
		println!("An error occurred while running the client: {:?}", why);
	}
}
