use crate::config::Config;
use crate::message::{
    send_messages, ActionMessage, Message, ResponseMessage, SendMessageDestination,
};
use crate::model_delegate::ModelDelegate;
use futures::prelude::*;
use mrvn_back_ytdl::{
    Brain, EndedHandler, GuildSpeakerEndedHandle, GuildSpeakerEndedRef, GuildSpeakerRef, Song,
};
use mrvn_model::{AppModel, GuildModel, NextEntry, ReplaceStatus, VoteStatus, VoteType};
use rand::distributions::{Distribution, Uniform};
use serenity::model::id::ChannelId;
use serenity::{
    model::prelude::{application_command, interactions, GuildId, UserId},
    prelude::*,
};
use std::ops::DerefMut;
use std::sync::Arc;
use std::time::Duration;

const SEND_WORKING_TIMEOUT_MS: u64 = 50;

enum HandleCommandError {
    CreateError(crate::error::Error),
    EditError(crate::error::Error),
}

enum QueuedSongsMetadata {
    Single(mrvn_back_ytdl::SongMetadata),
    Multiple(usize),
}

pub struct Frontend {
    pub config: Arc<Config>,
    pub backend_brain: Brain,
    pub model: AppModel<Song>,
}

impl Frontend {
    pub fn new(config: Arc<Config>, backend_brain: Brain, model: AppModel<Song>) -> Frontend {
        Frontend {
            config,
            backend_brain,
            model,
        }
    }

    pub async fn handle_command(
        self: &Arc<Self>,
        ctx: &Context,
        command: &interactions::application_command::ApplicationCommandInteraction,
    ) {
        let send_error_res = match self.handle_command_fallable(ctx, command).await {
            Ok(_) => Ok(()),
            Err(HandleCommandError::CreateError(why)) => {
                log::error!("Error while handling command: {}", why);
                command
                    .create_interaction_response(&ctx.http, |response| {
                        response
                            .kind(interactions::InteractionResponseType::ChannelMessageWithSource)
                            .interaction_response_data(|data| {
                                data.create_embed(|embed| {
                                    embed
                                        .description(
                                            self.config.get_raw_message("action.unknown_error"),
                                        )
                                        .color(self.config.response_embed_color)
                                })
                            })
                    })
                    .await
                    .map(|_| ())
            }
            Err(HandleCommandError::EditError(why)) => {
                log::error!("Error while handling command: {}", why);
                command
                    .edit_original_interaction_response(&ctx.http, |response| {
                        response.create_embed(|embed| {
                            embed
                                .description(self.config.get_raw_message("action.unknown_error"))
                                .color(self.config.response_embed_color)
                        })
                    })
                    .await
                    .map(|_| ())
            }
        };

        if let Err(why) = send_error_res {
            log::error!("Error while sending error response: {}", why);
        }
    }

    async fn handle_command_fallable(
        self: &Arc<Self>,
        ctx: &Context,
        command: &interactions::application_command::ApplicationCommandInteraction,
    ) -> Result<(), HandleCommandError> {
        let guild_id = command.guild_id.ok_or(HandleCommandError::CreateError(
            crate::error::Error::NoGuild,
        ))?;
        let message_channel_id = command.channel_id;

        // This signal is used to cancel sending a "loading..." message when we finish executing
        // the command.
        let (tx, rx) = tokio::sync::oneshot::channel();
        let send_deferred_message_future = async {
            let show_deferred_message = futures::select!(
                _ = rx.fuse() => false,
                _ = tokio::time::sleep(Duration::from_millis(SEND_WORKING_TIMEOUT_MS)).fuse() => true,
            );
            if show_deferred_message {
                if let Err(why) = command
                    .create_interaction_response(&ctx.http, |response| {
                        response.kind(
                            interactions::InteractionResponseType::DeferredChannelMessageWithSource,
                        )
                    })
                    .await
                {
                    log::error!("Error while sending deferred message: {}", why);
                }
            }
        };

        let send_future = async {
            // Ensure we have the guild locked for the duration of the command.
            let guild_model_handle = self.model.get(guild_id);
            let mut guild_model = guild_model_handle.lock().await;
            guild_model.set_message_channel(Some(message_channel_id));

            // Execute the command
            let messages_res = self
                .handle_guild_command(ctx, command, guild_id, guild_model.deref_mut())
                .await;

            // If the timeout has finished, rx will be closed so this send call will return an
            // error. We can use this to know that a response has been created, and we need to edit
            // it from now on.
            let has_sent_deferred = tx.send(()).is_err();
            let messages = messages_res.map_err(if has_sent_deferred {
                HandleCommandError::EditError
            } else {
                HandleCommandError::CreateError
            })?;

            let send_res = send_messages(
                &self.config,
                ctx,
                SendMessageDestination::Interaction {
                    interaction: command,
                    is_edit: has_sent_deferred,
                },
                guild_model.deref_mut(),
                messages,
            )
            .await;
            if let Err(why) = send_res {
                log::error!("Error while sending response: {}", why);
            }

            Ok(())
        };

        let (send_res, _) = futures::join!(send_future, send_deferred_message_future);
        send_res
    }

