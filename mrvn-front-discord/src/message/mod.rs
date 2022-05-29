use serenity::model::prelude::*;

mod send_message;

pub use self::send_message::*;

#[derive(Debug, Clone)]
pub enum Message {
    Action(ActionMessage),
    Response(ResponseMessage),
}

impl Message {
    pub fn is_action(&self) -> bool {
        match self {
            Message::Action(_) => true,
            Message::Response(_) => false,
        }
    }

    pub fn create_embed<'e>(
        &self,
        embed: &'e mut serenity::builder::CreateEmbed,
        config: &crate::config::Config,
    ) -> &'e mut serenity::builder::CreateEmbed {
        match self {
            Message::Action(action) => action.create_embed(embed, config),
            Message::Response(response) => response.create_embed(embed, config),
        }
    }
}

/// Action messages have the possibility of being sent not directly as a response to a command
/// invocation. Only one action message is kept around in a guild at a time, old ones are deleted
/// when new ones are sent.
#[derive(Debug, Clone)]
pub enum ActionMessage {
    Playing {
        song_title: String,
        song_url: String,
        voice_channel_id: ChannelId,
        user_id: UserId,
    },
    PlayingResponse {
        song_title: String,
        song_url: String,
        voice_channel_id: ChannelId,
    },
    Finished {
        voice_channel_id: ChannelId,
    },
    NoSpeakersError {
        voice_channel_id: ChannelId,
    },
    UnknownError,
}

/// Response messages are always sent directly as a response to a command invocation.
#[derive(Debug, Clone)]
pub enum ResponseMessage {
    Queued {
        song_title: String,
        song_url: String,
    },
    QueuedMultiple {
        count: usize,
    },
    QueuedNoSpeakers {
        song_title: String,
        song_url: String,
    },
    QueuedMultipleNoSpeakers {
        count: usize,
    },
    Replaced {
        old_song_title: String,
        old_song_url: String,
        new_song_title: String,
        new_song_url: String,
    },
    ReplaceSkipped {
        new_song_title: String,
        new_song_url: String,
        old_song_title: String,
        old_song_url: String,
        voice_channel_id: ChannelId,
    },
    Paused {
        song_title: String,
        song_url: String,
        voice_channel_id: ChannelId,
        user_id: UserId,
    },
    Skipped {
        song_title: String,
        song_url: String,
        voice_channel_id: ChannelId,
        user_id: UserId,
    },
    SkipMoreVotesNeeded {
        song_title: String,
        song_url: String,
        voice_channel_id: ChannelId,
        count: usize,
    },
    Stopped {
        song_title: String,
        song_url: String,
        voice_channel_id: ChannelId,
        user_id: UserId,
    },
    StopMoreVotesNeeded {
        voice_channel_id: ChannelId,
        count: usize,
    },
    ImageEmbed {
        image_url: String,
    },
    NoMatchingSongsError,
    NotInVoiceChannelError,
    UnsupportedSiteError,
    SkipAlreadyVotedError {
        song_title: String,
        song_url: String,
        voice_channel_id: ChannelId,
    },
    StopAlreadyVotedError {
        voice_channel_id: ChannelId,
    },
    NothingIsQueuedError {
        voice_channel_id: ChannelId,
    },
    NothingIsPlayingError {
        voice_channel_id: ChannelId,
    },
    AlreadyPlayingError {
        voice_channel_id: ChannelId,
    },
<<<<<<< HEAD
    Pet,
    ShinyPet,
    NowPlaying {
        song_title: String,
        song_url: String,
        voice_channel_id: ChannelId,
        user_id: UserId,
    },
=======

    StreakWait,
    Streak {
        streak_length: u64,
    },
    NoStreak,
>>>>>>> 25da9861779fb430f167acfd412312246e44a676
}

