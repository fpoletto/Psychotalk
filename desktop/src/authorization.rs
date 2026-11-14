use rand::{RngCore, rngs::OsRng};
use serde::Serialize;
use std::time::{Duration, Instant, SystemTime};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const IDLE_LIMIT: Duration = Duration::from_secs(8 * 60 * 60);

fn secret() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0; 32]);
    OsRng.fill_bytes(bytes.as_mut());
    Zeroizing::new(hex::encode(bytes.as_ref()))
}
fn matches(expected: Option<&Zeroizing<String>>, supplied: &str) -> bool {
    expected.is_some_and(|value| {
        supplied.len() == value.len() && bool::from(value.as_bytes().ct_eq(supplied.as_bytes()))
    })
}

#[derive(Clone, Copy)]
pub(crate) enum ProofKind {
    Bootstrap,
    Resume,
}
#[derive(Serialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct Grant {
    pub token: String,
    pub resume: String,
}
pub(crate) struct Authorization {
    bootstrap: Option<Zeroizing<String>>,
    access: Option<Zeroizing<String>>,
    resume: Option<Zeroizing<String>>,
    tab: Option<String>,
    generation: u64,
    activity: Instant,
    wall_activity: SystemTime,
}
impl Authorization {
    pub fn new(now: Instant) -> Self {
        Self {
            bootstrap: Some(secret()),
            access: None,
            resume: None,
            tab: None,
            generation: 0,
            activity: now,
            wall_activity: SystemTime::now(),
        }
    }
    pub fn bootstrap(&self) -> &str {
        self.bootstrap.as_ref().map_or("", |s| s.as_str())
    }
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.bootstrap = None;
        self.access = None;
        self.resume = None;
        self.tab = None;
    }
    pub fn reset(&mut self, now: Instant) {
        self.invalidate();
        self.bootstrap = Some(secret());
        self.activity = now;
        self.wall_activity = SystemTime::now();
    }
    fn expire(&mut self, now: Instant) {
        // Wall time covers system sleep; monotonic time prevents clock rollback from extending an active process.
        if now.saturating_duration_since(self.activity) >= IDLE_LIMIT
            || SystemTime::now()
                .duration_since(self.wall_activity)
                .is_ok_and(|idle| idle >= IDLE_LIMIT)
        {
            self.invalidate();
        }
    }
    pub fn authenticate(&mut self, token: &str, tab: &str, now: Instant) -> Option<u64> {
        self.expire(now);
        if matches(self.access.as_ref(), token) && self.tab.as_deref() == Some(tab) {
            self.activity = now;
            self.wall_activity = SystemTime::now();
            Some(self.generation)
        } else {
            None
        }
    }
    pub fn current(&self, generation: u64) -> bool {
        self.access.is_some() && self.generation == generation
    }
    pub fn consume(&mut self, kind: ProofKind, proof: &str, now: Instant) -> bool {
        self.expire(now);
        let expected = match kind {
            ProofKind::Bootstrap => self.bootstrap.as_ref(),
            ProofKind::Resume => self.resume.as_ref(),
        };
        if !matches(expected, proof) {
            return false;
        }
        self.invalidate();
        true
    }
    // Call only after the previous vault session has been closed, under the operation gate.
    pub fn issue(&mut self, tab: String, now: Instant) -> Grant {
        let token = secret();
        let resume = secret();
        let reply = Grant {
            token: token.to_string(),
            resume: resume.to_string(),
        };
        self.access = Some(token);
        self.resume = Some(resume);
        self.tab = Some(tab);
        self.activity = now;
        self.wall_activity = SystemTime::now();
        reply
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proofs_are_separate_single_use_and_bound_to_one_document() {
        let now = Instant::now();
        let mut auth = Authorization::new(now);
        let bootstrap = auth.bootstrap().to_string();
        assert!(auth.authenticate(&bootstrap, "first", now).is_none());
        assert!(!auth.consume(ProofKind::Resume, &bootstrap, now));
        assert!(auth.consume(ProofKind::Bootstrap, &bootstrap, now));
        let first = auth.issue("first".into(), now);
        assert!(!auth.consume(ProofKind::Bootstrap, &bootstrap, now));
        assert!(auth.authenticate(&first.resume, "first", now).is_none());
        assert!(auth.authenticate(&first.token, "second", now).is_none());
        let generation = auth.authenticate(&first.token, "first", now).unwrap();
        assert!(!auth.consume(ProofKind::Resume, "wrong", now));
        assert!(auth.current(generation));
        assert!(auth.consume(ProofKind::Resume, &first.resume, now));
        assert!(!auth.current(generation));
        let second = auth.issue("second".into(), now);
        assert_ne!(first.token, second.token);
        assert_ne!(first.resume, second.resume);
        assert!(!auth.consume(ProofKind::Resume, &first.resume, now));
        assert!(auth.authenticate(&first.token, "first", now).is_none());
        assert!(auth.authenticate(&second.token, "second", now).is_some());
    }

    #[test]
    fn idle_expiration_uses_native_time_and_only_valid_requests_extend_it() {
        let now = Instant::now();
        let mut auth = Authorization::new(now);
        let bootstrap = auth.bootstrap().to_string();
        assert!(auth.consume(ProofKind::Bootstrap, &bootstrap, now));
        let grant = auth.issue("window".into(), now);
        assert!(
            auth.authenticate("wrong", "window", now + IDLE_LIMIT / 2)
                .is_none()
        );
        assert!(!auth.consume(ProofKind::Resume, &grant.resume, now + IDLE_LIMIT));
        assert!(
            auth.authenticate(&grant.token, "window", now + IDLE_LIMIT)
                .is_none()
        );
        auth.reset(now);
        let bootstrap = auth.bootstrap().to_string();
        assert!(auth.consume(ProofKind::Bootstrap, &bootstrap, now));
        let grant = auth.issue("window".into(), now);
        assert!(
            auth.authenticate(&grant.token, "window", now + IDLE_LIMIT / 2)
                .is_some()
        );
        assert!(auth.consume(ProofKind::Resume, &grant.resume, now + IDLE_LIMIT));
    }

    #[test]
    fn elapsed_wall_time_expires_proof_even_if_the_monotonic_clock_paused() {
        let now = Instant::now();
        let mut auth = Authorization::new(now);
        let bootstrap = auth.bootstrap().to_string();
        assert!(auth.consume(ProofKind::Bootstrap, &bootstrap, now));
        let grant = auth.issue("window".into(), now);
        auth.wall_activity = SystemTime::now() - IDLE_LIMIT;
        assert!(!auth.consume(ProofKind::Resume, &grant.resume, now));
    }

    #[test]
    fn reset_wipe_and_process_restart_reject_previous_proofs() {
        let now = Instant::now();
        let mut auth = Authorization::new(now);
        let old_bootstrap = auth.bootstrap().to_string();
        assert!(auth.consume(ProofKind::Bootstrap, &old_bootstrap, now));
        let grant = auth.issue("window".into(), now);
        auth.reset(now);
        assert!(!auth.consume(ProofKind::Resume, &grant.resume, now));
        assert!(!auth.consume(ProofKind::Bootstrap, &old_bootstrap, now));
        let bootstrap = auth.bootstrap().to_string();
        assert!(auth.consume(ProofKind::Bootstrap, &bootstrap, now));
        let grant = auth.issue("window".into(), now);
        auth.invalidate();
        assert!(!auth.consume(ProofKind::Resume, &grant.resume, now));
        let mut restarted = Authorization::new(now);
        assert!(!restarted.consume(ProofKind::Resume, &grant.resume, now));
        assert!(!restarted.consume(ProofKind::Bootstrap, &bootstrap, now));
    }
}
