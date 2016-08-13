use std::collections::HashMap;
use std::collections::hash_map::{Entry, Keys};

#[derive(Debug, PartialEq, Eq)]
pub enum Guesser {
    Player(usize),
    NoOne,
    Gaveup,
}

pub struct CoopPlayer<T> {
    pub name: String,
    pub send: Option<T>,
    pub gaveup: bool,
}

impl<T> CoopPlayer<T> {
    pub fn new(name: String, send: T) -> CoopPlayer<T> {
        CoopPlayer {
            name: name,
            send: Some(send),
            gaveup: false,
        }
    }

    pub fn did_quit(&self) -> bool {
        return self.send.is_none();
    }
}

pub struct CoopGame<T> {
    pub players: Vec<CoopPlayer<T>>,
    pub words: HashMap<String, Guesser>,
}

pub enum JoinResult<T> {
    Ok(usize),
    Taken(T),
}

#[derive(Debug, PartialEq, Eq)]
pub enum QuitResult {
    AllGiveup,
    AllQuit,
}

impl<T> CoopGame<T> {
    pub fn new(first_player: CoopPlayer<T>, words: HashMap<String, Guesser>) -> CoopGame<T> {
        CoopGame {
            players: vec![first_player],
            words: words,
        }
    }

    /// Returns a String iterator that iterates through the list of words.
    pub fn words_iter(&self) -> Keys<String, Guesser> {
        return self.words.keys();
    }

    pub fn try_join(&mut self, name: String, send: T) -> JoinResult<T> {
        // First see if the player already exists.  If the player exists
        // but previously left the game, replace them.
        for (n, p) in self.players.iter_mut().enumerate() {
            if p.name == name {
                if p.did_quit() {
                    p.send = Some(send);
                    return JoinResult::Ok(n);
                } else {
                    return JoinResult::Taken(send);
                }
            }
        }

        let new_p = CoopPlayer::new(name, send);
        self.players.push(new_p);
        return JoinResult::Ok(self.players.len() - 1);
    }

    /// Attempt to guess a word.  Returns true if the guess was successful,
    /// otherwise false (the word was already guessed or does not exist).
    pub fn attempt(&mut self, player_num: usize, word: String) -> bool {
        if let Entry::Occupied(mut e) = self.words.entry(word) {
            if e.get() == &Guesser::NoOne {
                e.insert(Guesser::Player(player_num));
                return true;
            }
        }
        return false;
    }

    /// Modifies all words that have not been guessed to be guessed by 
    /// Guesser::NoOne.
    fn allgiveup(&mut self) {
        for (_, g) in self.words.iter_mut() {
            if *g == Guesser::NoOne {
                *g = Guesser::Gaveup;
            }
        }
    }

    /// Removes this player's send entry, and sets its giveup status to false.
    /// If this player quitting will trigger an AllGiveup or AllQuit, this
    /// function will return Some(QuitResult); otherwise will return None.
    pub fn player_quit(&mut self, player_num: usize) -> Option<QuitResult> {
        self.players[player_num].send = None;
        self.players[player_num].gaveup = false;

        let mut result = QuitResult::AllQuit;
        for p in self.players.iter() {
            if !p.did_quit() {
                if !p.gaveup {
                    return None;
                }
                result = QuitResult::AllGiveup;
            }
        }
        
        if result == QuitResult::AllGiveup {
            self.allgiveup();
        }

        return Some(result);
    }

    /// Sets a player's giveup status to true.  If this will trigger an
    /// allgiveup, returns true.
    pub fn player_giveup(&mut self, player_num: usize) -> bool {
        self.players[player_num].gaveup = true;

        if self.players.iter().all(|p| p.gaveup || p.did_quit()) {
            self.allgiveup();
            return true;
        }
        return false;
    }

    pub fn player_ungiveup(&mut self, player_num: usize) {
        self.players[player_num].gaveup = false;
    }
}
