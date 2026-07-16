use std::{
    collections::HashMap,
    fmt,
    sync::{Mutex, MutexGuard},
};

use frame_domain::{DurationMillis, IdempotencyKey, TenantId, TimestampMillis};

use crate::ApplicationError;

type LedgerKey = (TenantId, String);
type LedgerEntries<T> = HashMap<LedgerKey, Entry<T>>;
type LedgerGuard<'a, T> = MutexGuard<'a, LedgerEntries<T>>;

/// An opaque digest of the command shape, used to reject key reuse with different input.
#[derive(Clone, PartialEq, Eq)]
pub struct CommandFingerprint(String);

impl CommandFingerprint {
    pub fn parse(value: impl Into<String>) -> Result<Self, ApplicationError> {
        let value = value.into();
        if !(16..=128).contains(&value.len())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || matches!(byte, b'-' | b'_' | b':'))
        {
            return Err(ApplicationError::Invalid);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }
}

impl fmt::Debug for CommandFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CommandFingerprint([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandClaim {
    tenant_id: TenantId,
    key: IdempotencyKey,
    fingerprint: CommandFingerprint,
    generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandStatus<T> {
    Claimed(CommandClaim),
    InFlight,
    Replay(T),
}

#[derive(Clone)]
enum Entry<T> {
    Pending {
        fingerprint: CommandFingerprint,
        generation: u64,
        lease_expires_at: TimestampMillis,
    },
    Complete {
        fingerprint: CommandFingerprint,
        result: T,
    },
}

/// A lease-based idempotency ledger that fences stale workers after a crash.
pub struct CommandLedger<T> {
    entries: Mutex<LedgerEntries<T>>,
}

impl<T> Default for CommandLedger<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> fmt::Debug for CommandLedger<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandLedger")
            .field(
                "entry_count",
                &self.entries.lock().map_or(0, |rows| rows.len()),
            )
            .finish()
    }
}

impl<T: Clone> CommandLedger<T> {
    pub fn claim(
        &self,
        tenant_id: TenantId,
        key: IdempotencyKey,
        fingerprint: CommandFingerprint,
        now: TimestampMillis,
        lease_for: DurationMillis,
    ) -> Result<CommandStatus<T>, ApplicationError> {
        let expires_at = now
            .checked_add(lease_for)
            .map_err(|_| ApplicationError::Invalid)?;
        let map_key = (tenant_id, key.expose().to_owned());
        let mut entries = self.lock()?;

        match entries.get(&map_key) {
            Some(Entry::Complete {
                fingerprint: previous,
                result,
            }) => {
                ensure_same_fingerprint(previous, &fingerprint)?;
                return Ok(CommandStatus::Replay(result.clone()));
            }
            Some(Entry::Pending {
                fingerprint: previous,
                lease_expires_at,
                ..
            }) => {
                ensure_same_fingerprint(previous, &fingerprint)?;
                if *lease_expires_at > now {
                    return Ok(CommandStatus::InFlight);
                }
            }
            None => {}
        }

        let generation = match entries.get(&map_key) {
            Some(Entry::Pending { generation, .. }) => generation.saturating_add(1),
            Some(Entry::Complete { .. }) => return Err(ApplicationError::Conflict),
            None => 1,
        };
        entries.insert(
            map_key,
            Entry::Pending {
                fingerprint: fingerprint.clone(),
                generation,
                lease_expires_at: expires_at,
            },
        );
        Ok(CommandStatus::Claimed(CommandClaim {
            tenant_id,
            key,
            fingerprint,
            generation,
        }))
    }

    pub fn complete(&self, claim: &CommandClaim, result: T) -> Result<T, ApplicationError> {
        let map_key = (claim.tenant_id, claim.key.expose().to_owned());
        let mut entries = self.lock()?;
        let Some(Entry::Pending {
            fingerprint,
            generation,
            ..
        }) = entries.get(&map_key)
        else {
            return Err(ApplicationError::Conflict);
        };
        if fingerprint != &claim.fingerprint || *generation != claim.generation {
            return Err(ApplicationError::Conflict);
        }
        entries.insert(
            map_key,
            Entry::Complete {
                fingerprint: claim.fingerprint.clone(),
                result: result.clone(),
            },
        );
        Ok(result)
    }

    pub fn release_retryable(&self, claim: &CommandClaim) -> Result<(), ApplicationError> {
        let map_key = (claim.tenant_id, claim.key.expose().to_owned());
        let mut entries = self.lock()?;
        let matches_claim = matches!(
            entries.get(&map_key),
            Some(Entry::Pending { fingerprint, generation, .. })
                if fingerprint == &claim.fingerprint && *generation == claim.generation
        );
        if !matches_claim {
            return Err(ApplicationError::Conflict);
        }
        entries.remove(&map_key);
        Ok(())
    }

    fn lock(&self) -> Result<LedgerGuard<'_, T>, ApplicationError> {
        self.entries.lock().map_err(|_| ApplicationError::Internal)
    }
}