    async fn handle_guild_command(
        self: &Arc<Self>,
        ctx: &Context,
        command: &interactions::application_command::ApplicationCommandInteraction,
        guild_id: GuildId,
        guild_model: &mut GuildModel<Song>,
    ) -> Result<Vec<crate::message::Message>, crate::error::Error> {
        let user_id = command.user.id;
        match command.data.name.as_str() {
            "play" => {
                let term = match command
                    .data
                    .options
                    .get(0)
                    .and_then(|val| val.resolved.as_ref())
                    .unwrap() {
                        application_command::ApplicationCommandInteractionDataOptionValue::String(
                            val,
                        ) => val.clone(),
                    _ => "".to_string(),
                };

                log::debug!("Received play \"{}\"", term);
                self.handle_queue_play_command(ctx, user_id, guild_id, guild_model, &term)
                    .await
            }
            "resume" => {
                log::debug!("Received resume");
                self.handle_unpause_command(ctx, user_id, guild_id, guild_model)
                    .await
            }
            "replace" => {
                let term = match command
                    .data
                    .options
                    .get(0)
                    .and_then(|val| val.resolved.as_ref())
                {
                    Some(
                        application_command::ApplicationCommandInteractionDataOptionValue::String(
                            val,
                        ),
                    ) => val.clone(),
                    _ => "".to_string(),
                };

                log::debug!("Received replace \"{}\"", term);
                self.handle_replace_command(ctx, user_id, guild_id, guild_model, &term)
                    .await
            }
            "pause" => {
                log::debug!("Received pause");
                self.handle_pause_command(ctx, user_id, guild_id).await
            }
            "skip" => {
                log::debug!("Received skip");
                self.handle_skip_command(ctx, user_id, guild_id, guild_model)
                    .await
            }
            "stop" => {
                log::debug!("Received stop");
                self.handle_stop_command(ctx, user_id, guild_id, guild_model)
                    .await
            }
            "pet" => {
                log::debug!("Received pet");
                self.handle_pet_command().await
            }
            "nowplaying" => {
                log::debug!("Received now-playing command");
                self.handle_nowplaying_command(ctx, user_id, guild_id).await
            }
            command_name => Err(crate::error::Error::UnknownCommand(
                command_name.to_string(),
            )),
        }
    }