impl ActionMessage {
    pub fn to_string(&self, config: &crate::config::Config) -> String {
        match self {
            ActionMessage::Playing {
                song_title,
                song_url,
                voice_channel_id,
                user_id,
            } => {
                let channel_id_string = voice_channel_id.0.to_string();
                let user_id_string = user_id.0.to_string();
                config.get_message(
                    "action.playing",
                    &[
                        ("song_title", song_title),
                        ("song_url", song_url),
                        ("voice_channel_id", &channel_id_string),
                        ("user_id", &user_id_string),
                    ],
                )
            }
            ActionMessage::PlayingResponse {
                song_title,
                song_url,
                voice_channel_id,
            } => {
                let channel_id_string = voice_channel_id.0.to_string();
                config.get_message(
                    "action.playing_response",
                    &[
                        ("song_title", song_title),
                        ("song_url", song_url),
                        ("voice_channel_id", &channel_id_string),
                    ],
                )
            }
            ActionMessage::Finished { voice_channel_id } => {
                let channel_id_string = voice_channel_id.0.to_string();
                config.get_message(
                    "action.finished",
                    &[("voice_channel_id", &channel_id_string)],
                )
            }
            ActionMessage::NoSpeakersError { voice_channel_id } => {
                let channel_id_string = voice_channel_id.0.to_string();
                config.get_message(
                    "action.no_speakers_error",
                    &[("voice_channel_id", &channel_id_string)],
                )
            }
            ActionMessage::UnknownError => {
                config.get_raw_message("action.unknown_error").to_string()
            }
        }
    }

    pub fn is_error(&self) -> bool {
        match self {
            ActionMessage::Playing { .. }
            | ActionMessage::PlayingResponse { .. }
            | ActionMessage::Finished { .. } => false,
            ActionMessage::NoSpeakersError { .. } | ActionMessage::UnknownError => true,
        }
    }

    pub fn create_embed<'e>(
        &self,
        embed: &'e mut serenity::builder::CreateEmbed,
        config: &crate::config::Config,
    ) -> &'e mut serenity::builder::CreateEmbed {
        embed
            .description(self.to_string(config))
            .color(if self.is_error() {
                config.error_embed_color
            } else {
                config.action_embed_color
            })
    }
}

