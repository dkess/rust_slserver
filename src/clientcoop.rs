extern crate itertools;

//use std::string::FromUtf8Error;
use std::error::Error;
use self::itertools::free::join;
use regex::Regex;
use std::collections::HashMap;
use std::str;
use std::sync::Mutex;
use websocket;
use websocket::Receiver;
use websocket::Sender;
use websocket::WebSocketStream;
use websocket::dataframe::{DataFrame, Opcode};
use websocket::result::{WebSocketResult, WebSocketError};

use coop;

pub type WSSend = websocket::client::Sender<WebSocketStream>;
pub type WSReceive = websocket::client::Receiver<WebSocketStream>;
pub type WSGame = coop::CoopGame<WSSend>;

fn send_msg(send: &mut WSSend, msg: String) -> WebSocketResult<()> {
    let msg = DataFrame::new(true, Opcode::Text, msg.into_bytes());
    return send.send_dataframe(&msg);
}

/// Sends a message to everyone in the game
fn announce_msg(game: &mut WSGame, msg: String, except: Option<usize>) {
    let msg = DataFrame::new(true, Opcode::Text, msg.into_bytes());

    for mut send in game.players.iter_mut().enumerate().filter_map(|(n, p)| {
                if Some(n) != except {
                    p.send.as_mut()
                } else {
                    None
                }
            }) {
        send.send_dataframe(&msg);
    }
}

/// Waits for the user to provide all the necessary game information over the
/// websocket, then returns the CoopGame.
/// Will panic if anything goes wrong.
pub fn host_coop(send: WSSend, receive: &mut WSReceive, name: String) -> WSGame {
    lazy_static! {
        static ref WORDREGEX: Regex = Regex::new(r"[a-z]{3,6}_?").unwrap();
    }

    let mut words = HashMap::new();

    let msg = receive.recv_dataframe().unwrap();

    // get the list of words from the client
    for word in msg.data.split(|c| c == &(' ' as u8)).take(75) {
        let word = str::from_utf8(word).unwrap();
        if WORDREGEX.is_match(word) {
            // If a word has already been guessed, it will end with a _
            if word.bytes().rev().next() == Some('_' as u8) {
                words.insert(word[..word.len()-1].to_owned(),
                                  coop::Guesser::Player(0));
            } else {
                words.insert(word.to_owned(), coop::Guesser::NoOne);
            }
        } else {
            panic!();
        }
    }

    // create the player
    let p = coop::CoopPlayer::new(name, send);

    coop::CoopGame::new(p, words)
}

/// Sends the name of the game to the host
pub fn send_gamename(gamename: String, game: &Mutex<WSGame>) {
    let g = &mut game.lock().unwrap();
    let send = g.players[0].send.as_mut().unwrap();
    send_msg(send, gamename);
}


/// Gets called whenever a new player joins, on this player's thread.  This
/// function should send the new player the current game state, and announce
/// to everyone else that this player has joined.
/// Will return an error if and only if sending a message to the new player
/// fails.
fn on_playerjoin(player_num: usize, game: &mut WSGame) -> Result<(), WebSocketError> {
    let ref pname = game.players[player_num].name.clone();
    
    // announce to everyone else that this player has joined
    announce_msg(game, format!(":join {}", pname), Some(player_num));

    let mut msgs = Vec::with_capacity(2);

    // the player list
    msgs.push(join(game.players.iter().map(|p| {
        let mut s = p.name.to_owned();
        if p.did_quit() {
            s.push('_');
        }
        s
    }), " "));

    // the list of words
    msgs.push(join(game.words_iter(), " "));

    // previously guessed words
    msgs.extend(game.words.iter().filter_map(|(k, v)| {
        if let coop::Guesser::Player(n) = *v {
            Some(format!(":attempt {} {}",
                         k,
                         game.players[n].name))
        } else if let coop::Guesser::Gaveup = *v {
            Some(format!(":attempt {} _", k))
        } else {
            None
        }
    }));

    // players who have given up
    msgs.extend(game.players.iter().filter_map(|p| {
        if p.gaveup {
            Some(format!(":giveup {}", p.name))
        } else {
            None
        }
    }));

    let mut send = game.players[player_num].send.as_mut().unwrap();

    for m in msgs.into_iter() {
        try!(send_msg(send, m));
    }

    Ok(())
}