    async fn handle_queue_play_command(
        self: &Arc<Self>,
        ctx: &Context,
        user_id: UserId,
        guild_id: GuildId,
        guild_model: &mut GuildModel<Song>,
        term: &str,
    ) -> Result<Vec<crate::message::Message>, crate::error::Error> {
        let play_config = self.config.get_play_config();

        let delegate_future = ModelDelegate::new(ctx, guild_id);
        let song_future =
            Song::load(term, user_id, &play_config).map_err(crate::error::Error::Backend);

        let (delegate, songs) = match futures::try_join!(delegate_future, song_future) {
            Ok(data) => data,
            Err(crate::error::Error::Backend(mrvn_back_ytdl::Error::UnsupportedUrl)) => {
                return Ok(vec![Message::Response(
                    ResponseMessage::UnsupportedSiteError,
                )]);
            }
            Err(why) => return Err(why),
        };
        if songs.is_empty() {
            return Ok(vec![Message::Response(
                ResponseMessage::NoMatchingSongsError,
            )]);
        }

        let metadata = if songs.len() == 1 {
            let song_metadata = &songs[0].metadata;
            log::trace!(
                "Resolved song query as {} (\"{}\")",
                song_metadata.url,
                song_metadata.title
            );
            QueuedSongsMetadata::Single(song_metadata.clone())
        } else {
            log::trace!("Resolved song query as {} songs", songs.len());
            QueuedSongsMetadata::Multiple(songs.len())
        };

        guild_model.push_entries(user_id, songs);

        // From this point on the user needs to be in a channel, otherwise the songs will only stay
        // queued.
        let channel_id = match delegate.get_user_voice_channel(user_id) {
            Some(channel) => channel,
            None => {
                log::trace!("User is not in any voice channel, song will remain queued");
                return Ok(vec![Message::Response(match metadata {
                    QueuedSongsMetadata::Single(song_metadata) => ResponseMessage::Queued {
                        song_title: song_metadata.title,
                        song_url: song_metadata.url,
                    },
                    QueuedSongsMetadata::Multiple(count) => {
                        ResponseMessage::QueuedMultiple { count }
                    }
                })]);
            }
        };

        // Find a speaker that will be able to play in this channel. We do this before checking if
        // we actually need to play anything so the song can stay in the queue if a speaker isn't
        // found.
        let guild_speakers_handle = self.backend_brain.guild_speakers(guild_id);
        let mut guild_speakers_ref = guild_speakers_handle.lock().await;
        let guild_speaker = match guild_speakers_ref.find_to_play_in_channel(channel_id) {
            Some(speaker) => speaker,
            None => {
                log::trace!(
                    "No speakers are available to handle playback, song will remain queued"
                );
                return Ok(vec![Message::Response(match metadata {
                    QueuedSongsMetadata::Single(song_metadata) => {
                        ResponseMessage::QueuedNoSpeakers {
                            song_title: song_metadata.title,
                            song_url: song_metadata.url,
                        }
                    }
                    QueuedSongsMetadata::Multiple(count) => {
                        ResponseMessage::QueuedMultipleNoSpeakers { count }
                    }
                })]);
            }
        };

        // Play a song if the model indicates one isn't playing.
        let next_song = match guild_model.next_channel_entry(&delegate, channel_id) {
            NextEntry::Entry(song) => song,
            NextEntry::AlreadyPlaying | NextEntry::NoneAvailable => {
                log::trace!("Channel is already playing, song will remain queued");
                return Ok(vec![Message::Response(match metadata {
                    QueuedSongsMetadata::Single(song_metadata) => ResponseMessage::Queued {
                        song_title: song_metadata.title,
                        song_url: song_metadata.url,
                    },
                    QueuedSongsMetadata::Multiple(count) => {
                        ResponseMessage::QueuedMultiple { count }
                    }
                })]);
            }
        };

        let next_metadata = next_song.metadata.clone();
        self.play_to_speaker(ctx, guild_model, guild_speaker, channel_id, next_song)
            .await?;

        // We could be in one of three states:
        //  - One song was queued, and we're now playing that song. We only show a "playing"
        //    message.
        //  - Multiple songs were queued, and we're playing the first one. We show a "queued"
        //    message and a "playing" message.
        //    todo: maybe we should combine these in this case
        // - We queued one or more songs and started a different song, which can happen if there
        //   were other songs waiting but we weren't playing at the time.
        match metadata {
            QueuedSongsMetadata::Single(song_metadata) => {
                if next_metadata.url == song_metadata.url {
                    Ok(vec![Message::Action(ActionMessage::PlayingResponse {
                        song_title: song_metadata.title,
                        song_url: song_metadata.url,
                        voice_channel_id: channel_id,
                    })])
                } else {
                    Ok(vec![
                        Message::Response(ResponseMessage::Queued {
                            song_title: song_metadata.title,
                            song_url: song_metadata.url,
                        }),
                        Message::Action(ActionMessage::Playing {
                            song_title: next_metadata.title,
                            song_url: next_metadata.url,
                            voice_channel_id: channel_id,
                            user_id: next_metadata.user_id,
                        }),
                    ])
                }
            }
            QueuedSongsMetadata::Multiple(count) => Ok(vec![
                Message::Response(ResponseMessage::QueuedMultiple { count }),
                Message::Action(ActionMessage::Playing {
                    song_title: next_metadata.title,
                    song_url: next_metadata.url,
                    voice_channel_id: channel_id,
                    user_id: next_metadata.user_id,
                }),
            ]),
        }
    }

