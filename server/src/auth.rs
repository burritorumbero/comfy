//! The one password for the page and the app, set once with `--set-password`.
//!
//! It is kept only as a salted PBKDF2-SHA256 hash. There are no sessions: every request
//! carries the password (the app as a bearer token, browsers through their own password
//! prompt), so a password that checked out once is remembered by its SHA-256 instead of
//! paying for PBKDF2 again.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::store;

pub const MIN_PASSWORD_CHARS: usize = 10;
pub const PBKDF2_ITERATIONS: u32 = 600_000;
/// After this many wrong passwords within the window, a device waits out the window.
pub const WRONG_PASSWORDS_ALLOWED: usize = 5;
pub const WRONG_PASSWORD_WINDOW: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Serialize, Deserialize)]
struct StoredPassword {
    algorithm: String,
    iterations: u32,
    salt: String,
    hash: String,
}

#[derive(Default, Serialize, Deserialize)]
struct AuthFile {
    password: Option<StoredPassword>,
}

#[derive(Debug, PartialEq)]
pub enum Check {
    Ok,
    Wrong,
    /// Too many wrong passwords from this device; seconds until it may try again.
    Locked(u64),
}

#[derive(Default)]
struct State {
    stored: Option<StoredPassword>,
    modified: Option<SystemTime>,
    verified: HashSet<[u8; 32]>,
    failures: HashMap<String, Vec<Instant>>,
}

pub struct Auth {
    path: Option<PathBuf>,
    iterations: u32,
    state: Mutex<State>,
}

fn quick_hash(password: &str) -> [u8; 32] {
    Sha256::digest(password.as_bytes()).into()
}

fn pbkdf2(password: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, iterations, &mut out);
    out
}

fn matches(stored: &StoredPassword, password: &str) -> bool {
    let (Ok(salt), Ok(hash)) = (B64.decode(&stored.salt), B64.decode(&stored.hash)) else {
        return false;
    };
    stored.algorithm == "pbkdf2_sha256" && bool::from(pbkdf2(password, &salt, stored.iterations).ct_eq(hash.as_slice()))
}

impl Auth {
    /// The password file at `path`, or an in-memory one when `path` is None.
    pub fn open(path: Option<PathBuf>) -> Self {
        Self::with_iterations(path, PBKDF2_ITERATIONS)
    }

    /// Tests use fewer PBKDF2 rounds so they run quickly.
    pub fn with_iterations(path: Option<PathBuf>, iterations: u32) -> Self {
        let auth = Auth {
            path,
            iterations,
            state: Mutex::new(State::default()),
        };
        auth.reload(&mut auth.state.lock().unwrap());
        auth
    }