pub fn join_coop(mut send: WSSend, receive: &mut WSReceive, game: &Mutex<WSGame>) -> Result<usize, WebSocketError> {
    lazy_static! {
        static ref NAMEREGEX: Regex = Regex::new(r"^[a-zA-Z0-9]{1,10}$").unwrap();
    }
    send_msg(&mut send, String::from(":ok"));

    // keep asking for a name until the user enters a valid one
    for frame in receive.incoming_dataframes() {
        let name = String::from_utf8(frame.unwrap().data).unwrap();

        if !NAMEREGEX.is_match(&name) {
            send_msg(&mut send, String::from(":badname")).unwrap();
            continue;
        }
        let ref mut game = *game.lock().unwrap();
        match game.try_join(name, send) {
            coop::JoinResult::Ok(n) => {
                try!(on_playerjoin(n, game));
                return Ok(n);
            },
            coop::JoinResult::Taken(mut s) => {
                send_msg(&mut s, String::from(":taken")).unwrap();
                send = s;
            },
        };
    }

    panic!();
}

/// Sends the allgiveup message to everyone
fn on_allgiveup(game: &mut WSGame) {
    announce_msg(game, String::from(":allgiveup"), None);
}

pub fn game_loop(receive: &mut WSReceive, pnum: usize, game: &Mutex<WSGame>) -> Result<(), Box<Error>> {
    lazy_static! {
        static ref ATTEMPT: Regex = Regex::new(r"^:attempt ([a-z]{3,6})$").unwrap();
    }

    for frame in receive.incoming_dataframes() {
        let msg = try!(String::from_utf8(try!(frame).data));

        if let Some(c) = ATTEMPT.captures(&msg) {
            let word = &c[1];
            let mut game = game.lock().unwrap();
            let success = game.attempt(pnum, word.to_owned());
            if success {
                let announce = {
                    let ref name = game.players[pnum].name;
                    format!(":attempt {} {}", word, name)
                };
                announce_msg(&mut game, announce, Some(pnum));
            }
        } else if msg == ":giveup" {
            let mut game = game.lock().unwrap();
            let announce = {
                let ref name = game.players[pnum].name;
                format!(":giveup {}", name)
            };
            announce_msg(&mut game, announce, Some(pnum));

            if game.player_giveup(pnum) {
                on_allgiveup(&mut game);
            }
        } else if msg == ":ungiveup" {
            let mut game = game.lock().unwrap();
            let announce = {
                let ref name = game.players[pnum].name;
                format!(":ungiveup {}", name)
            };
            announce_msg(&mut game, announce, Some(pnum));

            game.player_ungiveup(pnum);
        } else if msg.starts_with(":chat ") {
            let mut game = game.lock().unwrap();
            let chatmsg = &msg[":chat ".len() ..];
            let announce = {
                let ref name = game.players[pnum].name;
                format!(":chat {} {}", name, chatmsg)
            };
            announce_msg(&mut game, announce, Some(pnum));
        }
    }
    Ok(())
}

/// Will be run when a player leaves the game.  This function will set the
/// player's status to quit, and will inform everyone else that the player has
/// quit.  Additionally, if this quit triggers an allgiveup, it will modify the
/// guessed players to reflect this, and send the allgiveup message.  Will
/// return true if no one is left in the game.
pub fn on_disconnect(pnum: usize, game: &Mutex<WSGame>) -> bool {
    let mut game = game.lock().unwrap();
    let msg = format!(":quit {}", game.players[pnum].name);
    announce_msg(&mut game, msg, Some(pnum));

    match game.player_quit(pnum) {
        Some(coop::QuitResult::AllGiveup) => {
            on_allgiveup(&mut game);
            false
        },
        Some(coop::QuitResult::AllQuit) => {
            println!("everyone left!");
            true
        },
        _ => false
    }
}