    async fn handle_unpause_command(
        self: &Arc<Self>,
        ctx: &Context,
        user_id: UserId,
        guild_id: GuildId,
        guild_model: &mut GuildModel<Song>,
    ) -> Result<Vec<crate::message::Message>, crate::error::Error> {
        let delegate = ModelDelegate::new(ctx, guild_id).await?;
        let channel_id = match delegate.get_user_voice_channel(user_id) {
            Some(channel) => channel,
            None => {
                return Ok(vec![Message::Response(
                    ResponseMessage::NotInVoiceChannelError,
                )])
            }
        };

        // See if there's currently a speaker in this channel to unpause.
        let guild_speakers_handle = self.backend_brain.guild_speakers(guild_id);
        let mut guild_speakers_ref = guild_speakers_handle.lock().await;
        if let Some((guild_speaker, active_metadata)) =
            guild_speakers_ref.find_active_in_channel(channel_id)
        {
            return if guild_speaker.is_paused() {
                log::trace!(
                    "Found a paused speaker in the user's voice channel, starting playback"
                );
                guild_speaker
                    .unpause()
                    .map_err(crate::error::Error::Backend)?;
                Ok(vec![Message::Action(ActionMessage::Playing {
                    song_title: active_metadata.title.clone(),
                    song_url: active_metadata.url.clone(),
                    voice_channel_id: channel_id,
                    user_id: active_metadata.user_id,
                })])
            } else {
                log::trace!(
                    "Found an unpaused speaker in the user's voice channel, playback will continue"
                );
                Ok(vec![Message::Response(
                    ResponseMessage::AlreadyPlayingError {
                        voice_channel_id: channel_id,
                    },
                )])
            };
        };

        // Otherwise, try starting to play in this channel.
        let guild_speaker = match guild_speakers_ref.find_to_play_in_channel(channel_id) {
            Some(speaker) => speaker,
            None => {
                log::trace!("No speakers are available to handle playback, nothing will be played");
                return Ok(vec![Message::Action(ActionMessage::NoSpeakersError {
                    voice_channel_id: channel_id,
                })]);
            }
        };
        let next_song = match guild_model.next_channel_entry(&delegate, channel_id) {
            NextEntry::Entry(song) => song,
            NextEntry::AlreadyPlaying | NextEntry::NoneAvailable => {
                log::trace!(
                    "No songs are available to play back in the channel, nothing will be played"
                );
                return Ok(vec![Message::Response(
                    ResponseMessage::NothingIsQueuedError {
                        voice_channel_id: channel_id,
                    },
                )]);
            }
        };

        let next_metadata = next_song.metadata.clone();
        self.play_to_speaker(ctx, guild_model, guild_speaker, channel_id, next_song)
            .await?;

        Ok(vec![Message::Action(ActionMessage::Playing {
            song_title: next_metadata.title,
            song_url: next_metadata.url,
            voice_channel_id: channel_id,
            user_id: next_metadata.user_id,
        })])
    }

