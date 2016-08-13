#[macro_use] extern crate lazy_static;
extern crate rand;
extern crate regex;
extern crate websocket;

use rand::{Rng, weak_rng};
use regex::Regex;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex, Condvar, RwLock};
use std::thread;
use std::time::Duration;
use websocket::Server;
use websocket::server::request::RequestUri;

mod clientcoop;
mod coop;

/// How long we should wait before destroying an empty game
const GAME_KILL_TIMER_MILLIS: u64 = 86400000;

#[derive(Debug)]
enum URLAction {
    Host(String),
    Join(String),
}

struct GameEntry<T> {
    game: Mutex<T>,

    /// This Condvar gets notified every time someone joins the game.  It
    /// "saves" the game from being deleted if everyone has left and the
    /// deletion timer is running.
    idle_condvar: Condvar,

    /// The number of people connected to this game, including players who have
    /// not yet chosen their name.  When this drops to zero, the deletion timer
    /// for this game will begin.
    connections: Mutex<u8>,
}

/// Parses an action from a URL.  If this is a coop game, the bool value will
/// be true.
fn get_urlaction(url: &RequestUri) -> Option<(bool, URLAction)> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"(?x)^/ws
                  # Possibility 1: hosting a game, and supplying
                  # player name
                  /(?:(hostcoop|hostcomp)/([a-zA-Z0-9]{1,10})

                  # Possibility 2: joining a pre-existing game.
                  # If the game name starts with a c, this is a coop
                  # game; otherwise it is a competitive game.
                     |join/(c|m)([a-z0-9]{5,8}))$").unwrap();
    }

    if let &RequestUri::AbsolutePath(ref path) = url {
        if let Some(cap) = RE.captures(path) {
            if cap.at(1) == Some("hostcoop") {
                return cap.at(2).map(|x|
                                     (true, URLAction::Host(String::from(x))));
            } else if cap.at(1) == Some("hostcomp") {
                return cap.at(2).map(|x|
                                     (false, URLAction::Host(String::from(x))));
            } else if cap.at(3) == Some("c") {
                return cap.at(4).map(|x|
                                     (true, URLAction::Join(String::from(x))));
            } else if cap.at(3) == Some("m") {
                return cap.at(4).map(|x|
                                     (false, URLAction::Join(String::from(x))));
            }
        }
    }
    return None;
}

fn generate_gamename() -> String {
    const GAMENAME_CHARS: &'static [u8] =
        b"abcdefghijklmnopqrstuvwxyz0123456789";
    
    let num_chars = 5;
    let mut rng = weak_rng();
    let mut s = String::with_capacity(10);
    for _ in 0 .. num_chars {
        s.push(*rng.choose(GAMENAME_CHARS).unwrap() as char);
    }
    return s;
}

fn main() {
    let coop_games = Arc::new(RwLock::new(HashMap::new()));

    let server = Server::bind("127.0.0.1:8754").unwrap();

    for connection in server {
        let coop_games = coop_games.clone();

        thread::spawn(move || {
            let request = connection.unwrap().read_request().unwrap();

            if let Some((is_coop, action)) = get_urlaction(&request.url) {
                request.validate().unwrap();
                let response = request.accept();
                let client = response.send().unwrap();
                let (send, mut receive) = client.split();

                if is_coop {
                    let (game_entry, pnum, gamename) = match action {
                        URLAction::Host(name) => {
                            let game = clientcoop::host_coop(send,
                                                             &mut receive,
                                                             name);

                            let game_entry = Arc::new(GameEntry {
                                game: Mutex::new(game),
                                idle_condvar: Condvar::new(),
                                connections: Mutex::new(1),
                            });

                            // Keep generating a gamename until we find one 
                            // that hasn't been taken, then place the game into
                            // the dict
                            let mut gamename = generate_gamename();
                            {
                                let game_entry = game_entry.clone();
                                let mut coop_games =
                                        coop_games.write().unwrap();
                                loop {
                                    match coop_games.entry(gamename) {
                                        // if this gamename has already been
                                        // taken, generate a new one
                                        Entry::Occupied(_) =>
                                            gamename = generate_gamename(),
                                            Entry::Vacant(e) => {
                                                gamename = e.key().to_owned();
                                                e.insert(game_entry);
                                                break;
                                            },
                                    }
                                }
                            };

                            clientcoop::send_gamename(gamename.clone(), &game_entry.game);
                            (game_entry, 0, gamename)
                        },
                        URLAction::Join(gamename) => {
                            let game_entry = {
                                let coop_games = coop_games.read().unwrap();
                                coop_games.get(&gamename).unwrap().clone()
                            };

                            game_entry.idle_condvar.notify_all();
                            *game_entry.connections.lock().unwrap() += 1;

                            let pnum = clientcoop::join_coop(send,
                                                             &mut receive,
                                                             &game_entry.game)
                                                            .unwrap();
                            (game_entry, pnum, gamename)
                        }
                    };

                    let mut g = &game_entry.game;

                    let err = clientcoop::game_loop(&mut receive, pnum, &g);

                    println!("player quit {:?}", err);

                    clientcoop::on_disconnect(pnum, &mut g);

                    let condvar = &game_entry.idle_condvar;
                    let connections = &game_entry.connections;

                    let mut connections = connections.lock().unwrap();
                    *connections -= 1;

                    // If there is no one connected, start a timer to destroy
                    // the game
                    if *connections == 0 {
                        let dur = Duration::from_millis(GAME_KILL_TIMER_MILLIS);
                        let r = condvar.wait_timeout(connections, dur);
                        let (_, r) = r.unwrap();
                        if r.timed_out() {
                            let mut coop_games = coop_games.write().unwrap();
                            println!("destroying game {}", gamename);
                            coop_games.remove(&gamename);
                        } else {
                            println!("saved {}", gamename);
                        }
                    }
                }
            }
            return;
        });
    }
}
