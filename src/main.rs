extern crate irc;
extern crate rustc_serialize;
extern crate discord;
extern crate regex;
#[macro_use]
extern crate lazy_static;

use std::fs::File;
use std::io::prelude::*;
use std::io::{Error, ErrorKind, Result};
use std::path::Path;
use std::thread::spawn;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

use discord::Discord;
use discord::model::{Event, ChannelId};
use irc::client::prelude::*;
use rustc_serialize::json::decode;
use regex::Regex;


#[derive(Clone, RustcDecodable, RustcEncodable, PartialEq, Debug)]
struct DiscordConfig {
    bot_token: String,
}

#[derive(Clone, RustcDecodable, RustcEncodable, PartialEq, Debug)]
struct Mapping {
    discord2irc: HashMap<u64, String>,
    irc2discord: HashMap<String, u64>,
}

#[derive(Clone, RustcDecodable, RustcEncodable, PartialEq, Debug)]
struct Config {
    irc_config: irc::client::data::Config,
    discord_config: DiscordConfig,
    mapping: Mapping,
    filterchars: String,
}

/// Loads a Config and parses it into a Config struct
fn load<P: AsRef<Path>>(path: P) -> Result<Config> {
    let mut file = File::open(path)?;
    let mut data = String::new();
    file.read_to_string(&mut data)?;
    decode(&data[..]).map_err(|_| {
        Error::new(ErrorKind::InvalidInput,
                   "Failed to decode configuration file.")
    })
}

/// Hashes the given value
fn hash<T: Hash>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn colorize(s: &str) -> u64 {
    hash(&s) % 16
}

fn remove_formatting<'t>(s: &'t str) -> std::borrow::Cow<'t, str> {
    lazy_static! {
        static ref RE: Regex = Regex::new("[\x02\x1F\x0F\x16]|\x03(\\d\\d?(,\\d\\d?)?)?").unwrap();
    }
    RE.replace_all(s, "")
}


fn main() {
    println!("Starting bridge");
    let config = load("config.json").unwrap();
    let irc_config = config.irc_config;
    let discord_config = config.discord_config;
    let discord2irc = config.mapping.discord2irc;
    let irc2discord = config.mapping.irc2discord;
    let filterchars = config.filterchars;

    let irc_server = IrcServer::from_config(irc_config).unwrap();
    irc_server.identify().unwrap();

    let discord_api = Discord::from_bot_token(&discord_config.bot_token).unwrap();
    let (mut listener, _) = discord_api.connect().unwrap();

    println!("Bridge started.");

    let filterchars_ = filterchars.clone();
    let iserver2 = irc_server.clone();

    let discord = spawn(move || {
        loop {
            match listener.recv_event() {
                Ok(Event::MessageCreate(msg)) => {
                    if msg.author.bot {
                        continue; // ignore bots
                    }
                    if filterchars_.chars().any(|c| msg.content.starts_with(c)) {
                        continue;
                    }
                    if let Some(target) = discord2irc.get(&msg.channel_id.0) {
                        let attachments = if msg.attachments.len() > 0 {
                            "[Attachments: ".to_string() +
                            &msg.attachments
                                .into_iter()
                                .map(|a| a.filename + " (" + &a.url + ")")
                                .collect::<Vec<_>>()
                                .join(", ") + "]"
                        } else {
                            "".into()
                        };
                        let mut c = msg.content;
                        for u in msg.mentions.iter() {
                            c = c.replace(&format!("<@{}>", u.id), &format!("@{}", u.name));
                        }
                        //for r in msg.mention_roles.iter() {
                        //    c = c.replace(&format!("<@&{}>", r, &format!("@RoleNameHere??"));
                        //}

                        for line in c.lines() {
                            match iserver2.send_privmsg(target,
                                                        &format!("<\x03{}{}\x03> {} {}",
                                                                 colorize(&msg.author.name),
                                                                 msg.author.name,
                                                                 line,
                                                                 attachments)) {
                                Ok(_) => continue,
                                Err(e) => println!("Error writing to irc: {:?}", e),
                            }
                        }
                    }

                }
                Ok(_) => continue,
                Err(e) => println!("Discord recv event error: {:?}", e),
            }
        }
    });


    spawn(move || {
            for msg in irc_server.iter() {
                let msg = msg.unwrap();
                if let Command::PRIVMSG(ref target, ref content) = msg.command {
                    if filterchars.chars().any(|c| content.starts_with(c)) {
                        continue;
                    }
                    if let Some(target) = irc2discord.get(target) {
                        msg.source_nickname().map(|nick| {
                            if let Err(e) = discord_api.send_message(&ChannelId(*target),
                                                                     &format!("**<{}>** {}",
                                                                          nick,
                                                                          remove_formatting(content)),
                                                                     "",
                                                                     false) {
                                println!("{:?}", e);
                            }
                        });
                    }
                };

            }
        })
        .join()
        .unwrap_or_else(|e| println!("IRC Thread errored with: {:?}", e));
    println!("IRC Thread died.");
    discord.join().unwrap_or_else(|e| println!("Discord Thread errored with: {:?}", e));
    println!("Discord Thread died.");
}