    async fn handle_replace_command(
        self: &Arc<Self>,
        ctx: &Context,
        user_id: UserId,
        guild_id: GuildId,
        guild_model: &mut GuildModel<Song>,
        term: &str,
    ) -> Result<Vec<crate::message::Message>, crate::error::Error> {
        let play_config = self.config.get_play_config();

        let delegate_future = ModelDelegate::new(ctx, guild_id);
        let song_future =
            Song::load(term, user_id, &play_config).map_err(crate::error::Error::Backend);

        let (delegate, songs) = match futures::try_join!(delegate_future, song_future) {
            Ok(data) => data,
            Err(crate::error::Error::Backend(mrvn_back_ytdl::Error::UnsupportedUrl)) => {
                return Ok(vec![Message::Response(
                    ResponseMessage::UnsupportedSiteError,
                )]);
            }
            Err(why) => return Err(why),
        };

        if songs.len() == 1 {
            let song_metadata = &songs[0].metadata;
            log::trace!(
                "Resolved song query as {} (\"{}\")",
                song_metadata.url,
                song_metadata.title
            );
        } else {
            log::trace!("Resolved song query as {} songs", songs.len());
        }

        let mut songs_iter = songs.into_iter();
        let song = match songs_iter.next() {
            Some(song) => song,
            None => {
                return Ok(vec![Message::Response(
                    ResponseMessage::NoMatchingSongsError,
                )])
            }
        };

        let song_metadata = song.metadata.clone();
        let maybe_channel_id = delegate.get_user_voice_channel(user_id);
        let replace_status = guild_model.replace_entry(user_id, maybe_channel_id, song);
        guild_model.push_entries(user_id, songs_iter);

        let channel_id = match replace_status {
            // If the song was queued, no playback changes are needed so we send a status message
            // and leave it there. But if the model indicated we're replacing the current song,
            // we need to start playing the next song.
            ReplaceStatus::Queued => {
                log::trace!("No songs in queue to replace, song will be queued");
                return Ok(vec![Message::Response(ResponseMessage::Queued {
                    song_title: song_metadata.title,
                    song_url: song_metadata.url,
                })]);
            }
            ReplaceStatus::ReplacedInQueue(old_song) => {
                log::trace!("Latest song in the users queue will be replaced");
                return Ok(vec![Message::Response(ResponseMessage::Replaced {
                    old_song_title: old_song.metadata.title,
                    old_song_url: old_song.metadata.url,
                    new_song_title: song_metadata.title,
                    new_song_url: song_metadata.url,
                })]);
            }
            ReplaceStatus::ReplacedCurrent(channel_id) => channel_id,
        };

        log::trace!("Only song queued by user is currently playing, it will be skipped");

        // We're replacing an already-playing song, so if there's no speaker for this channel
        // something has gone very wrong :(
        let guild_speakers_handle = self.backend_brain.guild_speakers(guild_id);
        let mut guild_speakers_ref = guild_speakers_handle.lock().await;
        let (guild_speaker, playing_metadata) = guild_speakers_ref
            .find_active_in_channel(channel_id)
            .ok_or(crate::error::Error::ModelPlayingSpeakerNotDesync)?;

        // Play a song if the model indicates one isn't playing.
        let next_song = match guild_model.next_channel_entry_finished(&delegate, channel_id) {
            Some(song) => song,
            None => {
                log::trace!("New song is no longer accessible in queue, nothing will play");
                return Ok(vec![Message::Response(
                    ResponseMessage::NothingIsQueuedError {
                        voice_channel_id: channel_id,
                    },
                )]);
            }
        };

        let next_metadata = next_song.metadata.clone();
        self.play_to_speaker(ctx, guild_model, guild_speaker, channel_id, next_song)
            .await?;

        // We could be in one of two states:
        //  - The song that's now playing is the one we just queued, in which case we only show a
        //    "playing" message.
        //  - We queued a song and started a different song, which can happen if there were other
        //    songs waiting but we weren't playing at the time. In this case we show a "queued"
        //    message and a "playing" message.
        if next_metadata.url == song_metadata.url {
            Ok(vec![Message::Action(ActionMessage::PlayingResponse {
                song_title: song_metadata.title,
                song_url: song_metadata.url,
                voice_channel_id: channel_id,
            })])
        } else {
            Ok(vec![
                Message::Response(ResponseMessage::ReplaceSkipped {
                    new_song_title: song_metadata.title,
                    new_song_url: song_metadata.url,
                    old_song_title: playing_metadata.title,
                    old_song_url: playing_metadata.url,
                    voice_channel_id: channel_id,
                }),
                Message::Action(ActionMessage::Playing {
                    song_title: next_metadata.title,
                    song_url: next_metadata.url,
                    voice_channel_id: channel_id,
                    user_id: next_metadata.user_id,
                }),
            ])
        }
    }

    async fn handle_pause_command(
        self: &Arc<Self>,
        ctx: &Context,
        user_id: UserId,
        guild_id: GuildId,
    ) -> Result<Vec<crate::message::Message>, crate::error::Error> {
        let delegate = ModelDelegate::new(ctx, guild_id).await?;
        let channel_id = match delegate.get_user_voice_channel(user_id) {
            Some(channel) => channel,
            None => {
                return Ok(vec![Message::Response(
                    ResponseMessage::NotInVoiceChannelError,
                )])
            }
        };

        let guild_speakers_handle = self.backend_brain.guild_speakers(guild_id);
        let mut guild_speakers_ref = guild_speakers_handle.lock().await;
        match guild_speakers_ref.find_active_in_channel(channel_id) {
            Some((guild_speaker, active_metadata)) => {
                if guild_speaker.is_paused() {
                    log::trace!("Found a paused speaker in the user's voice channel, playback will remain paused");
                    Ok(vec![Message::Response(
                        ResponseMessage::NothingIsPlayingError {
                            voice_channel_id: channel_id,
                        },
                    )])
                } else {
                    log::trace!("Found an unpaused speaker in the user's voice channel, playback will be paused");
                    guild_speaker
                        .pause()
                        .map_err(crate::error::Error::Backend)?;
                    Ok(vec![Message::Response(ResponseMessage::Paused {
                        song_title: active_metadata.title.clone(),
                        song_url: active_metadata.url.clone(),
                        voice_channel_id: channel_id,
                        user_id: active_metadata.user_id,
                    })])
                }
            }
            _ => {
                log::trace!(
                    "No speakers are in the user's voice channel, playback will not change"
                );
                Ok(vec![Message::Response(
                    ResponseMessage::NothingIsPlayingError {
                        voice_channel_id: channel_id,
                    },
                )])
            }
        }
    }