impl ResponseMessage {
    pub fn to_string(&self, config: &crate::config::Config) -> String {
        match self {
            ResponseMessage::Queued {
                song_title,
                song_url,
            } => config.get_message(
                "response.queued",
                &[("song_title", song_title), ("song_url", song_url)],
            ),
            ResponseMessage::QueuedMultiple { count } => {
                let count_string = count.to_string();
                config.get_message("response.queued_multiple", &[("count", &count_string)])
            }
            ResponseMessage::QueuedNoSpeakers {
                song_title,
                song_url,
            } => config.get_message(
                "response.queued_no_speakers",
                &[("song_title", song_title), ("song_url", song_url)],
            ),
            ResponseMessage::QueuedMultipleNoSpeakers { count } => {
                let count_string = count.to_string();
                config.get_message(
                    "response.queued_multiple_no_speakers",
                    &[("count", &count_string)],
                )
            }
            ResponseMessage::Replaced {
                old_song_title,
                old_song_url,
                new_song_title,
                new_song_url,
            } => config.get_message(
                "response.replaced",
                &[
                    ("old_song_title", old_song_title),
                    ("old_song_url", old_song_url),
                    ("new_song_title", new_song_title),
                    ("new_song_url", new_song_url),
                ],
            ),
            ResponseMessage::ReplaceSkipped {
                new_song_title,
                new_song_url,
                old_song_title,
                old_song_url,
                voice_channel_id,
            } => {
                let channel_id_string = voice_channel_id.0.to_string();
                config.get_message(
                    "response.replace_skipped",
                    &[
                        ("new_song_title", new_song_title),
                        ("new_song_url", new_song_url),
                        ("old_song_title", old_song_title),
                        ("old_song_url", old_song_url),
                        ("voice_channel_id", &channel_id_string),
                    ],
                )
            }
            ResponseMessage::Paused {
                song_title,
                song_url,
                voice_channel_id,
                user_id,
            } => {
                let channel_id_string = voice_channel_id.0.to_string();
                let user_id_string = user_id.0.to_string();
                config.get_message(
                    "response.paused",
                    &[
                        ("song_title", song_title),
                        ("song_url", song_url),
                        ("voice_channel_id", &channel_id_string),
                        ("user_id", &user_id_string),
                    ],
                )
            }
            ResponseMessage::Skipped {
                song_title,
                song_url,
                voice_channel_id,
                user_id,
            } => {
                let channel_id_string = voice_channel_id.0.to_string();
                let user_id_string = user_id.0.to_string();
                config.get_message(
                    "response.skipped",
                    &[
                        ("song_title", song_title),
                        ("song_url", song_url),
                        ("voice_channel_id", &channel_id_string),
                        ("user_id", &user_id_string),
                    ],
                )
            }
            ResponseMessage::SkipMoreVotesNeeded {
                song_title,
                song_url,
                voice_channel_id,
                count,
            } => {
                let channel_id_string = voice_channel_id.0.to_string();
                if *count == 1 {
                    config.get_message(
                        "response.skip_more_votes_needed.singular",
                        &[
                            ("song_title", song_title),
                            ("song_url", song_url),
                            ("voice_channel_id", &channel_id_string),
                        ],
                    )
                } else {
                    let count_string = count.to_string();
                    config.get_message(
                        "response.skip_more_votes_needed.plural",
                        &[
                            ("song_title", song_title),
                            ("song_url", song_url),
                            ("voice_channel_id", &channel_id_string),
                            ("count", &count_string),
                        ],
                    )
                }
            }
            ResponseMessage::Stopped {
                song_title,
                song_url,
                voice_channel_id,
                user_id,
            } => {
                let channel_id_string = voice_channel_id.0.to_string();
                let user_id_string = user_id.0.to_string();
                config.get_message(
                    "response.stopped",
                    &[
                        ("song_title", song_title),
                        ("song_url", song_url),
                        ("voice_channel_id", &channel_id_string),
                        ("user_id", &user_id_string),
                    ],
                )
            }
            ResponseMessage::StopMoreVotesNeeded {
                voice_channel_id,
                count,
            } => {
                let channel_id_string = voice_channel_id.0.to_string();
                if *count == 1 {
                    config.get_message(
                        "response.stop_more_votes_needed.singular",
                        &[("voice_channel_id", &channel_id_string)],
                    )
                } else {
                    let count_string = count.to_string();
                    config.get_message(
                        "response.stop_more_votes_needed.plural",
                        &[
                            ("voice_channel_id", &channel_id_string),
                            ("count", &count_string),
                        ],
                    )
                }
            }
            ResponseMessage::NoMatchingSongsError => config
                .get_raw_message("response.no_matching_songs_error")
                .to_string(),
            ResponseMessage::NotInVoiceChannelError => config
                .get_raw_message("response.not_in_voice_channel_error")
                .to_string(),
            ResponseMessage::UnsupportedSiteError => config
                .get_raw_message("response.unsupported_site_error")
                .to_string(),
            ResponseMessage::SkipAlreadyVotedError {
                song_title,
                song_url,
                voice_channel_id,
            } => {
                let channel_id_string = voice_channel_id.0.to_string();
                config.get_message(
                    "response.skip_already_voted_error",
                    &[
                        ("song_title", song_title),
                        ("song_url", song_url),
                        ("voice_channel_id", &channel_id_string),
                    ],
                )
            }
            ResponseMessage::StopAlreadyVotedError { voice_channel_id } => {
                let channel_id_string = voice_channel_id.0.to_string();
                config.get_message(
                    "response.stop_already_voted_error",
                    &[("voice_channel_id", &channel_id_string)],
                )
            }
            ResponseMessage::NothingIsQueuedError { voice_channel_id } => {
                let channel_id_string = voice_channel_id.0.to_string();
                config.get_message(
                    "response.nothing_is_queued_error",
                    &[("voice_channel_id", &channel_id_string)],
                )
            }
            ResponseMessage::NothingIsPlayingError { voice_channel_id } => {
                let channel_id_string = voice_channel_id.0.to_string();
                config.get_message(
                    "response.nothing_is_playing_error",
                    &[("voice_channel_id", &channel_id_string)],
                )
            }
            ResponseMessage::AlreadyPlayingError { voice_channel_id } => {
                let channel_id_string = voice_channel_id.0.to_string();
                config.get_message(
                    "response.already_playing_error",
                    &[("voice_channel_id", &channel_id_string)],
                )
            }
            ResponseMessage::ImageEmbed { image_url } => image_url.clone(),
<<<<<<< HEAD
            ResponseMessage::Pet => config
                .get_raw_message("response.pet")
                .to_string(),
            ResponseMessage::ShinyPet => config
                .get_raw_message("response.shiny_pet")
                .to_string(),
            ResponseMessage::NowPlaying {
                song_title,
                song_url,
                voice_channel_id,
                user_id,
            } => {
                let channel_id_string = voice_channel_id.0.to_string();
                let user_id_string = user_id.0.to_string();
                config.get_message(
                    "response.now_playing",
                    &[
                        ("song_title", song_title),
                        ("song_url", song_url),
                        ("voice_channel_id", &channel_id_string),
                        ("user_id", &user_id_string),
                    ],
                )
            }
=======

            ResponseMessage::StreakWait => {
                config.get_raw_message("response.streak_wait").to_string()
            }
            ResponseMessage::Streak { streak_length } => {
                let streak_length_string = streak_length.to_string();
                config.get_message(
                    "response.streak",
                    &[("streak_length", &streak_length_string)],
                )
            }
            ResponseMessage::NoStreak => config.get_raw_message("response.no_streak").to_string(),
>>>>>>> 25da9861779fb430f167acfd412312246e44a676
        }
    }