fn ensure_same_fingerprint(
    previous: &CommandFingerprint,
    next: &CommandFingerprint,
) -> Result<(), ApplicationError> {
    if previous == next {
        Ok(())
    } else {
        Err(ApplicationError::Conflict)
    }
}

#[cfg(test)]
mod tests {
    use frame_domain::{DurationMillis, IdempotencyKey, TenantId, TimestampMillis};

    use super::*;

    fn timestamp(value: i64) -> TimestampMillis {
        TimestampMillis::new(value).expect("valid timestamp")
    }

    fn duration(value: u64) -> DurationMillis {
        DurationMillis::new(value).expect("valid duration")
    }

    fn key() -> IdempotencyKey {
        IdempotencyKey::parse("upload-command-0001").expect("valid key")
    }

    fn fingerprint(value: &str) -> CommandFingerprint {
        CommandFingerprint::parse(value).expect("valid fingerprint")
    }

    #[test]
    fn duplicate_commands_are_in_flight_then_replayed() {
        let ledger = CommandLedger::<String>::default();
        let tenant = TenantId::new();
        let digest = fingerprint("aabbccddeeff0011");
        let first = ledger
            .claim(tenant, key(), digest.clone(), timestamp(10), duration(50))
            .expect("claim");
        let CommandStatus::Claimed(claim) = first else {
            panic!("first request must claim");
        };
        assert_eq!(
            ledger
                .claim(tenant, key(), digest.clone(), timestamp(20), duration(50))
                .expect("duplicate"),
            CommandStatus::InFlight
        );
        ledger
            .complete(&claim, "video-1".to_owned())
            .expect("complete");
        assert_eq!(
            ledger
                .claim(tenant, key(), digest, timestamp(30), duration(50))
                .expect("replay"),
            CommandStatus::Replay("video-1".to_owned())
        );
    }

    #[test]
    fn expired_claim_is_reclaimed_and_stale_completion_is_fenced() {
        let ledger = CommandLedger::<u64>::default();
        let tenant = TenantId::new();
        let digest = fingerprint("0011223344556677");
        let CommandStatus::Claimed(stale) = ledger
            .claim(tenant, key(), digest.clone(), timestamp(1), duration(5))
            .expect("first claim")
        else {
            panic!("claim expected");
        };
        let CommandStatus::Claimed(current) = ledger
            .claim(tenant, key(), digest, timestamp(6), duration(5))
            .expect("reclaim")
        else {
            panic!("reclaim expected");
        };
        assert_eq!(ledger.complete(&stale, 1), Err(ApplicationError::Conflict));
        assert_eq!(ledger.complete(&current, 2), Ok(2));
    }

    #[test]
    fn key_reuse_with_different_command_is_rejected() {
        let ledger = CommandLedger::<u64>::default();
        let tenant = TenantId::new();
        ledger
            .claim(
                tenant,
                key(),
                fingerprint("0011223344556677"),
                timestamp(1),
                duration(5),
            )
            .expect("claim");
        assert_eq!(
            ledger.claim(
                tenant,
                key(),
                fingerprint("8899aabbccddeeff"),
                timestamp(2),
                duration(5),
            ),
            Err(ApplicationError::Conflict)
        );
    }
}