    async fn handle_nowplaying_command(
        self: &Arc<Self>,
        ctx: &Context,
        user_id: UserId,
        guild_id: GuildId,
    ) -> Result<Vec<crate::message::Message>, crate::error::Error> {
        let delegate = ModelDelegate::new(ctx, guild_id).await?;
        let channel_id = match delegate.get_user_voice_channel(user_id) {
            Some(channel) => channel,
            None => {
                return Ok(vec![Message::Response(
                    ResponseMessage::NotInVoiceChannelError,
                )])
            }
        };

        let guild_speakers_handle = self.backend_brain.guild_speakers(guild_id);
        let mut guild_speakers_ref = guild_speakers_handle.lock().await;
        match guild_speakers_ref.find_active_in_channel(channel_id) {
            Some((_, active_metadata)) => {
                Ok(vec![Message::Response(ResponseMessage::NowPlaying {
                    song_title: active_metadata.title.clone(),
                    song_url: active_metadata.url.clone(),
                    voice_channel_id: channel_id,
                    user_id: active_metadata.user_id,
                })])
            }
            _ => {
                log::trace!(
                    "No speakers are in the user's voice channel, there is nothing playing"
                );
                Ok(vec![Message::Response(
                    ResponseMessage::NothingIsPlayingError {
                        voice_channel_id: channel_id,
                    },
                )])
            }
        }
    }

    async fn handle_skip_command(
        self: &Arc<Self>,
        ctx: &Context,
        user_id: UserId,
        guild_id: GuildId,
        guild_model: &mut GuildModel<Song>,
    ) -> Result<Vec<crate::message::Message>, crate::error::Error> {
        let delegate = ModelDelegate::new(ctx, guild_id).await?;
        let channel_id = match delegate.get_user_voice_channel(user_id) {
            Some(channel) => channel,
            None => {
                return Ok(vec![Message::Response(
                    ResponseMessage::NotInVoiceChannelError,
                )])
            }
        };

        let skip_status = guild_model.vote_for_skip(&delegate, VoteType::Skip, channel_id, user_id);

        let guild_speakers_handle = self.backend_brain.guild_speakers(guild_id);
        let mut guild_speakers_ref = guild_speakers_handle.lock().await;
        let maybe_guild_speaker = guild_speakers_ref.find_active_in_channel(channel_id);

        match (skip_status, maybe_guild_speaker) {
            (VoteStatus::Success, Some((guild_speaker, active_metadata))) => {
                log::trace!("Skip command passed preconditions, stopping current playback");
                guild_speaker.stop().map_err(crate::error::Error::Backend)?;
                Ok(vec![Message::Response(ResponseMessage::Skipped {
                    song_title: active_metadata.title,
                    song_url: active_metadata.url,
                    voice_channel_id: channel_id,
                    user_id: active_metadata.user_id,
                })])
            }
            (VoteStatus::AlreadyVoted, Some((_, active_metadata))) => {
                log::trace!("User attempting to skip has already voted, not stopping playback");
                Ok(vec![Message::Response(
                    ResponseMessage::SkipAlreadyVotedError {
                        song_title: active_metadata.title,
                        song_url: active_metadata.url,
                        voice_channel_id: channel_id,
                    },
                )])
            }
            (VoteStatus::NeedsMoreVotes(count), Some((_, active_metadata))) => {
                log::trace!(
                    "Skip vote has been counted but more are needed, not stopping playback"
                );
                Ok(vec![Message::Response(
                    ResponseMessage::SkipMoreVotesNeeded {
                        song_title: active_metadata.title,
                        song_url: active_metadata.url,
                        voice_channel_id: channel_id,
                        count,
                    },
                )])
            }
            (VoteStatus::NothingPlaying, _) => {
                log::trace!(
                    "Nothing is playing in the user's voice channel, not stopping playback"
                );
                Ok(vec![Message::Response(
                    ResponseMessage::NothingIsPlayingError {
                        voice_channel_id: channel_id,
                    },
                )])
            }
            (_, None) => Err(crate::error::Error::ModelPlayingSpeakerNotDesync),
        }
    }