    pub fn is_error(&self) -> bool {
        match self {
            ResponseMessage::Queued { .. }
            | ResponseMessage::QueuedMultiple { .. }
            | ResponseMessage::QueuedNoSpeakers { .. }
            | ResponseMessage::QueuedMultipleNoSpeakers { .. }
            | ResponseMessage::Replaced { .. }
            | ResponseMessage::ReplaceSkipped { .. }
            | ResponseMessage::Paused { .. }
            | ResponseMessage::Skipped { .. }
            | ResponseMessage::SkipMoreVotesNeeded { .. }
            | ResponseMessage::Stopped { .. }
            | ResponseMessage::StopMoreVotesNeeded { .. }
<<<<<<< HEAD
            | ResponseMessage::ImageEmbed { .. } 
            | ResponseMessage::Pet 
            | ResponseMessage::ShinyPet 
            | ResponseMessage::NowPlaying { .. } => false,
=======
            | ResponseMessage::ImageEmbed { .. }
            | ResponseMessage::StreakWait
            | ResponseMessage::Streak { .. }
            | ResponseMessage::NoStreak => false,
>>>>>>> 25da9861779fb430f167acfd412312246e44a676
            ResponseMessage::NoMatchingSongsError
            | ResponseMessage::NotInVoiceChannelError
            | ResponseMessage::UnsupportedSiteError
            | ResponseMessage::SkipAlreadyVotedError { .. }
            | ResponseMessage::StopAlreadyVotedError { .. }
            | ResponseMessage::NothingIsQueuedError { .. }
            | ResponseMessage::NothingIsPlayingError { .. }
            | ResponseMessage::AlreadyPlayingError { .. } => true,
        }
    }

    pub fn create_embed<'e>(
        &self,
        embed: &'e mut serenity::builder::CreateEmbed,
        config: &crate::config::Config,
    ) -> &'e mut serenity::builder::CreateEmbed {
        embed.color(if self.is_error() {
            config.error_embed_color
        } else {
            config.response_embed_color
        });
        match self {
            ResponseMessage::ImageEmbed { image_url } => embed.image(image_url),
            ResponseMessage::Pet => embed.image(self.to_string(config)),
            ResponseMessage::ShinyPet => embed.image(self.to_string(config)),
            _ => embed.description(self.to_string(config)),
        }
    }
}
