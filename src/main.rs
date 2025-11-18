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
	CreateAllowedMentions,
	CreateAttachment,
	CreateCommand,
	CreateCommandOption,
	CreateInteractionResponseFollowup,
	CreateWebhook,
	EventHandler,
	ExecuteWebhook,
	GatewayIntents,
	GuildId,
	Interaction,
	Mentionable,
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
const MVT_MIGRATOR: &str = "MVT_MIGRATOR";

struct Handler {
	in_progress: Arc<RwLock<bool>>,
	guild_id: RwLock<GuildId>,
	role_id: RwLock<RoleId>,
}

enum Error {
	NoChannel,
	NoMessages,
	UnableToRetieveMessages(String),
	UnableToSendMessage(String),
	UnableToCreateWebhook(String),
	NotAllowed,
	AlreadyProcessing,
	NotImplemented,
}

impl std::fmt::Display for Error {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Error::NoChannel => write!(f, "No channel found"),
			Error::NoMessages => write!(f, "No messages found"),
      Error::UnableToRetieveMessages(why) => write!(f, "Unable to retrieve the messages: {why}"),
			Error::UnableToSendMessage(why) => write!(f, "Unable to send the message: {why}"),
			Error::UnableToCreateWebhook(why) => write!(f, "Unable to create the webhook: {why}"),
			Error::NotAllowed => write!(f, "Not allowed"),
			Error::AlreadyProcessing => write!(f, "Already processing"),
			Error::NotImplemented => write!(f, "Not implemented :("),
		}
	}
}

async fn move_thread_to_forum_channel(
	ctx: &Context,
	command: &CommandInteraction,
	channel: &PartialChannel,
) -> Result<(), Error> {
	let Some(ref source_channel) = command.channel else {
		return Err(Error::NoChannel);
	};

	let target_channel_id = channel.id;

	let wh = if let Some(wh) = target_channel_id
		.webhooks(ctx)
		.await
		.unwrap_or_default()
		.into_iter()
		.find(|e| e.name.as_deref().unwrap_or_default() == MVT_MIGRATOR)
	{
		wh
	} else {
		let webhook = CreateWebhook::new(MVT_MIGRATOR).name(MVT_MIGRATOR);

		target_channel_id
			.create_webhook(ctx, webhook)
			.await
			.map_err(|e| Error::UnableToCreateWebhook(e.to_string()))?
	};

	let mut messages = get_messages(ctx, source_channel.id)
		.await
		.map(std::iter::IntoIterator::into_iter)
		.map_err(|e| Error::UnableToRetieveMessages(e.to_string()))?;

	let thread = {
		let message = { messages.next() };

		let Some(first_message) = message else {
			return Err(Error::NoMessages);
		};

		let (username, display_name) = {
			if let Some(ref member) = first_message.author.member
				&& let Some(ref member) = member.user
			{
				(member.name.as_str(), member.display_name())
			} else {
				let user = &first_message.author;
				(user.name.as_str(), user.display_name())
			}
		};

		let content = format!(
			r"
{}
||OP: <@{}>||
",
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
			.allowed_mentions(CreateAllowedMentions::new().empty_users().empty_roles())
			.embeds(
				first_message
					.embeds
					.into_iter()
					.map(std::convert::Into::into)
					.collect::<Vec<_>>(),
			)
			.username(format!("{display_name} - ({username})"))
			.add_files(files);

		let ex = if let Some(avatar) = first_message.author.avatar_url() {
			ex.avatar_url(avatar)
		} else {
			ex
		};

		let Ok(x) = wh.execute(ctx, true, ex).await else {
			return Err(Error::UnableToSendMessage(
				"Unable to send the first message".to_string(),
			));
		};

		let Some(message) = x else {
			return Err(Error::NoMessages);
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

			let mut files = vec![];

			for a in message.attachments {
				let Ok(attachment) = CreateAttachment::url(&ctx, &a.url).await else {
					continue;
				};
				files.push(attachment);
			}

			let ex = ExecuteWebhook::new()
				.in_thread(thread)
				.content(message.content)
				.username(format!("{display_name} - ({username})"))
				.allowed_mentions(CreateAllowedMentions::new().empty_users().empty_roles())
				.add_files(files)
				.embeds(
					message
						.embeds
						.into_iter()
						.map(std::convert::Into::into)
						.collect::<Vec<_>>(),
				);

			let ex = if let Some(avatar) = message.author.avatar_url() {
				ex.avatar_url(avatar)
			} else {
				ex
			};

			wh.execute(ctx, false, ex).await.ok();
		}
	}

	Ok(())
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
	match command.channel {
		Some(PartialChannel {
			kind: ChannelType::PublicThread,
			parent_id: Some(parent_id),
			..
		}) if let Ok(Channel::Guild(parent)) = parent_id.to_channel(ctx).await
			&& parent.kind == ChannelType::Forum => {}
		_ => return "Invoke the command from a forum thread".to_string(),
	}

	let option = options.iter().find(|o| o.name == "channel");
	match option {
		Some(ResolvedOption {
			value:
				ResolvedValue::Channel(
					target_channel @ PartialChannel {
						kind: ChannelType::Forum,
						..
					},
				),
			..
		}) => match move_thread_to_forum_channel(ctx, command, target_channel).await {
			Ok(_) => format!("Message sent to {}", target_channel.id.mention()),
			Err(error) => error.to_string(),
		},
		_ => "Target channel is not a forum".to_string(),
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
		if let Interaction::Command(command) = interaction {
			command.defer_ephemeral(&ctx).await.ok();
			#[allow(clippy::redundant_closure_call)]
			let r = (async || -> Result<String, Error> {
				match command.data.name.as_str() {
					"mv" => {
						let allowed_role_id = { *self.role_id.read().await };

						if let Some(ref member) = command.member
							&& !member.roles.iter().any(|e| e == &allowed_role_id)
						{
							return Err(Error::NotAllowed);
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
							return Err(Error::AlreadyProcessing);
						}

						let s = move_thread(&ctx, &command, &command.data.options()).await;

						{
							let mut in_progress = self.in_progress.write().await;
							*in_progress = false;
						}

						Ok(s)
					}
					_ => Err(Error::NotImplemented),
				}
			})()
			.await;

			let content = match r {
				Ok(content) => content,
				Err(Error::NotImplemented) => return,
				Err(error) => error.to_string(),
			};

			let builder = CreateInteractionResponseFollowup::new().content(format!("{} {content}", command.user.mention()));
			if let Err(why) = command.create_followup(&ctx.http, builder).await {
				println!("Cannot respond to slash command: {why}");
			}
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
		guild_id: RwLock::new(GuildId::from(guild_id.parse::<u64>().expect("Expect a valid guild id"))),
		role_id: RwLock::new(RoleId::from(role_id.parse::<u64>().expect("Expect a valid role id"))),
	};

	let client = serenity::Client::builder(token, GatewayIntents::all())
		.event_handler(event_handler)
		.await;

	if let Err(why) = client.expect("Could not start client").start().await {
		println!("An error occurred while running the client: {why:?}");
	}
}