    async fn handle_stop_command(
        self: &Arc<Self>,
        ctx: &Context,
        user_id: UserId,
        guild_id: GuildId,
        guild_model: &mut GuildModel<Song>,
    ) -> Result<Vec<crate::message::Message>, crate::error::Error> {
        let delegate = ModelDelegate::new(ctx, guild_id).await?;
        let channel_id = match delegate.get_user_voice_channel(user_id) {
            Some(channel) => channel,
            None => {
                return Ok(vec![Message::Response(
                    ResponseMessage::NotInVoiceChannelError,
                )])
            }
        };

        match guild_model.vote_for_skip(&delegate, VoteType::Stop, channel_id, user_id) {
            VoteStatus::Success => {
                let guild_speakers_handle = self.backend_brain.guild_speakers(guild_id);
                let mut guild_speakers_ref = guild_speakers_handle.lock().await;
                let maybe_guild_speaker = guild_speakers_ref.find_active_in_channel(channel_id);
                match maybe_guild_speaker {
                    Some((guild_speaker, active_metadata)) => {
                        log::trace!("Stop command passed preconditions, stopping playback");
                        guild_model.set_channel_stopped(channel_id);
                        guild_speaker.stop().map_err(crate::error::Error::Backend)?;
                        Ok(vec![Message::Response(ResponseMessage::Stopped {
                            song_title: active_metadata.title.clone(),
                            song_url: active_metadata.url.clone(),
                            voice_channel_id: channel_id,
                            user_id: active_metadata.user_id,
                        })])
                    }
                    None => Err(crate::error::Error::ModelPlayingSpeakerNotDesync),
                }
            }
            VoteStatus::AlreadyVoted => {
                log::trace!("User attempting to stop has already voted, not stopping playback");
                Ok(vec![Message::Response(
                    ResponseMessage::StopAlreadyVotedError {
                        voice_channel_id: channel_id,
                    },
                )])
            }
            VoteStatus::NeedsMoreVotes(count) => {
                log::trace!(
                    "Stop vote has been counted but more are needed, not stopping playback"
                );
                Ok(vec![Message::Response(
                    ResponseMessage::StopMoreVotesNeeded {
                        voice_channel_id: channel_id,
                        count,
                    },
                )])
            }
            VoteStatus::NothingPlaying => {
                log::trace!(
                    "Nothing is playing in the user's voice channel, not stopping playback"
                );
                Ok(vec![Message::Response(
                    ResponseMessage::NothingIsPlayingError {
                        voice_channel_id: channel_id,
                    },
                )])
            }
        }
    }

    async fn handle_pet_command(
        self: &Arc<Self>,
    ) -> Result<Vec<crate::message::Message>, crate::error::Error> {
        let between = Uniform::from(1..8192);
        let mut rng = rand::thread_rng();
        if between.sample(&mut rng) == 1 {
            Ok(vec![Message::Response(
                ResponseMessage::ShinyPet
            )])
        } else {
            Ok(vec![Message::Response(
                ResponseMessage::Pet
            )])
        }
    }

    async fn handle_playback_ended(
        self: Arc<Self>,
        ctx: Context,
        started_channel_id: ChannelId,
        ended_handle: GuildSpeakerEndedHandle,
    ) {
        log::trace!("Playback has ended, preparing to play the next available song");

        let guild_model_handle = self.model.get(ended_handle.guild_id());
        let mut guild_model = guild_model_handle.lock().await;
        let maybe_message_channel = guild_model.message_channel();

        let (state, speaker_ended_ref) = ended_handle.lock().await;
        let messages = match state.channel_id {
            Some(channel_id) => {
                self.continue_channel_playback(
                    &ctx,
                    ended_handle.guild_id(),
                    guild_model.deref_mut(),
                    started_channel_id,
                    channel_id,
                    speaker_ended_ref,
                )
                .await
            }
            None => {
                // The speaker that played a song is no longer in a voice channel. Interpret
                // this as a forced stop command, instead of just trying to play the next song.
                guild_model.set_channel_stopped(started_channel_id);
                speaker_ended_ref.stop();
                match state.ended_metadata {
                    Some(active_metadata) => {
                        Ok(vec![Message::Response(ResponseMessage::Stopped {
                            song_title: active_metadata.title.clone(),
                            song_url: active_metadata.url.clone(),
                            voice_channel_id: started_channel_id,
                            user_id: active_metadata.user_id,
                        })])
                    }
                    None => Ok(Vec::new()),
                }
            }
        };

        let send_result = match (messages, maybe_message_channel) {
            (Ok(messages), Some(message_channel)) => {
                send_messages(
                    &self.config,
                    &ctx,
                    SendMessageDestination::Channel(message_channel),
                    guild_model.deref_mut(),
                    messages,
                )
                .await
            }
            (Err(why), Some(message_channel)) => {
                log::error!("Error while continuing playback: {}", why);
                send_messages(
                    &self.config,
                    &ctx,
                    SendMessageDestination::Channel(message_channel),
                    guild_model.deref_mut(),
                    vec![Message::Action(ActionMessage::UnknownError)],
                )
                .await
            }
            (Err(why), _) => Err(why),
            (_, None) => Ok(()),
        };

        if let Err(why) = send_result {
            log::error!("Error while continuing playback: {}", why);
        }
    }