    /// Read the file again if it changed, e.g. after `--set-password` while serving.
    fn reload(&self, state: &mut State) {
        let Some(path) = &self.path else { return };
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if modified == state.modified {
            return;
        }
        state.modified = modified;
        state.verified.clear();
        state.stored = std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str::<AuthFile>(&raw).ok())
            .and_then(|f| f.password);
    }

    pub fn has_password(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        self.reload(&mut state);
        state.stored.is_some()
    }

    pub fn set_password(&self, password: &str) -> Result<(), String> {
        if password.chars().count() < MIN_PASSWORD_CHARS {
            return Err(format!("Use at least {MIN_PASSWORD_CHARS} characters."));
        }
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt).map_err(|e| format!("No randomness available: {e}"))?;
        let stored = StoredPassword {
            algorithm: "pbkdf2_sha256".into(),
            iterations: self.iterations,
            salt: B64.encode(salt),
            hash: B64.encode(pbkdf2(password, &salt, self.iterations)),
        };
        let mut state = self.state.lock().unwrap();
        if let Some(path) = &self.path {
            store::save_json(
                path,
                &AuthFile {
                    password: Some(stored.clone()),
                },
                true,
            )
            .map_err(|e| format!("Could not save {}: {e}", path.display()))?;
            state.modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        }
        state.stored = Some(stored);
        state.verified.clear();
        Ok(())
    }

    /// Seconds until a device with too many wrong passwords may try again, or 0.
    fn wait(state: &mut State, client: &str) -> u64 {
        let now = Instant::now();
        let Some(times) = state.failures.get_mut(client) else { return 0 };
        times.retain(|t| now.duration_since(*t) < WRONG_PASSWORD_WINDOW);
        if times.len() < WRONG_PASSWORDS_ALLOWED {
            if times.is_empty() {
                state.failures.remove(client);
            }
            return 0;
        }
        (WRONG_PASSWORD_WINDOW - now.duration_since(times[0])).as_secs() + 1
    }

    /// Check a request's password. The slow PBKDF2 runs on a blocking thread, and only
    /// for a password not already verified.
    pub async fn check(self: &Arc<Self>, password: &str, client: &str) -> Check {
        let quick = quick_hash(password);
        let stored = {
            let mut state = self.state.lock().unwrap();
            self.reload(&mut state);
            let Some(stored) = state.stored.clone() else { return Check::Wrong };
            if state.verified.contains(&quick) {
                return Check::Ok;
            }
            let wait = Self::wait(&mut state, client);
            if wait > 0 {
                return Check::Locked(wait);
            }
            stored
        };
        let password_owned = password.to_string();
        let checked = stored.clone();
        let ok = tokio::task::spawn_blocking(move || matches(&checked, &password_owned))
            .await
            .unwrap_or(false);

        let mut state = self.state.lock().unwrap();
        if ok {
            // Only remember it against the password that was current when the check began.
            if state.stored.as_ref().is_some_and(|s| s.hash == stored.hash) {
                state.verified.insert(quick);
            }
            state.failures.remove(client);
            Check::Ok
        } else {
            state.failures.entry(client.to_string()).or_default().push(Instant::now());
            if state.failures.len() > 1000 {
                // Forget the device whose oldest failure is oldest; bounded memory under a flood.
                if let Some(oldest) = state
                    .failures
                    .iter()
                    .min_by_key(|(_, t)| t.first().copied())
                    .map(|(k, _)| k.clone())
                {
                    state.failures.remove(&oldest);
                }
            }
            Check::Wrong
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "correct horse battery";

    fn fast(path: Option<PathBuf>) -> Arc<Auth> {
        Arc::new(Auth::with_iterations(path, 1000))
    }

    #[test]
    fn real_installs_hash_slowly() {
        assert_eq!(PBKDF2_ITERATIONS, 600_000);
        assert_eq!(Auth::open(None).iterations, PBKDF2_ITERATIONS);
    }

    #[tokio::test]
    async fn password_is_stored_only_as_a_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data/auth.json");
        let auth = fast(Some(path.clone()));
        assert!(!auth.has_password());
        auth.set_password(PASSWORD).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains(PASSWORD));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let reloaded = fast(Some(path));
        assert_eq!(reloaded.check(PASSWORD, "c").await, Check::Ok);
        assert_eq!(reloaded.check("not the password", "c").await, Check::Wrong);
    }

    #[tokio::test]
    async fn no_password_set_lets_nobody_in() {
        assert_eq!(fast(None).check("", "c").await, Check::Wrong);
    }

    #[test]
    fn short_passwords_are_refused() {
        assert!(fast(None).set_password("short").is_err());
    }

    #[tokio::test]
    async fn a_password_set_while_serving_takes_effect_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let serving = fast(Some(path.clone()));
        fast(Some(path.clone())).set_password(PASSWORD).unwrap();
        assert_eq!(serving.check(PASSWORD, "c").await, Check::Ok);
        std::thread::sleep(Duration::from_millis(20)); // a new modification time
        fast(Some(path)).set_password("another long password").unwrap();
        assert_eq!(
            serving.check(PASSWORD, "c").await,
            Check::Wrong,
            "the remembered old password is forgotten"
        );
        assert_eq!(serving.check("another long password", "c").await, Check::Ok);
    }

    #[tokio::test]
    async fn wrong_passwords_lock_out_that_device_only() {
        let auth = fast(None);
        auth.set_password(PASSWORD).unwrap();
        for _ in 0..WRONG_PASSWORDS_ALLOWED {
            assert_eq!(auth.check("a guess, wrong", "100.64.0.9").await, Check::Wrong);
        }
        assert!(matches!(auth.check(PASSWORD, "100.64.0.9").await, Check::Locked(s) if s > 0));
        assert_eq!(auth.check(PASSWORD, "100.64.0.2").await, Check::Ok);
    }
}