    async fn continue_channel_playback(
        self: &Arc<Self>,
        ctx: &Context,
        guild_id: GuildId,
        guild_model: &mut GuildModel<Song>,
        started_channel_id: ChannelId,
        current_channel_id: ChannelId,
        mut speaker_ended_ref: GuildSpeakerEndedRef<'_>,
    ) -> Result<Vec<Message>, crate::error::Error> {
        // If the speaker has moved channels, simply indicate the original channel as stopped and
        // do not play anything in the new channel. This ensures we follow the behavior of not
        // playing songs until the user instructs the bot to.
        if started_channel_id != current_channel_id {
            log::trace!("Speaker has switched channel, not playing any more songs.");
            guild_model.set_channel_stopped(started_channel_id);
            speaker_ended_ref.stop();
            return Ok(Vec::new());
        }

        // Don't play anything more if the channel was stopped.
        if guild_model.is_channel_stopped(current_channel_id) {
            log::trace!("Channel has been stopped, not playing any more songs.");
            speaker_ended_ref.stop();
            return Ok(Vec::new());
        }

        let delegate = ModelDelegate::new(ctx, guild_id).await?;

        // Playing a song can fail - keep trying to play until we succeed or run out of songs
        while let Some(song) =
            guild_model.next_channel_entry_finished(&delegate, current_channel_id)
        {
            let next_metadata = song.metadata.clone();
            log::trace!("Playing \"{}\" to speaker", next_metadata.title);

            let play_res = speaker_ended_ref
                .play(
                    song,
                    &self.config.get_play_config(),
                    EndedDelegate {
                        frontend: self.clone(),
                        ctx: ctx.clone(),
                        started_channel_id: current_channel_id,
                    },
                )
                .await;

            match play_res {
                Ok(_) => {
                    return Ok(vec![Message::Action(ActionMessage::Playing {
                        song_title: next_metadata.title,
                        song_url: next_metadata.url,
                        voice_channel_id: current_channel_id,
                        user_id: next_metadata.user_id,
                    })])
                }
                Err((new_ref, why)) => {
                    log::error!("Error while continuing playback: {}", why);
                    speaker_ended_ref = new_ref;
                }
            }
        }

        log::trace!("No songs are available to play in the channel, nothing will be played");
        speaker_ended_ref.stop();
        Ok(vec![Message::Action(ActionMessage::Finished {
            voice_channel_id: current_channel_id,
        })])
    }

    async fn play_to_speaker(
        self: &Arc<Self>,
        ctx: &Context,
        guild_model: &mut GuildModel<Song>,
        guild_speaker: &mut GuildSpeakerRef<'_>,
        channel_id: ChannelId,
        song: Song,
    ) -> Result<(), crate::error::Error> {
        log::trace!("Playing \"{}\" to speaker", song.metadata.title);
        let play_res = guild_speaker
            .play(
                channel_id,
                song,
                &self.config.get_play_config(),
                EndedDelegate {
                    frontend: self.clone(),
                    ctx: ctx.clone(),
                    started_channel_id: channel_id,
                },
            )
            .await;

        match play_res {
            Ok(()) => Ok(()),
            Err(why) => {
                guild_model.set_channel_stopped(channel_id);
                Err(crate::error::Error::Backend(why))
            }
        }
    }
}

struct EndedDelegate {
    frontend: Arc<Frontend>,
    ctx: Context,
    started_channel_id: ChannelId,
}

impl EndedHandler for EndedDelegate {
    fn on_ended(self, ended_handle: GuildSpeakerEndedHandle) {
        tokio::task::spawn(self.frontend.handle_playback_ended(
            self.ctx,
            self.started_channel_id,
            ended_handle,
        ));
    }
}
