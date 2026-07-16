use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    rc::Rc,
    time::Duration,
};

use frame_media::*;

fn session(marker: u8) -> InstantSessionId {
    InstantSessionId::from_csprng([marker; 16]).expect("session")
}

fn operation(marker: u8) -> InstantOperationId {
    InstantOperationId::from_csprng([marker; 16]).expect("operation")
}

fn worker(marker: u8) -> InstantWorkerId {
    InstantWorkerId::from_csprng([marker; 16]).expect("worker")
}

fn upload(marker: u8) -> InstantUploadId {
    InstantUploadId::from_csprng([marker; 16]).expect("upload")
}

fn object(marker: u8) -> InstantObjectId {
    InstantObjectId::from_csprng([marker; 16]).expect("object")
}

fn publication(marker: u8) -> InstantPublicationId {
    InstantPublicationId::from_csprng([marker; 16]).expect("publication")
}

fn binding(
    session_id: InstantSessionId,
    marker: u8,
    generation: u64,
    expires_at_ns: u64,
) -> InstantMultipartBinding {
    InstantMultipartBinding {
        session_id,
        upload_id: upload(marker),
        expires_at_ns,
        generation,
        minimum_part_bytes: 1,
        maximum_part_bytes: MAX_INSTANT_SEGMENT_BYTES,
        maximum_parts: MAX_INSTANT_SEGMENTS,
    }
    .validate()
    .expect("binding")
}

fn segment_bytes(index: u32) -> Vec<u8> {
    let length = 257 + usize::try_from(index).expect("index");
    (0..length)
        .map(|offset| u8::try_from((offset + length) % 251).expect("byte"))
        .collect()
}

fn segment_descriptor(session_id: InstantSessionId, index: u32) -> InstantSegmentDescriptor {
    let bytes = segment_bytes(index);
    let start_ns = u64::from(index) * 1_000_000_000;
    let tracks = vec![
        InstantTrackMetadata::new(
            1,
            InstantTrackRole::ScreenVideo,
            InstantCodec::H264Avc,
            90_000,
            30,
            start_ns,
            1_000_000_000,
        )
        .expect("video track"),
        InstantTrackMetadata::new(
            2,
            InstantTrackRole::MixedAudio,
            InstantCodec::AacLowComplexity,
            48_000,
            47,
            start_ns,
            1_000_000_000,
        )
        .expect("audio track"),
    ];
    InstantSegmentDescriptor::new(
        session_id,
        index,
        start_ns,
        1_000_000_000,
        true,
        InstantContainer::FragmentedMp4Cmaf,
        tracks,
        bytes.len() as u64,
        strong_sha256(&bytes),
    )
    .expect("segment")
}

fn spool_receipt(descriptor: &InstantSegmentDescriptor) -> SpoolCommitReceipt {
    SpoolCommitReceipt {
        segment_index: descriptor.index(),
        segment_identity: descriptor.identity(),
        bytes: descriptor.bytes(),
        ciphertext_integrity: strong_sha256(
            format!("ciphertext-{}", descriptor.index()).as_bytes(),
        ),
        durable: true,
    }
}

#[derive(Debug)]
struct MemoryPayload {
    declared: u64,
    chunks: VecDeque<Vec<u8>>,
    cancelled: Rc<Cell<u32>>,
}

impl MemoryPayload {
    fn exact(bytes: Vec<u8>, chunk_size: usize, cancelled: Rc<Cell<u32>>) -> Self {
        let declared = bytes.len() as u64;
        let chunks = bytes
            .chunks(chunk_size)
            .map(<[u8]>::to_vec)
            .collect::<VecDeque<_>>();
        Self {
            declared,
            chunks,
            cancelled,
        }
    }
}

impl InstantSegmentPayload for MemoryPayload {
    fn declared_len(&self) -> u64 {
        self.declared
    }

    fn pull(&mut self, _max_bytes: usize) -> Result<Option<Vec<u8>>, InstantError> {
        Ok(self.chunks.pop_front())
    }

    fn cancel(&mut self) {
        self.cancelled.set(self.cancelled.get().saturating_add(1));
    }
}

#[derive(Debug, Default)]
struct JournalShared {
    snapshot: Option<InstantJournalSnapshot>,
    acknowledge_lost_next: bool,
}

#[derive(Debug, Clone, Default)]
struct FakeJournalPort {
    shared: Rc<RefCell<JournalShared>>,
}

impl FakeJournalPort {
    fn lose_next_acknowledgement(&self) {
        self.shared.borrow_mut().acknowledge_lost_next = true;
    }
}

impl InstantJournalPort for FakeJournalPort {
    fn load(
        &mut self,
        session_id: InstantSessionId,
    ) -> Result<Option<InstantJournalSnapshot>, InstantError> {
        Ok(self
            .shared
            .borrow()
            .snapshot
            .clone()
            .filter(|snapshot| snapshot.session_id() == session_id))
    }

    fn create(
        &mut self,
        initial: InstantJournalSnapshot,
    ) -> Result<JournalPortOutcome<InstantJournalSnapshot>, InstantError> {
        let mut shared = self.shared.borrow_mut();
        if let Some(existing) = shared.snapshot.clone() {
            return Ok(JournalPortOutcome::Conflict(Box::new(existing)));
        }
        shared.snapshot = Some(initial.clone());
        if std::mem::take(&mut shared.acknowledge_lost_next) {
            Ok(JournalPortOutcome::AcknowledgementLost)
        } else {
            Ok(JournalPortOutcome::Committed(initial))
        }
    }

    fn compare_and_swap(
        &mut self,
        request: InstantJournalCasRequest,
    ) -> Result<JournalPortOutcome<InstantJournalSnapshot>, InstantError> {
        let mut shared = self.shared.borrow_mut();
        let current = shared
            .snapshot
            .clone()
            .ok_or(InstantError::JournalMissing)?;
        if current.session_id() != request.session_id
            || current.revision() != request.expected_revision
            || current.fence() != request.expected_fence
        {
            return Ok(JournalPortOutcome::Conflict(Box::new(current)));
        }
        shared.snapshot = Some(request.next.clone());
        if std::mem::take(&mut shared.acknowledge_lost_next) {
            Ok(JournalPortOutcome::AcknowledgementLost)
        } else {
            Ok(JournalPortOutcome::Committed(request.next))
        }
    }
}

type Journal = DurableInstantJournal<FakeJournalPort>;

#[derive(Debug, Clone)]
struct FakeSpoolEntry {
    descriptor: InstantSegmentDescriptor,
    receipt: SpoolCommitReceipt,
    bytes: Vec<u8>,
}

#[derive(Debug, Default)]
struct FakeSpoolShared {
    entries: BTreeMap<(InstantSessionId, u32), FakeSpoolEntry>,
    abort_count: u32,
    evict_count: u32,
    wipe_count: u32,
    disk_full: bool,
    fail_write: bool,
    key_unavailable: bool,
    bad_key_marker: bool,
    corrupt_open: bool,
    corrupt_recovery: bool,
    fail_wipe: bool,
}

#[derive(Debug, Clone)]
struct FakeSpoolPort {
    shared: Rc<RefCell<FakeSpoolShared>>,
    protection: SpoolProtectionCapability,
}

impl Default for FakeSpoolPort {
    fn default() -> Self {
        Self {
            shared: Rc::new(RefCell::new(FakeSpoolShared::default())),
            protection: SpoolProtectionCapability::EncryptedAndAuthenticated {
                algorithm: SpoolAeadAlgorithm::XChaCha20Poly1305,
                atomic_replace: true,
                private_permissions: true,
            },
        }
    }
}

#[derive(Debug)]
struct FakeSpoolLease {
    shared: Rc<RefCell<FakeSpoolShared>>,
    claim: SpoolWriteClaim,
    bytes: Vec<u8>,
    terminal: bool,
}

impl SpoolReservationLease for FakeSpoolLease {
    fn write(&mut self, chunk: &[u8]) -> Result<(), InstantError> {
        if self.shared.borrow().fail_write {
            return Err(InstantError::SpoolDiskFull);
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    fn commit(
        &mut self,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<SpoolCommitReceipt, InstantError> {
        if self.terminal
            || descriptor.index() != self.claim.segment_index
            || descriptor.identity() != self.claim.segment_identity
            || descriptor.bytes() != self.claim.bytes
            || self.bytes.len() as u64 != self.claim.bytes
        {
            return Err(InstantError::InvalidSpoolReceipt);
        }
        let receipt = SpoolCommitReceipt {
            segment_index: self.claim.segment_index,
            segment_identity: self.claim.segment_identity,
            bytes: self.claim.bytes,
            ciphertext_integrity: strong_sha256(&self.bytes),
            durable: true,
        };
        self.shared.borrow_mut().entries.insert(
            (self.claim.session_id, self.claim.segment_index),
            FakeSpoolEntry {
                descriptor: descriptor.clone(),
                receipt,
                bytes: self.bytes.clone(),
            },
        );
        self.terminal = true;
        Ok(receipt)
    }

    fn abort(&mut self) {
        if !self.terminal {
            let mut shared = self.shared.borrow_mut();
            shared.abort_count = shared.abort_count.saturating_add(1);
            self.terminal = true;
        }
    }
}

impl PrivateSpoolPort for FakeSpoolPort {
    fn protection(&self) -> SpoolProtectionCapability {
        self.protection
    }

    fn acquire_runtime_key(
        &mut self,
        _session_id: InstantSessionId,
    ) -> Result<RuntimeSpoolKeyHandle, InstantError> {
        if self.shared.borrow().key_unavailable {
            return Err(InstantError::SpoolKeyUnavailable);
        }
        RuntimeSpoolKeyHandle::from_runtime([9; 16])
    }

    fn key_marker(&self, _key: &RuntimeSpoolKeyHandle) -> Sha256Digest {
        if self.shared.borrow().bad_key_marker {
            strong_sha256(&[8; 16])
        } else {
            strong_sha256(&[9; 16])
        }
    }

    fn reserve(
        &mut self,
        _key: &RuntimeSpoolKeyHandle,
        claim: SpoolWriteClaim,
    ) -> Result<Box<dyn SpoolReservationLease>, InstantError> {
        if self.shared.borrow().disk_full {
            return Err(InstantError::SpoolDiskFull);
        }
        Ok(Box::new(FakeSpoolLease {
            shared: self.shared.clone(),
            claim,
            bytes: Vec::new(),
            terminal: false,
        }))
    }

    fn open(
        &mut self,
        _key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<Box<dyn InstantSegmentPayload>, InstantError> {
        let shared = self.shared.borrow();
        let entry = shared
            .entries
            .get(&(session_id, descriptor.index()))
            .ok_or(InstantError::SpoolEntryMissing)?;
        let mut bytes = entry.bytes.clone();
        if shared.corrupt_open && !bytes.is_empty() {
            bytes[0] ^= 0xff;
        }
        Ok(Box::new(MemoryPayload::exact(
            bytes,
            31,
            Rc::new(Cell::new(0)),
        )))
    }

    fn recover(
        &mut self,
        _key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
    ) -> Result<Vec<RecoveredSpoolEntry>, InstantError> {
        let shared = self.shared.borrow();
        Ok(shared
            .entries
            .iter()
            .filter(|((entry_session, _), _)| *entry_session == session_id)
            .map(|(_, entry)| RecoveredSpoolEntry {
                descriptor: entry.descriptor.clone(),
                commit_receipt: entry.receipt,
                committed: !shared.corrupt_recovery,
            })
            .collect())
    }

    fn evict(
        &mut self,
        _key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
        segment_identity: Sha256Digest,
    ) -> Result<(), InstantError> {
        let mut shared = self.shared.borrow_mut();
        let key = shared
            .entries
            .iter()
            .find(|((entry_session, _), entry)| {
                *entry_session == session_id && entry.descriptor.identity() == segment_identity
            })
            .map(|(key, _)| *key)
            .ok_or(InstantError::SpoolEntryMissing)?;
        shared.entries.remove(&key);
        shared.evict_count = shared.evict_count.saturating_add(1);
        Ok(())
    }

    fn wipe_session(
        &mut self,
        _key: &RuntimeSpoolKeyHandle,
        session_id: InstantSessionId,
    ) -> Result<(), InstantError> {
        let mut shared = self.shared.borrow_mut();
        if shared.fail_wipe {
            return Err(InstantError::SpoolDiskFull);
        }
        shared
            .entries
            .retain(|(entry_session, _), _| *entry_session != session_id);
        shared.wipe_count = shared.wipe_count.saturating_add(1);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FakeProviderBehavior {
    Commit,
    Duplicate,
    AcknowledgementLost,
    Offline,
    Throttled,
    Expired,
    Unavailable,
    CorruptReceipt,
}

#[derive(Debug)]
struct FakeMultipartShared {
    binding: InstantMultipartBinding,
    parts: BTreeMap<u32, InstantPartReceipt>,
    complete: Option<InstantMultipartCompleteReceipt>,
    part_behaviors: VecDeque<FakeProviderBehavior>,
    create_behavior: FakeProviderBehavior,
    renew_behavior: FakeProviderBehavior,
    complete_behavior: FakeProviderBehavior,
    abort_behavior: FakeProviderBehavior,
    aborted: bool,
    put_calls: u32,
    inspect_part_failures: u32,
}

#[derive(Debug, Clone)]
struct FakeMultipartPort {
    shared: Rc<RefCell<FakeMultipartShared>>,
}

impl FakeMultipartPort {
    fn new(binding: InstantMultipartBinding) -> Self {
        Self {
            shared: Rc::new(RefCell::new(FakeMultipartShared {
                binding,
                parts: BTreeMap::new(),
                complete: None,
                part_behaviors: VecDeque::new(),
                create_behavior: FakeProviderBehavior::Commit,
                renew_behavior: FakeProviderBehavior::Commit,
                complete_behavior: FakeProviderBehavior::Commit,
                abort_behavior: FakeProviderBehavior::Commit,
                aborted: false,
                put_calls: 0,
                inspect_part_failures: 0,
            })),
        }
    }

    fn push_part_behavior(&self, behavior: FakeProviderBehavior) {
        self.shared.borrow_mut().part_behaviors.push_back(behavior);
    }

    fn set_complete_behavior(&self, behavior: FakeProviderBehavior) {
        self.shared.borrow_mut().complete_behavior = behavior;
    }

    fn set_create_behavior(&self, behavior: FakeProviderBehavior) {
        self.shared.borrow_mut().create_behavior = behavior;
    }

    fn set_renew_behavior(&self, behavior: FakeProviderBehavior) {
        self.shared.borrow_mut().renew_behavior = behavior;
    }

    fn set_abort_behavior(&self, behavior: FakeProviderBehavior) {
        self.shared.borrow_mut().abort_behavior = behavior;
    }

    fn fail_part_inspection(&self, count: u32) {
        self.shared.borrow_mut().inspect_part_failures = count;
    }
}

fn consume_upload_body(body: &mut ValidatedSegmentPayload) -> Result<(), InstantError> {
    while body.next_chunk()?.is_some() {}
    Ok(())
}

impl InstantMultipartPort for FakeMultipartPort {
    fn create_or_reconcile(
        &mut self,
        _session_id: InstantSessionId,
        _operation_id: InstantOperationId,
    ) -> Result<ProviderCall<InstantMultipartBinding>, InstantError> {
        let shared = self.shared.borrow();
        Ok(match shared.create_behavior {
            FakeProviderBehavior::Commit => ProviderCall::Committed(shared.binding),
            FakeProviderBehavior::Duplicate => ProviderCall::Duplicate(shared.binding),
            FakeProviderBehavior::AcknowledgementLost => ProviderCall::AcknowledgementLost,
            FakeProviderBehavior::Offline => ProviderCall::Offline,
            FakeProviderBehavior::Throttled => ProviderCall::Throttled {
                retry_after: Duration::from_millis(250),
            },
            FakeProviderBehavior::Expired => ProviderCall::Expired,
            FakeProviderBehavior::Unavailable | FakeProviderBehavior::CorruptReceipt => {
                ProviderCall::Unavailable
            }
        })
    }

    fn inspect_upload(
        &mut self,
        _session_id: InstantSessionId,
    ) -> Result<ProviderCall<InstantMultipartBinding>, InstantError> {
        Ok(ProviderCall::Committed(self.shared.borrow().binding))
    }

    fn renew(
        &mut self,
        binding: InstantMultipartBinding,
        _operation_id: InstantOperationId,
    ) -> Result<ProviderCall<InstantMultipartBinding>, InstantError> {
        let behavior = self.shared.borrow().renew_behavior;
        let mut next = binding;
        next.generation = next.generation.saturating_add(1);
        next.expires_at_ns = next.expires_at_ns.saturating_add(10_000_000_000);
        self.shared.borrow_mut().binding = next;
        Ok(match behavior {
            FakeProviderBehavior::Commit => ProviderCall::Committed(next),
            FakeProviderBehavior::Duplicate => ProviderCall::Duplicate(next),
            FakeProviderBehavior::AcknowledgementLost => ProviderCall::AcknowledgementLost,
            FakeProviderBehavior::Offline => ProviderCall::Offline,
            FakeProviderBehavior::Throttled => ProviderCall::Throttled {
                retry_after: Duration::from_millis(250),
            },
            FakeProviderBehavior::Expired => ProviderCall::Expired,
            FakeProviderBehavior::Unavailable | FakeProviderBehavior::CorruptReceipt => {
                ProviderCall::Unavailable
            }
        })
    }

    fn put_part(
        &mut self,
        binding: InstantMultipartBinding,
        ticket: &InstantPartUploadTicket,
        body: &mut ValidatedSegmentPayload,
    ) -> Result<ProviderCall<InstantPartReceipt>, InstantError> {
        let behavior = {
            let mut shared = self.shared.borrow_mut();
            shared.put_calls = shared.put_calls.saturating_add(1);
            shared
                .part_behaviors
                .pop_front()
                .unwrap_or(FakeProviderBehavior::Commit)
        };
        match behavior {
            FakeProviderBehavior::Offline => return Ok(ProviderCall::Offline),
            FakeProviderBehavior::Throttled => {
                return Ok(ProviderCall::Throttled {
                    retry_after: Duration::from_millis(250),
                });
            }
            FakeProviderBehavior::Expired => return Ok(ProviderCall::Expired),
            FakeProviderBehavior::Unavailable => return Ok(ProviderCall::Unavailable),
            _ => {}
        }
        consume_upload_body(body)?;
        let mut receipt = InstantPartReceipt::new(
            binding,
            ticket.descriptor(),
            strong_sha256(format!("provider-part-{}", ticket.descriptor().index()).as_bytes()),
        )?;
        if behavior == FakeProviderBehavior::CorruptReceipt {
            receipt.bytes = receipt.bytes.saturating_add(1);
            return Ok(ProviderCall::Committed(receipt));
        }
        self.shared
            .borrow_mut()
            .parts
            .insert(receipt.part_number, receipt);
        match behavior {
            FakeProviderBehavior::Duplicate => Ok(ProviderCall::Duplicate(receipt)),
            FakeProviderBehavior::AcknowledgementLost => Ok(ProviderCall::AcknowledgementLost),
            _ => Ok(ProviderCall::Committed(receipt)),
        }
    }

    fn inspect_part(
        &mut self,
        _binding: InstantMultipartBinding,
        descriptor: &InstantSegmentDescriptor,
    ) -> Result<ProviderCall<InstantPartReceipt>, InstantError> {
        let mut shared = self.shared.borrow_mut();
        if shared.inspect_part_failures != 0 {
            shared.inspect_part_failures -= 1;
            return Ok(ProviderCall::Unavailable);
        }
        Ok(shared
            .parts
            .get(&descriptor.index().saturating_add(1))
            .copied()
            .map_or(ProviderCall::NotFound, ProviderCall::Committed))
    }

    fn complete(
        &mut self,
        binding: InstantMultipartBinding,
        manifest: &InstantManifest,
        ordered_parts: &[InstantPartReceipt],
        _operation_id: InstantOperationId,
    ) -> Result<ProviderCall<InstantMultipartCompleteReceipt>, InstantError> {
        let behavior = self.shared.borrow().complete_behavior;
        match behavior {
            FakeProviderBehavior::Offline => return Ok(ProviderCall::Offline),
            FakeProviderBehavior::Throttled => {
                return Ok(ProviderCall::Throttled {
                    retry_after: Duration::from_millis(250),
                });
            }
            FakeProviderBehavior::Expired => return Ok(ProviderCall::Expired),
            FakeProviderBehavior::Unavailable => return Ok(ProviderCall::Unavailable),
            _ => {}
        }
        let mut receipt = InstantMultipartCompleteReceipt::new(
            binding,
            manifest,
            ordered_parts,
            object(1),
            strong_sha256(b"immutable-object-version"),
        )?;
        if behavior == FakeProviderBehavior::CorruptReceipt {
            receipt.object.bytes = receipt.object.bytes.saturating_add(1);
            return Ok(ProviderCall::Committed(receipt));
        }
        self.shared.borrow_mut().complete = Some(receipt);
        match behavior {
            FakeProviderBehavior::Duplicate => Ok(ProviderCall::Duplicate(receipt)),
            FakeProviderBehavior::AcknowledgementLost => Ok(ProviderCall::AcknowledgementLost),
            _ => Ok(ProviderCall::Committed(receipt)),
        }
    }

    fn inspect_complete(
        &mut self,
        _binding: InstantMultipartBinding,
        _manifest: &InstantManifest,
    ) -> Result<ProviderCall<InstantMultipartCompleteReceipt>, InstantError> {
        Ok(self
            .shared
            .borrow()
            .complete
            .map_or(ProviderCall::Unavailable, ProviderCall::Committed))
    }

    fn abort(
        &mut self,
        _binding: InstantMultipartBinding,
        _operation_id: InstantOperationId,
    ) -> Result<ProviderCall<()>, InstantError> {
        let behavior = self.shared.borrow().abort_behavior;
        self.shared.borrow_mut().aborted = matches!(
            behavior,
            FakeProviderBehavior::Commit
                | FakeProviderBehavior::Duplicate
                | FakeProviderBehavior::AcknowledgementLost
        );
        Ok(match behavior {
            FakeProviderBehavior::Commit => ProviderCall::Committed(()),
            FakeProviderBehavior::Duplicate => ProviderCall::Duplicate(()),
            FakeProviderBehavior::AcknowledgementLost => ProviderCall::AcknowledgementLost,
            FakeProviderBehavior::Offline => ProviderCall::Offline,
            FakeProviderBehavior::Throttled => ProviderCall::Throttled {
                retry_after: Duration::from_millis(250),
            },
            FakeProviderBehavior::Expired => ProviderCall::Expired,
            FakeProviderBehavior::Unavailable | FakeProviderBehavior::CorruptReceipt => {
                ProviderCall::Unavailable
            }
        })
    }

    fn inspect_abort(
        &mut self,
        _binding: InstantMultipartBinding,
    ) -> Result<ProviderCall<()>, InstantError> {
        Ok(if self.shared.borrow().aborted {
            ProviderCall::Committed(())
        } else {
            ProviderCall::Unavailable
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FakeFinalizeBehavior {
    Publish,
    Duplicate,
    AcknowledgementLost,
    Pending,
    Stale,
    Unavailable,
}

#[derive(Debug)]
struct FakeFinalizeShared {
    behavior: FakeFinalizeBehavior,
    receipt: Option<InstantFinalizeReceipt>,
    cancel_behavior: FakeProviderBehavior,
    cancelled: bool,
}

#[derive(Debug, Clone)]
struct FakeFinalizePort {
    shared: Rc<RefCell<FakeFinalizeShared>>,
}

impl Default for FakeFinalizePort {
    fn default() -> Self {
        Self {
            shared: Rc::new(RefCell::new(FakeFinalizeShared {
                behavior: FakeFinalizeBehavior::Publish,
                receipt: None,
                cancel_behavior: FakeProviderBehavior::Commit,
                cancelled: false,
            })),
        }
    }
}

impl FakeFinalizePort {
    fn set_behavior(&self, behavior: FakeFinalizeBehavior) {
        self.shared.borrow_mut().behavior = behavior;
    }
}

impl InstantFinalizePort for FakeFinalizePort {
    fn reconcile(
        &mut self,
        request: &InstantFinalizeRequest,
        _operation_id: InstantOperationId,
    ) -> Result<FinalizeProviderCall, InstantError> {
        let behavior = self.shared.borrow().behavior;
        if matches!(
            behavior,
            FakeFinalizeBehavior::Publish
                | FakeFinalizeBehavior::Duplicate
                | FakeFinalizeBehavior::AcknowledgementLost
        ) {
            let receipt = InstantFinalizeReceipt::new(request, publication(1))?;
            self.shared.borrow_mut().receipt = Some(receipt);
            return Ok(match behavior {
                FakeFinalizeBehavior::Publish => FinalizeProviderCall::Published(receipt),
                FakeFinalizeBehavior::Duplicate => FinalizeProviderCall::Duplicate(receipt),
                FakeFinalizeBehavior::AcknowledgementLost => {
                    FinalizeProviderCall::AcknowledgementLost
                }
                _ => unreachable!("matched publish behaviors"),
            });
        }
        Ok(match behavior {
            FakeFinalizeBehavior::Pending => FinalizeProviderCall::Pending,
            FakeFinalizeBehavior::Stale => FinalizeProviderCall::StaleGeneration,
            FakeFinalizeBehavior::Unavailable => FinalizeProviderCall::Unavailable,
            _ => unreachable!("publish behaviors returned above"),
        })
    }

    fn inspect(
        &mut self,
        _request: &InstantFinalizeRequest,
    ) -> Result<FinalizeProviderCall, InstantError> {
        Ok(self.shared.borrow().receipt.map_or(
            FinalizeProviderCall::Pending,
            FinalizeProviderCall::Published,
        ))
    }

    fn cancel_job(
        &mut self,
        _session_id: InstantSessionId,
        _fence: InstantFence,
        _operation_id: InstantOperationId,
    ) -> Result<ProviderCall<()>, InstantError> {
        let behavior = self.shared.borrow().cancel_behavior;
        self.shared.borrow_mut().cancelled = matches!(
            behavior,
            FakeProviderBehavior::Commit
                | FakeProviderBehavior::Duplicate
                | FakeProviderBehavior::AcknowledgementLost
        );
        Ok(match behavior {
            FakeProviderBehavior::Commit => ProviderCall::Committed(()),
            FakeProviderBehavior::Duplicate => ProviderCall::Duplicate(()),
            FakeProviderBehavior::AcknowledgementLost => ProviderCall::AcknowledgementLost,
            FakeProviderBehavior::Offline => ProviderCall::Offline,
            FakeProviderBehavior::Throttled => ProviderCall::Throttled {
                retry_after: Duration::from_millis(250),
            },
            FakeProviderBehavior::Expired => ProviderCall::Expired,
            FakeProviderBehavior::Unavailable | FakeProviderBehavior::CorruptReceipt => {
                ProviderCall::Unavailable
            }
        })
    }

    fn inspect_cancel_job(
        &mut self,
        _session_id: InstantSessionId,
    ) -> Result<ProviderCall<()>, InstantError> {
        Ok(if self.shared.borrow().cancelled {
            ProviderCall::Committed(())
        } else {
            ProviderCall::NotFound
        })
    }
}

fn seeded_journal(
    session_id: InstantSessionId,
    segment_count: u32,
) -> (Journal, FakeJournalPort, Vec<InstantSegmentDescriptor>) {
    let port = FakeJournalPort::default();
    let initial =
        InstantJournalSnapshot::new(session_id, InstantFence::new(1).expect("initial fence"))
            .expect("snapshot");
    let mut journal = DurableInstantJournal::create(port.clone(), initial).expect("journal");
    journal
        .apply(
            operation(1),
            InstantJournalCommand::Begin {
                network_available: true,
            },
            1,
        )
        .expect("begin");
    journal
        .apply(
            operation(2),
            InstantJournalCommand::BindMultipart {
                binding: binding(session_id, 5, 1, 10_000_000_000),
            },
            2,
        )
        .expect("bind");
    let mut descriptors = Vec::new();
    for index in 0..segment_count {
        let descriptor = segment_descriptor(session_id, index);
        journal
            .apply(
                operation(u8::try_from(10 + index).expect("operation marker")),
                InstantJournalCommand::CommitSegment {
                    descriptor: descriptor.clone(),
                    spool_receipt: spool_receipt(&descriptor),
                },
                3 + u64::from(index),
            )
            .expect("commit segment");
        descriptors.push(descriptor);
    }
    (journal, port, descriptors)
}

#[test]
fn sha256_segment_and_manifest_identity_are_deterministic_and_strict() {
    assert_eq!(
        strong_sha256(b"abc").to_hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let session_id = session(1);
    let zero = segment_descriptor(session_id, 0);
    let one = segment_descriptor(session_id, 1);
    let a = InstantManifest::from_segments(session_id, vec![one.clone(), zero.clone()])
        .expect("manifest sorts exact indexes");
    let b = InstantManifest::from_segments(session_id, vec![zero.clone(), one.clone()])
        .expect("manifest");
    assert_eq!(a.digest(), b.digest());
    assert_eq!(a.segment_count(), 2);
    assert_eq!(a.duration_ns(), 2_000_000_000);
    assert_eq!(a.total_bytes(), zero.bytes().saturating_add(one.bytes()));

    let gap = segment_descriptor(session_id, 2);
    assert!(matches!(
        InstantManifest::from_segments(session_id, vec![zero, gap]),
        Err(InstantError::SegmentContinuity { .. })
    ));
    assert!(matches!(
        InstantManifest::from_segments(session(2), vec![one]),
        Err(InstantError::SessionBindingMismatch)
    ));
}

#[test]
fn segment_rejects_track_misalignment_and_non_keyframe_boundary() {
    let session_id = session(1);
    let track = InstantTrackMetadata::new(
        1,
        InstantTrackRole::ScreenVideo,
        InstantCodec::H264Avc,
        90_000,
        30,
        1,
        1_000,
    )
    .expect("track");
    assert!(matches!(
        InstantSegmentDescriptor::new(
            session_id,
            0,
            0,
            1_000,
            true,
            InstantContainer::FragmentedMp4Cmaf,
            vec![track.clone()],
            3,
            strong_sha256(b"abc"),
        ),
        Err(InstantError::TrackAlignmentMismatch)
    ));
    let aligned = InstantTrackMetadata::new(
        1,
        InstantTrackRole::ScreenVideo,
        InstantCodec::H264Avc,
        90_000,
        30,
        0,
        1_000,
    )
    .expect("track");
    assert!(matches!(
        InstantSegmentDescriptor::new(
            session_id,
            0,
            0,
            1_000,
            false,
            InstantContainer::FragmentedMp4Cmaf,
            vec![aligned],
            3,
            strong_sha256(b"abc"),
        ),
        Err(InstantError::InvalidSegment)
    ));
}

fn pipeline_capabilities() -> InstantPipelineCapabilities {
    InstantPipelineCapabilities {
        splitmuxsink: true,
        mp4mux_fragmented: true,
        h264_avc_encoder: true,
        aac_lc_encoder: true,
        force_key_unit: true,
        exact_split_running_time: true,
        aligned_audio_fragments: true,
        media_distribution_master: true,
    }
}

fn pipeline_request() -> InstantPipelineRequest {
    InstantPipelineRequest {
        video: InstantVideoCaps {
            width: 1_920,
            height: 1_080,
            frame_rate_numerator: 60,
            frame_rate_denominator: 1,
        },
        audio: Some(InstantAudioCaps {
            sample_rate: 48_000,
            channels: 2,
        }),
        segment_duration_ns: 2_000_000_000,
        max_split_slip_ns: 50_000_000,
    }
}

#[test]
fn distribution_master_graph_is_exact_and_fails_closed() {
    let spec = InstantPipelineSpec::negotiate(pipeline_capabilities(), pipeline_request())
        .expect("eligible graph");
    assert!(spec.distribution_master_eligible());
    assert_eq!(
        spec.nodes().last(),
        Some(&InstantPipelineNode::SplitMuxSink)
    );
    assert!(
        spec.nodes()
            .contains(&InstantPipelineNode::FragmentedMp4Muxer)
    );
    let mut missing_keyframe = pipeline_capabilities();
    missing_keyframe.force_key_unit = false;
    assert!(matches!(
        InstantPipelineSpec::negotiate(missing_keyframe, pipeline_request()),
        Err(InstantError::DistributionMasterUnavailable)
    ));
    let mut missing_alignment = pipeline_capabilities();
    missing_alignment.aligned_audio_fragments = false;
    assert!(matches!(
        InstantPipelineSpec::negotiate(missing_alignment, pipeline_request()),
        Err(InstantError::DistributionMasterUnavailable)
    ));
}

#[test]
fn bounded_payload_detects_empty_truncated_extra_corrupt_and_drop() {
    let descriptor = segment_descriptor(session(1), 0);
    let bytes = segment_bytes(0);
    let cancelled = Rc::new(Cell::new(0));
    let payload = MemoryPayload::exact(bytes.clone(), 17, cancelled.clone());
    let mut validated =
        ValidatedSegmentPayload::new(Box::new(payload), &descriptor, 32).expect("validated body");
    let mut observed = Vec::new();
    while let Some(chunk) = validated.next_chunk().expect("chunk") {
        assert!(!chunk.is_empty());
        assert!(chunk.len() <= 32);
        observed.extend_from_slice(&chunk);
    }
    assert_eq!(observed, bytes);
    assert!(validated.is_complete());
    drop(validated);
    assert_eq!(cancelled.get(), 0);

    let truncated_cancel = Rc::new(Cell::new(0));
    let truncated = MemoryPayload {
        declared: descriptor.bytes(),
        chunks: VecDeque::from([segment_bytes(0)[..10].to_vec()]),
        cancelled: truncated_cancel.clone(),
    };
    let mut body =
        ValidatedSegmentPayload::new(Box::new(truncated), &descriptor, 32).expect("body");
    assert!(body.next_chunk().expect("first").is_some());
    assert!(matches!(
        body.next_chunk(),
        Err(InstantError::PayloadLengthMismatch)
    ));
    drop(body);
    assert_eq!(truncated_cancel.get(), 1);

    let empty = MemoryPayload {
        declared: descriptor.bytes(),
        chunks: VecDeque::from([Vec::new()]),
        cancelled: Rc::new(Cell::new(0)),
    };
    let mut body = ValidatedSegmentPayload::new(Box::new(empty), &descriptor, 32).expect("body");
    assert!(matches!(
        body.next_chunk(),
        Err(InstantError::InvalidPayloadChunk)
    ));

    let corrupt = vec![4_u8; segment_bytes(0).len()];
    let corrupt_cancel = Rc::new(Cell::new(0));
    let corrupt_payload = MemoryPayload::exact(corrupt, 16, corrupt_cancel.clone());
    let mut body =
        ValidatedSegmentPayload::new(Box::new(corrupt_payload), &descriptor, 32).expect("body");
    loop {
        match body.next_chunk() {
            Ok(Some(_)) => {}
            Err(InstantError::PayloadChecksumMismatch) => break,
            other => panic!("unexpected corrupt-body result: {other:?}"),
        }
    }
    drop(body);
    assert_eq!(corrupt_cancel.get(), 1);

    let mut extra_bytes = segment_bytes(0);
    extra_bytes.push(7);
    let extra = MemoryPayload {
        declared: descriptor.bytes(),
        chunks: VecDeque::from([extra_bytes]),
        cancelled: Rc::new(Cell::new(0)),
    };
    let mut body = ValidatedSegmentPayload::new(Box::new(extra), &descriptor, 512).expect("body");
    assert!(matches!(
        body.next_chunk(),
        Err(InstantError::PayloadLengthMismatch)
    ));
}

#[test]
fn journal_recovers_lost_ack_and_rejects_operation_key_reuse() {
    let (mut journal, port, _) = seeded_journal(session(3), 0);
    port.lose_next_acknowledgement();
    let command = InstantJournalCommand::Connectivity {
        network_available: false,
    };
    let receipt = journal
        .apply(operation(40), command.clone(), 40)
        .expect("read-after-ambiguous commit");
    assert_eq!(
        journal.snapshot().state(),
        InstantJournalState::CapturingOffline
    );
    assert_eq!(
        journal
            .apply(operation(40), command, 41)
            .expect("stable operation replay"),
        receipt
    );
    assert!(matches!(
        journal.apply(
            operation(40),
            InstantJournalCommand::Connectivity {
                network_available: true,
            },
            42,
        ),
        Err(InstantError::OperationKeyConflict)
    ));
}

#[test]
fn every_durable_boundary_can_be_reopened_before_the_next_command() {
    let session_id = session(4);
    let (mut journal, port, descriptors) = seeded_journal(session_id, 2);
    for (offset, descriptor) in descriptors.iter().enumerate() {
        let receipt = InstantPartReceipt::new(
            binding(session_id, 5, 1, 10_000_000_000),
            descriptor,
            strong_sha256(format!("provider-{offset}").as_bytes()),
        )
        .expect("receipt");
        journal
            .apply(
                operation(u8::try_from(50 + offset).expect("marker")),
                InstantJournalCommand::VerifyPart {
                    index: descriptor.index(),
                    receipt,
                },
                100 + offset as u64,
            )
            .expect("verify");
        drop(journal);
        journal = DurableInstantJournal::recover(port.clone(), session_id).expect("restart");
    }
    journal
        .apply(
            operation(60),
            InstantJournalCommand::RequestFinalize { job_generation: 7 },
            200,
        )
        .expect("finalize boundary");
    drop(journal);
    let recovered = DurableInstantJournal::recover(port, session_id).expect("restart finalizing");
    assert_eq!(
        recovered.snapshot().state(),
        InstantJournalState::Finalizing
    );
    assert_eq!(
        recovered
            .snapshot()
            .manifest()
            .expect("manifest")
            .segment_count(),
        2
    );
}

#[test]
fn stale_worker_plan_cannot_activate_after_fence_change() {
    let (mut journal, _, _) = seeded_journal(session(5), 1);
    let policy = InstantUploadPolicy {
        maximum_concurrency: 1,
        maximum_attempts: 5,
        claim_lease: Duration::from_secs(10),
        initial_backoff: Duration::from_millis(50),
        maximum_backoff: Duration::from_secs(5),
        maximum_retry_after: Duration::from_secs(30),
    };
    let plan = InstantUploadPlanner::plan_claim(journal.snapshot(), policy, worker(1), 100)
        .expect("plan")
        .expect("claim");
    journal
        .apply(
            operation(70),
            InstantJournalCommand::AdvanceFence {
                next_fence: InstantFence::new(2).expect("fence"),
                worker_id: worker(2),
            },
            101,
        )
        .expect("new owner");
    assert!(matches!(
        InstantUploadPlanner::activate_ticket(journal.snapshot(), plan),
        Err(InstantError::StaleUploadClaim)
    ));
}

fn spool_quota() -> SpoolQuotaPolicy {
    SpoolQuotaPolicy {
        max_retained_bytes: 2_000,
        max_reserved_bytes: 1_000,
        max_segment_bytes: 1_000,
    }
    .validate()
    .expect("quota")
}

#[test]
fn encrypted_spool_commits_recovers_streams_and_evicts_only_with_proof() {
    let session_id = session(6);
    let descriptor = segment_descriptor(session_id, 0);
    let port = FakeSpoolPort::default();
    let shared = port.shared.clone();
    let mut spool =
        InstantSpool::open(port.clone(), session_id, spool_quota()).expect("secure spool");
    let cancel = Rc::new(Cell::new(0));
    let source = MemoryPayload::exact(segment_bytes(0), 19, cancel.clone());
    let receipt = spool
        .commit_segment(operation(80), &descriptor, Box::new(source))
        .expect("atomic commit");
    assert!(receipt.durable);
    assert_eq!(spool.retained_bytes(), descriptor.bytes());
    assert_eq!(cancel.get(), 0);

    let recovered = InstantSpool::open(port.clone(), session_id, spool_quota())
        .expect("open after crash")
        .recover()
        .expect("recover committed entry");
    assert_eq!(recovered.retained_bytes(), descriptor.bytes());
    drop(recovered);

    let mut upload_body = spool.open_upload(&descriptor).expect("stream spool");
    let mut observed = Vec::new();
    while let Some(chunk) = upload_body.next_chunk().expect("spool chunk") {
        observed.extend_from_slice(&chunk);
    }
    assert_eq!(observed, segment_bytes(0));

    let part = InstantPartReceipt::new(
        binding(session_id, 6, 1, 10_000),
        &descriptor,
        strong_sha256(b"provider receipt"),
    )
    .expect("part");
    let wrong_descriptor = segment_descriptor(session_id, 1);
    let wrong = InstantPartReceipt::new(
        binding(session_id, 6, 1, 10_000),
        &wrong_descriptor,
        strong_sha256(b"wrong provider receipt"),
    )
    .expect("wrong part")
    .durability_proof();
    assert!(matches!(
        spool.evict(
            &descriptor,
            wrong,
            SpoolEvictionPolicy::AfterVerifiedRemotePart,
        ),
        Err(InstantError::RemoteDurabilityUnproven)
    ));
    assert!(
        spool
            .evict(
                &descriptor,
                part.durability_proof(),
                SpoolEvictionPolicy::AfterVerifiedRemotePart,
            )
            .expect("verified eviction")
    );
    assert_eq!(spool.retained_bytes(), 0);
    assert_eq!(shared.borrow().evict_count, 1);
}

#[test]
fn spool_fails_closed_for_capability_key_quota_disk_and_corruption() {
    let session_id = session(7);
    let descriptor = segment_descriptor(session_id, 0);
    let insecure = FakeSpoolPort {
        protection: SpoolProtectionCapability::PrivateButUnencrypted,
        ..FakeSpoolPort::default()
    };
    assert!(matches!(
        InstantSpool::open(insecure, session_id, spool_quota()),
        Err(InstantError::SecureSpoolUnavailable)
    ));

    let bad_key = FakeSpoolPort::default();
    bad_key.shared.borrow_mut().key_unavailable = true;
    assert!(matches!(
        InstantSpool::open(bad_key, session_id, spool_quota()),
        Err(InstantError::SpoolKeyUnavailable)
    ));

    let disk = FakeSpoolPort::default();
    disk.shared.borrow_mut().disk_full = true;
    let mut spool = InstantSpool::open(disk, session_id, spool_quota()).expect("open secure spool");
    assert!(matches!(
        spool.commit_segment(
            operation(81),
            &descriptor,
            Box::new(MemoryPayload::exact(
                segment_bytes(0),
                32,
                Rc::new(Cell::new(0)),
            )),
        ),
        Err(InstantError::SpoolDiskFull)
    ));

    let write_fail = FakeSpoolPort::default();
    write_fail.shared.borrow_mut().fail_write = true;
    let shared = write_fail.shared.clone();
    let mut spool =
        InstantSpool::open(write_fail, session_id, spool_quota()).expect("open secure spool");
    assert!(matches!(
        spool.commit_segment(
            operation(82),
            &descriptor,
            Box::new(MemoryPayload::exact(
                segment_bytes(0),
                32,
                Rc::new(Cell::new(0)),
            )),
        ),
        Err(InstantError::SpoolDiskFull)
    ));
    assert_eq!(shared.borrow().abort_count, 1);

    let small_quota = SpoolQuotaPolicy {
        max_retained_bytes: 256,
        max_reserved_bytes: 256,
        max_segment_bytes: 256,
    }
    .validate()
    .expect("small quota");
    let port = FakeSpoolPort::default();
    let mut spool = InstantSpool::open(port, session_id, small_quota).expect("spool");
    assert!(matches!(
        spool.commit_segment(
            operation(83),
            &descriptor,
            Box::new(MemoryPayload::exact(
                segment_bytes(0),
                32,
                Rc::new(Cell::new(0)),
            )),
        ),
        Err(InstantError::SessionBindingMismatch | InstantError::SpoolQuotaExceeded)
    ));

    let corrupt = FakeSpoolPort::default();
    let shared = corrupt.shared.clone();
    let mut spool = InstantSpool::open(corrupt.clone(), session_id, spool_quota()).expect("spool");
    spool
        .commit_segment(
            operation(84),
            &descriptor,
            Box::new(MemoryPayload::exact(
                segment_bytes(0),
                32,
                Rc::new(Cell::new(0)),
            )),
        )
        .expect("commit");
    shared.borrow_mut().corrupt_open = true;
    let mut body = spool.open_upload(&descriptor).expect("open corrupt body");
    loop {
        match body.next_chunk() {
            Ok(Some(_)) => {}
            Err(InstantError::PayloadChecksumMismatch) => break,
            other => panic!("unexpected corrupt spool result: {other:?}"),
        }
    }
    shared.borrow_mut().corrupt_recovery = true;
    assert!(matches!(
        InstantSpool::open(corrupt, session_id, spool_quota())
            .expect("open")
            .recover(),
        Err(InstantError::SpoolCorrupt)
    ));
}

fn upload_policy() -> InstantUploadPolicy {
    InstantUploadPolicy {
        maximum_concurrency: 2,
        maximum_attempts: 6,
        claim_lease: Duration::from_secs(10),
        initial_backoff: Duration::from_millis(50),
        maximum_backoff: Duration::from_secs(2),
        maximum_retry_after: Duration::from_secs(5),
    }
    .validate()
    .expect("upload policy")
}

fn claim_part(
    journal: &mut Journal,
    policy: InstantUploadPolicy,
    worker_id: InstantWorkerId,
    now_ns: u64,
    operation_id: InstantOperationId,
) -> InstantPartUploadTicket {
    let plan = InstantUploadPlanner::plan_claim(journal.snapshot(), policy, worker_id, now_ns)
        .expect("plan")
        .expect("eligible part");
    journal
        .apply(operation_id, plan.command.clone(), now_ns)
        .expect("durable claim");
    InstantUploadPlanner::activate_ticket(journal.snapshot(), plan).expect("ticket")
}

fn apply_part_resolution(
    journal: &mut Journal,
    resolution: InstantPartResolution,
    operation_id: InstantOperationId,
    now_ns: u64,
) {
    let InstantPartResolution::Journal(command) = resolution else {
        panic!("expected journal command");
    };
    journal
        .apply(operation_id, *command, now_ns)
        .expect("apply part resolution");
}

#[test]
fn postcommit_lost_part_ack_is_probed_without_duplicate_upload() {
    let session_id = session(8);
    let (mut journal, _, descriptors) = seeded_journal(session_id, 1);
    let provider_binding = binding(session_id, 5, 1, 10_000_000_000);
    let mut port = FakeMultipartPort::new(provider_binding);
    port.push_part_behavior(FakeProviderBehavior::AcknowledgementLost);
    let ticket = claim_part(&mut journal, upload_policy(), worker(1), 100, operation(90));
    let descriptor = &descriptors[0];
    let mut body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            13,
            Rc::new(Cell::new(0)),
        )),
        descriptor,
        64,
    )
    .expect("body");
    let resolution = upload_instant_part(
        &mut port,
        provider_binding,
        journal.snapshot(),
        ticket,
        &mut body,
        upload_policy(),
        101,
    )
    .expect("lost ack reconciled");
    assert!(body.is_complete());
    apply_part_resolution(&mut journal, resolution, operation(91), 102);
    assert!(matches!(
        journal
            .snapshot()
            .segments()
            .next()
            .expect("segment")
            .upload(),
        SegmentUploadJournalState::Verified(_)
    ));
    assert_eq!(port.shared.borrow().put_calls, 1);
    let progress = InstantProgress::from_snapshot(journal.snapshot(), descriptor.bytes(), None)
        .expect("progress");
    assert_eq!(progress.upload_basis_points, 10_000);
    assert_eq!(progress.verified_segment_count, 1);
}

#[test]
fn offline_throttle_outage_and_reconnect_preserve_one_ordered_part() {
    let session_id = session(9);
    let (mut journal, _, descriptors) = seeded_journal(session_id, 1);
    let provider_binding = binding(session_id, 5, 1, 10_000_000_000);
    let mut port = FakeMultipartPort::new(provider_binding);
    let policy = upload_policy();

    port.push_part_behavior(FakeProviderBehavior::Offline);
    let ticket = claim_part(&mut journal, policy, worker(1), 100, operation(92));
    let mut body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            17,
            Rc::new(Cell::new(0)),
        )),
        &descriptors[0],
        64,
    )
    .expect("body");
    let resolution = upload_instant_part(
        &mut port,
        provider_binding,
        journal.snapshot(),
        ticket,
        &mut body,
        policy,
        101,
    )
    .expect("offline defer");
    assert!(!body.is_complete());
    apply_part_resolution(&mut journal, resolution, operation(93), 101);
    journal
        .apply(
            operation(94),
            InstantJournalCommand::Connectivity {
                network_available: false,
            },
            102,
        )
        .expect("offline state");
    assert!(
        InstantUploadPlanner::plan_claim(journal.snapshot(), policy, worker(1), 1_000_000_000)
            .expect("offline planner")
            .is_none()
    );
    journal
        .apply(
            operation(95),
            InstantJournalCommand::Connectivity {
                network_available: true,
            },
            103,
        )
        .expect("reconnect");

    port.push_part_behavior(FakeProviderBehavior::Throttled);
    let ticket = claim_part(&mut journal, policy, worker(1), 100_000_000, operation(96));
    let mut body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            17,
            Rc::new(Cell::new(0)),
        )),
        &descriptors[0],
        64,
    )
    .expect("body");
    let resolution = upload_instant_part(
        &mut port,
        provider_binding,
        journal.snapshot(),
        ticket,
        &mut body,
        policy,
        100_000_001,
    )
    .expect("throttle defer");
    apply_part_resolution(&mut journal, resolution, operation(97), 100_000_001);
    assert!(matches!(
        journal
            .snapshot()
            .segments()
            .next()
            .expect("segment")
            .upload(),
        SegmentUploadJournalState::Deferred {
            reason: UploadDeferReason::Throttled,
            ..
        }
    ));

    port.push_part_behavior(FakeProviderBehavior::Unavailable);
    let ticket = claim_part(&mut journal, policy, worker(1), 500_000_000, operation(98));
    let mut body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            17,
            Rc::new(Cell::new(0)),
        )),
        &descriptors[0],
        64,
    )
    .expect("body");
    let resolution = upload_instant_part(
        &mut port,
        provider_binding,
        journal.snapshot(),
        ticket,
        &mut body,
        policy,
        500_000_001,
    )
    .expect("outage defer");
    apply_part_resolution(&mut journal, resolution, operation(99), 500_000_001);

    port.push_part_behavior(FakeProviderBehavior::Duplicate);
    let ticket = claim_part(&mut journal, policy, worker(1), 800_000_000, operation(100));
    let mut body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            17,
            Rc::new(Cell::new(0)),
        )),
        &descriptors[0],
        64,
    )
    .expect("body");
    let resolution = upload_instant_part(
        &mut port,
        provider_binding,
        journal.snapshot(),
        ticket,
        &mut body,
        policy,
        800_000_001,
    )
    .expect("duplicate verified");
    apply_part_resolution(&mut journal, resolution, operation(101), 800_000_001);
    assert!(matches!(
        journal
            .snapshot()
            .segments()
            .next()
            .expect("segment")
            .upload(),
        SegmentUploadJournalState::Verified(_)
    ));
}

#[test]
fn expiry_requires_generation_renewal_before_a_new_claim() {
    let session_id = session(10);
    let (mut journal, _, descriptors) = seeded_journal(session_id, 1);
    let old_binding = binding(session_id, 5, 1, 10_000_000_000);
    let mut port = FakeMultipartPort::new(old_binding);
    let policy = upload_policy();
    let ticket = claim_part(
        &mut journal,
        policy,
        worker(1),
        9_000_000_000,
        operation(102),
    );
    let mut body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            23,
            Rc::new(Cell::new(0)),
        )),
        &descriptors[0],
        64,
    )
    .expect("body");
    assert_eq!(
        upload_instant_part(
            &mut port,
            old_binding,
            journal.snapshot(),
            ticket,
            &mut body,
            policy,
            10_000_000_001,
        )
        .expect("renew required"),
        InstantPartResolution::RenewRequired
    );
    let renewed = match port
        .renew(old_binding, operation(103))
        .expect("provider renewal")
    {
        ProviderCall::Committed(binding) => binding,
        other => panic!("unexpected renewal: {other:?}"),
    };
    journal
        .apply(
            operation(104),
            InstantJournalCommand::RenewMultipart { binding: renewed },
            10_000_000_002,
        )
        .expect("journal renewal");
    let plan =
        InstantUploadPlanner::plan_claim(journal.snapshot(), policy, worker(2), 10_000_000_003)
            .expect("planner")
            .expect("new generation claim");
    assert_eq!(plan.attempt, 2);
}

fn verify_all_parts(
    journal: &mut Journal,
    descriptors: &[InstantSegmentDescriptor],
    provider_binding: InstantMultipartBinding,
    operation_start: u8,
) -> Vec<InstantPartReceipt> {
    descriptors
        .iter()
        .enumerate()
        .map(|(index, descriptor)| {
            let receipt = InstantPartReceipt::new(
                provider_binding,
                descriptor,
                strong_sha256(format!("verified-{index}").as_bytes()),
            )
            .expect("receipt");
            journal
                .apply(
                    operation(operation_start.saturating_add(index as u8)),
                    InstantJournalCommand::VerifyPart {
                        index: descriptor.index(),
                        receipt,
                    },
                    1_000 + index as u64,
                )
                .expect("verify");
            receipt
        })
        .collect()
}

#[test]
fn out_of_order_parts_lost_complete_ack_and_callbacks_publish_once() {
    let session_id = session(11);
    let (mut journal, _, descriptors) = seeded_journal(session_id, 2);
    let provider_binding = binding(session_id, 5, 1, 10_000_000_000);
    let second = InstantPartReceipt::new(
        provider_binding,
        &descriptors[1],
        strong_sha256(b"second-first"),
    )
    .expect("second receipt");
    journal
        .apply(
            operation(110),
            InstantJournalCommand::VerifyPart {
                index: 1,
                receipt: second,
            },
            100,
        )
        .expect("out-of-order receipt");
    assert!(matches!(
        journal.apply(
            operation(111),
            InstantJournalCommand::RequestFinalize { job_generation: 3 },
            101,
        ),
        Err(InstantError::SegmentsNotRemotelyVerified)
    ));
    let first = InstantPartReceipt::new(
        provider_binding,
        &descriptors[0],
        strong_sha256(b"first-second"),
    )
    .expect("first receipt");
    journal
        .apply(
            operation(112),
            InstantJournalCommand::VerifyPart {
                index: 0,
                receipt: first,
            },
            102,
        )
        .expect("first receipt");
    journal
        .apply(
            operation(113),
            InstantJournalCommand::RequestFinalize { job_generation: 3 },
            103,
        )
        .expect("finalizing");

    let mut multipart = FakeMultipartPort::new(provider_binding);
    multipart.set_complete_behavior(FakeProviderBehavior::AcknowledgementLost);
    let complete = complete_instant_multipart(&mut multipart, journal.snapshot(), operation(114))
        .expect("complete postcondition probe");
    journal
        .apply(
            operation(115),
            InstantJournalCommand::CompleteMultipart { receipt: complete },
            104,
        )
        .expect("complete journal");
    journal
        .apply(
            operation(116),
            InstantJournalCommand::SealFinalizeRequest,
            105,
        )
        .expect("seal finalize request");
    let request = journal
        .snapshot()
        .finalize_request()
        .expect("finalize request");
    let mut finalize = FakeFinalizePort::default();
    finalize.set_behavior(FakeFinalizeBehavior::AcknowledgementLost);
    let published = reconcile_instant_finalize(&mut finalize, &request, operation(117))
        .expect("finalize reconciliation")
        .expect("published receipt");
    journal
        .apply(
            operation(118),
            InstantJournalCommand::Publish { receipt: published },
            105,
        )
        .expect("publish");
    assert_eq!(journal.snapshot().state(), InstantJournalState::Ready);
    journal
        .apply(
            operation(119),
            InstantJournalCommand::ApplyCallback { receipt: published },
            106,
        )
        .expect("duplicate callback");
    let conflicting = InstantFinalizeReceipt::new(&request, publication(2))
        .expect("structurally valid conflicting publication");
    assert!(matches!(
        journal.apply(
            operation(120),
            InstantJournalCommand::ApplyCallback {
                receipt: conflicting,
            },
            107,
        ),
        Err(InstantError::PublishConflict)
    ));
    let plan = choose_derivative_plan(
        published,
        DerivativeCapabilities {
            managed_media_available: true,
            managed_media_accepts_master: true,
            native_gstreamer_available: true,
        },
        DerivativePreference::PreferManagedMedia,
    )
    .expect("derivative plan");
    assert!(matches!(plan, DerivativePlan::ManagedMedia { .. }));

    journal
        .apply(operation(121), InstantJournalCommand::Cancel, 108)
        .expect("delete tombstone");
    assert!(matches!(
        journal.apply(
            operation(122),
            InstantJournalCommand::ApplyCallback { receipt: published },
            109,
        ),
        Err(InstantError::TombstoneSealed)
    ));
}

#[test]
fn corrupt_provider_receipts_and_wrong_finalize_generation_fail_closed() {
    let session_id = session(12);
    let (mut journal, _, descriptors) = seeded_journal(session_id, 1);
    let provider_binding = binding(session_id, 5, 1, 10_000_000_000);
    let mut multipart = FakeMultipartPort::new(provider_binding);
    multipart.push_part_behavior(FakeProviderBehavior::CorruptReceipt);
    let ticket = claim_part(
        &mut journal,
        upload_policy(),
        worker(1),
        100,
        operation(122),
    );
    let mut body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            19,
            Rc::new(Cell::new(0)),
        )),
        &descriptors[0],
        64,
    )
    .expect("body");
    assert!(matches!(
        upload_instant_part(
            &mut multipart,
            provider_binding,
            journal.snapshot(),
            ticket,
            &mut body,
            upload_policy(),
            101,
        ),
        Err(InstantError::InvalidPartReceipt)
    ));

    let second_session = session(13);
    let (mut journal, _, descriptors) = seeded_journal(second_session, 1);
    let provider_binding = binding(second_session, 5, 1, 10_000_000_000);
    let mut multipart = FakeMultipartPort::new(provider_binding);
    verify_all_parts(&mut journal, &descriptors, provider_binding, 123);
    journal
        .apply(
            operation(124),
            InstantJournalCommand::RequestFinalize { job_generation: 8 },
            200,
        )
        .expect("finalize");
    multipart.set_complete_behavior(FakeProviderBehavior::Commit);
    let complete = complete_instant_multipart(&mut multipart, journal.snapshot(), operation(125))
        .expect("complete");
    journal
        .apply(
            operation(126),
            InstantJournalCommand::CompleteMultipart { receipt: complete },
            201,
        )
        .expect("complete journal");
    journal
        .apply(
            operation(127),
            InstantJournalCommand::SealFinalizeRequest,
            202,
        )
        .expect("seal request");
    let request = journal.snapshot().finalize_request().expect("request");
    let finalize = FakeFinalizePort::default();
    finalize.set_behavior(FakeFinalizeBehavior::Stale);
    assert!(matches!(
        reconcile_instant_finalize(&mut finalize.clone(), &request, operation(128)),
        Err(InstantError::StaleJobGeneration)
    ));
}

fn completed_finalize_journal(
    session_id: InstantSessionId,
) -> (Journal, InstantFinalizeRequest, InstantFinalizeReceipt) {
    let (mut journal, _, descriptors) = seeded_journal(session_id, 1);
    let provider_binding = binding(session_id, 5, 1, 10_000_000_000);
    let parts = verify_all_parts(&mut journal, &descriptors, provider_binding, 130);
    journal
        .apply(
            operation(131),
            InstantJournalCommand::RequestFinalize { job_generation: 4 },
            1_100,
        )
        .expect("finalize");
    let complete = InstantMultipartCompleteReceipt::new(
        provider_binding,
        journal.snapshot().manifest().expect("manifest"),
        &parts,
        object(4),
        strong_sha256(b"completed-object-version"),
    )
    .expect("complete receipt");
    journal
        .apply(
            operation(132),
            InstantJournalCommand::CompleteMultipart { receipt: complete },
            1_101,
        )
        .expect("complete journal");
    journal
        .apply(
            operation(133),
            InstantJournalCommand::SealFinalizeRequest,
            1_102,
        )
        .expect("seal request");
    let request = journal.snapshot().finalize_request().expect("request");
    let receipt = InstantFinalizeReceipt::new(&request, publication(4)).expect("receipt");
    (journal, request, receipt)
}

#[test]
fn finalize_pending_duplicate_and_unavailable_outcomes_are_stable() {
    let (_, request, _) = completed_finalize_journal(session(14));
    let pending = FakeFinalizePort::default();
    pending.set_behavior(FakeFinalizeBehavior::Pending);
    assert_eq!(
        reconcile_instant_finalize(&mut pending.clone(), &request, operation(133))
            .expect("pending"),
        None
    );
    let duplicate = FakeFinalizePort::default();
    duplicate.set_behavior(FakeFinalizeBehavior::Duplicate);
    assert!(
        reconcile_instant_finalize(&mut duplicate.clone(), &request, operation(134))
            .expect("duplicate")
            .is_some()
    );
    let unavailable = FakeFinalizePort::default();
    unavailable.set_behavior(FakeFinalizeBehavior::Unavailable);
    assert!(matches!(
        reconcile_instant_finalize(&mut unavailable.clone(), &request, operation(135)),
        Err(InstantError::ProviderUnavailable)
    ));
}

#[test]
fn concurrency_limit_expired_lease_and_current_snapshot_fence_dispatch() {
    let session_id = session(15);
    let (mut journal, _, descriptors) = seeded_journal(session_id, 2);
    let mut policy = upload_policy();
    policy.maximum_concurrency = 1;
    let long_binding = binding(session_id, 5, 2, 100_000_000_000);
    journal
        .apply(
            operation(135),
            InstantJournalCommand::RenewMultipart {
                binding: long_binding,
            },
            99,
        )
        .expect("long-lived binding");
    let stale_ticket = claim_part(&mut journal, policy, worker(1), 100, operation(136));
    assert!(
        InstantUploadPlanner::plan_claim(journal.snapshot(), policy, worker(2), 101)
            .expect("planner")
            .is_none()
    );
    let replacement_plan =
        InstantUploadPlanner::plan_claim(journal.snapshot(), policy, worker(2), 11_000_000_000)
            .expect("planner")
            .expect("expired lease replacement");
    assert_eq!(replacement_plan.attempt, 2);
    journal
        .apply(
            operation(137),
            replacement_plan.command.clone(),
            11_000_000_000,
        )
        .expect("replace claim");
    let mut provider = FakeMultipartPort::new(long_binding);
    let mut old_body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            16,
            Rc::new(Cell::new(0)),
        )),
        &descriptors[0],
        32,
    )
    .expect("old body");
    assert!(matches!(
        upload_instant_part(
            &mut provider,
            long_binding,
            journal.snapshot(),
            stale_ticket,
            &mut old_body,
            policy,
            11_000_000_001,
        ),
        Err(InstantError::StaleUploadClaim)
    ));
    let replacement = InstantUploadPlanner::activate_ticket(journal.snapshot(), replacement_plan)
        .expect("replacement ticket");
    assert_eq!(replacement.attempt(), 2);
}

#[test]
fn provider_expiry_signal_requires_renewal_even_before_local_deadline() {
    let session_id = session(16);
    let (mut journal, _, descriptors) = seeded_journal(session_id, 1);
    let provider_binding = binding(session_id, 5, 1, 10_000_000_000);
    let mut provider = FakeMultipartPort::new(provider_binding);
    provider.push_part_behavior(FakeProviderBehavior::Expired);
    let ticket = claim_part(
        &mut journal,
        upload_policy(),
        worker(1),
        100,
        operation(138),
    );
    let mut body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            16,
            Rc::new(Cell::new(0)),
        )),
        &descriptors[0],
        32,
    )
    .expect("body");
    assert_eq!(
        upload_instant_part(
            &mut provider,
            provider_binding,
            journal.snapshot(),
            ticket,
            &mut body,
            upload_policy(),
            101,
        )
        .expect("expiry"),
        InstantPartResolution::RenewRequired
    );
}

#[test]
fn sealed_tombstone_drives_abort_job_cancel_and_spool_wipe() {
    let session_id = session(17);
    let (mut journal, _, descriptors) = seeded_journal(session_id, 1);
    let spool_port = FakeSpoolPort::default();
    let spool_shared = spool_port.shared.clone();
    let mut spool = InstantSpool::open(spool_port, session_id, spool_quota()).expect("spool");
    spool
        .commit_segment(
            operation(139),
            &descriptors[0],
            Box::new(MemoryPayload::exact(
                segment_bytes(0),
                17,
                Rc::new(Cell::new(0)),
            )),
        )
        .expect("spool segment");
    journal
        .apply(operation(140), InstantJournalCommand::Cancel, 200)
        .expect("seal tombstone");
    let mut multipart = FakeMultipartPort::new(binding(session_id, 5, 1, 10_000_000_000));
    multipart.set_abort_behavior(FakeProviderBehavior::AcknowledgementLost);
    let mut finalize = FakeFinalizePort::default();
    let report = reconcile_instant_tombstone(
        journal.snapshot(),
        &mut multipart,
        &mut finalize,
        spool,
        operation(141),
        operation(142),
    )
    .expect("cleanup report");
    assert_eq!(
        report,
        InstantCancellationReport {
            upload_aborted: true,
            finalize_job_cancelled: true,
            spool_wiped: true,
            retry_required: false,
        }
    );
    assert!(multipart.shared.borrow().aborted);
    assert!(finalize.shared.borrow().cancelled);
    assert_eq!(spool_shared.borrow().wipe_count, 1);
    assert!(spool_shared.borrow().entries.is_empty());
}

#[test]
fn tombstone_cleanup_attempts_every_branch_and_reports_retry() {
    let session_id = session(18);
    let (mut journal, _, _) = seeded_journal(session_id, 0);
    journal
        .apply(operation(143), InstantJournalCommand::Cancel, 200)
        .expect("tombstone");
    let mut multipart = FakeMultipartPort::new(binding(session_id, 5, 1, 10_000_000_000));
    multipart.set_abort_behavior(FakeProviderBehavior::Unavailable);
    let finalize = FakeFinalizePort::default();
    finalize.shared.borrow_mut().cancel_behavior = FakeProviderBehavior::Unavailable;
    let spool_port = FakeSpoolPort::default();
    spool_port.shared.borrow_mut().fail_wipe = true;
    let spool = InstantSpool::open(spool_port, session_id, spool_quota()).expect("spool");
    let report = reconcile_instant_tombstone(
        journal.snapshot(),
        &mut multipart,
        &mut finalize.clone(),
        spool,
        operation(144),
        operation(145),
    )
    .expect("best-effort report");
    assert_eq!(
        report,
        InstantCancellationReport {
            upload_aborted: false,
            finalize_job_cancelled: false,
            spool_wiped: false,
            retry_required: true,
        }
    );
}

#[test]
fn progress_resource_accounting_and_time_to_share_are_bounded() {
    let (journal, _, descriptors) = seeded_journal(session(19), 2);
    let retained = descriptors
        .iter()
        .map(InstantSegmentDescriptor::bytes)
        .sum();
    let progress = InstantProgress::from_snapshot(
        journal.snapshot(),
        retained,
        Some(InstantPublicErrorCode::UploadDelayed),
    )
    .expect("progress");
    assert_eq!(progress.segment_count, 2);
    assert_eq!(progress.verified_segment_count, 0);
    assert_eq!(progress.total_media_bytes, retained);
    assert_eq!(
        progress.public_error,
        Some(InstantPublicErrorCode::UploadDelayed)
    );
    let budget = InstantResourceBudget {
        maximum_spool_bytes: 2 * 1024 * 1024 * 1024,
        maximum_inflight_upload_bytes: 64 * 1024 * 1024,
        maximum_inflight_parts: 4,
        share_target: Duration::from_secs(5),
    }
    .validate()
    .expect("budget");
    assert_eq!(
        budget
            .simulate_time_to_share(10_000_000, 5_000_000, Duration::from_millis(250))
            .expect("simulation"),
        Duration::from_millis(2_250)
    );
    assert!(matches!(
        budget.simulate_time_to_share(1, 0, Duration::ZERO),
        Err(InstantError::InvalidThroughput)
    ));

    let debug = format!("{:?} {:?}", journal.snapshot(), descriptors[0]);
    assert!(!debug.contains(&descriptors[0].sha256().to_hex()));
    assert!(!debug.contains("1313131313131313"));
}

#[test]
fn multipart_create_and_renew_lost_acks_reconcile_by_postcondition() {
    let current = binding(session(21), 21, 1, 10_000_000_000);
    let provider = FakeMultipartPort::new(current);
    provider.set_create_behavior(FakeProviderBehavior::AcknowledgementLost);
    let created =
        reconcile_instant_multipart_binding(&mut provider.clone(), session(21), operation(146))
            .expect("create postcondition");
    assert_eq!(created, current);
    provider.set_renew_behavior(FakeProviderBehavior::AcknowledgementLost);
    let renewed =
        renew_instant_multipart(&mut provider.clone(), session(21), current, operation(147))
            .expect("renew postcondition");
    assert_eq!(renewed.generation, 2);
    assert!(renewed.expires_at_ns > current.expires_at_ns);
}

#[test]
fn unresolved_lost_ack_stays_probe_only_until_remote_postcondition_is_known() {
    let session_id = session(22);
    let (mut journal, _, descriptors) = seeded_journal(session_id, 1);
    let provider_binding = binding(session_id, 5, 1, 10_000_000_000);
    let mut provider = FakeMultipartPort::new(provider_binding);
    provider.push_part_behavior(FakeProviderBehavior::AcknowledgementLost);
    provider.fail_part_inspection(1);
    let policy = upload_policy();
    let ticket = claim_part(&mut journal, policy, worker(1), 100, operation(148));
    let mut body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            16,
            Rc::new(Cell::new(0)),
        )),
        &descriptors[0],
        32,
    )
    .expect("body");
    let resolution = upload_instant_part(
        &mut provider,
        provider_binding,
        journal.snapshot(),
        ticket,
        &mut body,
        policy,
        101,
    )
    .expect("ambiguous upload");
    apply_part_resolution(&mut journal, resolution, operation(149), 101);
    assert!(matches!(
        journal
            .snapshot()
            .segments()
            .next()
            .expect("segment")
            .upload(),
        SegmentUploadJournalState::ProbeRequired { .. }
    ));

    provider.fail_part_inspection(1);
    let probe_ticket = claim_part(&mut journal, policy, worker(2), 100_000_000, operation(150));
    assert_eq!(probe_ticket.work_kind(), InstantPartWorkKind::Probe);
    let probe_cancelled = Rc::new(Cell::new(0));
    let mut probe_body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            16,
            probe_cancelled.clone(),
        )),
        &descriptors[0],
        32,
    )
    .expect("body");
    let resolution = upload_instant_part(
        &mut provider,
        provider_binding,
        journal.snapshot(),
        probe_ticket,
        &mut probe_body,
        policy,
        100_000_001,
    )
    .expect("probe still unavailable");
    assert!(!probe_body.is_complete());
    apply_part_resolution(&mut journal, resolution, operation(151), 100_000_001);
    drop(probe_body);
    assert_eq!(probe_cancelled.get(), 1);
    assert_eq!(provider.shared.borrow().put_calls, 1);

    let final_probe = claim_part(&mut journal, policy, worker(3), 400_000_000, operation(152));
    assert_eq!(final_probe.work_kind(), InstantPartWorkKind::Probe);
    let mut unused_body = ValidatedSegmentPayload::new(
        Box::new(MemoryPayload::exact(
            segment_bytes(0),
            16,
            Rc::new(Cell::new(0)),
        )),
        &descriptors[0],
        32,
    )
    .expect("body");
    let resolution = upload_instant_part(
        &mut provider,
        provider_binding,
        journal.snapshot(),
        final_probe,
        &mut unused_body,
        policy,
        400_000_001,
    )
    .expect("probe finds committed part");
    apply_part_resolution(&mut journal, resolution, operation(153), 400_000_001);
    assert_eq!(provider.shared.borrow().put_calls, 1);
    assert!(matches!(
        journal
            .snapshot()
            .segments()
            .next()
            .expect("segment")
            .upload(),
        SegmentUploadJournalState::Verified(_)
    ));
}

#[test]
fn durably_sealed_finalize_request_survives_later_journal_revisions() {
    let (mut journal, request, receipt) = completed_finalize_journal(session(23));
    let digest = request.digest();
    journal
        .apply(
            operation(154),
            InstantJournalCommand::AdvanceFence {
                next_fence: InstantFence::new(2).expect("new fence"),
                worker_id: worker(4),
            },
            2_000,
        )
        .expect("new recovery owner");
    assert_eq!(
        journal
            .snapshot()
            .finalize_request()
            .expect("persisted request")
            .digest(),
        digest
    );
    journal
        .apply(
            operation(155),
            InstantJournalCommand::Publish { receipt },
            2_001,
        )
        .expect("exact old request receipt remains valid");
    assert_eq!(journal.snapshot().state(), InstantJournalState::Ready);
}

#[test]
fn spool_and_journal_reconcile_every_atomic_commit_and_eviction_crash_window() {
    let session_id = session(24);
    let descriptor = segment_descriptor(session_id, 0);
    let spool_port = FakeSpoolPort::default();
    let mut spool =
        InstantSpool::open(spool_port.clone(), session_id, spool_quota()).expect("spool");
    let commit_receipt = spool
        .commit_segment(
            operation(156),
            &descriptor,
            Box::new(MemoryPayload::exact(
                segment_bytes(0),
                17,
                Rc::new(Cell::new(0)),
            )),
        )
        .expect("spool commit before journal");
    drop(spool);
    let mut spool = InstantSpool::open(spool_port.clone(), session_id, spool_quota())
        .expect("spool restart")
        .recover()
        .expect("recover orphan");
    let (mut journal, _, _) = seeded_journal(session_id, 0);
    let actions = spool
        .reconcile_journal(journal.snapshot())
        .expect("orphan plan");
    assert_eq!(actions.len(), 1);
    let SpoolRecoveryAction::JournalOrphanCommit {
        descriptor: recovered,
        receipt,
    } = actions.into_iter().next().expect("action")
    else {
        panic!("expected orphan commit action");
    };
    assert_eq!(recovered, descriptor);
    assert_eq!(receipt, commit_receipt);
    journal
        .apply(
            operation(157),
            InstantJournalCommand::CommitSegment {
                descriptor: recovered,
                spool_receipt: receipt,
            },
            300,
        )
        .expect("reattach orphan");
    assert!(
        spool
            .reconcile_journal(journal.snapshot())
            .expect("synchronized")
            .is_empty()
    );

    let part = InstantPartReceipt::new(
        binding(session_id, 5, 1, 10_000_000_000),
        &descriptor,
        strong_sha256(b"recovery-part"),
    )
    .expect("part");
    journal
        .apply(
            operation(158),
            InstantJournalCommand::VerifyPart {
                index: 0,
                receipt: part,
            },
            301,
        )
        .expect("verify");
    spool
        .evict(
            &descriptor,
            part.durability_proof(),
            SpoolEvictionPolicy::AfterVerifiedRemotePart,
        )
        .expect("physical eviction before journal");
    let actions = spool
        .reconcile_journal(journal.snapshot())
        .expect("missing verified file is repairable");
    assert!(matches!(
        actions.as_slice(),
        [SpoolRecoveryAction::JournalVerifiedEviction { index: 0, .. }]
    ));
    let SpoolRecoveryAction::JournalVerifiedEviction { proof, .. } = actions[0].clone() else {
        panic!("expected journal eviction");
    };
    journal
        .apply(
            operation(159),
            InstantJournalCommand::EvictSpool {
                index: 0,
                proof,
                policy: SpoolEvictionPolicy::AfterVerifiedRemotePart,
            },
            302,
        )
        .expect("close physical-first crash window");
    assert!(
        spool
            .reconcile_journal(journal.snapshot())
            .expect("synchronized")
            .is_empty()
    );

    let journal_first_session = session(26);
    let journal_first_descriptor = segment_descriptor(journal_first_session, 0);
    let journal_first_port = FakeSpoolPort::default();
    let mut journal_first_spool =
        InstantSpool::open(journal_first_port, journal_first_session, spool_quota())
            .expect("spool");
    let journal_first_receipt = journal_first_spool
        .commit_segment(
            operation(160),
            &journal_first_descriptor,
            Box::new(MemoryPayload::exact(
                segment_bytes(0),
                17,
                Rc::new(Cell::new(0)),
            )),
        )
        .expect("commit");
    let (mut journal_first, _, _) = seeded_journal(journal_first_session, 0);
    journal_first
        .apply(
            operation(161),
            InstantJournalCommand::CommitSegment {
                descriptor: journal_first_descriptor.clone(),
                spool_receipt: journal_first_receipt,
            },
            400,
        )
        .expect("journal segment");
    let journal_first_part = InstantPartReceipt::new(
        binding(journal_first_session, 5, 1, 10_000_000_000),
        &journal_first_descriptor,
        strong_sha256(b"journal-first-part"),
    )
    .expect("part");
    journal_first
        .apply(
            operation(162),
            InstantJournalCommand::VerifyPart {
                index: 0,
                receipt: journal_first_part,
            },
            401,
        )
        .expect("verify");
    journal_first
        .apply(
            operation(163),
            InstantJournalCommand::EvictSpool {
                index: 0,
                proof: journal_first_part.durability_proof(),
                policy: SpoolEvictionPolicy::AfterVerifiedRemotePart,
            },
            402,
        )
        .expect("journal eviction before physical deletion");
    let actions = journal_first_spool
        .reconcile_journal(journal_first.snapshot())
        .expect("journal-first plan");
    assert!(matches!(
        actions.as_slice(),
        [SpoolRecoveryAction::RemoveAlreadyJournaledEviction { .. }]
    ));
    let SpoolRecoveryAction::RemoveAlreadyJournaledEviction { descriptor, proof } =
        actions[0].clone()
    else {
        panic!("expected physical eviction action");
    };
    journal_first_spool
        .evict(
            &descriptor,
            proof,
            SpoolEvictionPolicy::AfterVerifiedRemotePart,
        )
        .expect("close journal-first crash window");
    assert!(
        journal_first_spool
            .reconcile_journal(journal_first.snapshot())
            .expect("synchronized")
            .is_empty()
    );

    let missing_session = session(25);
    let (missing_journal, _, _) = seeded_journal(missing_session, 1);
    let empty_spool = InstantSpool::open(FakeSpoolPort::default(), missing_session, spool_quota())
        .expect("empty spool")
        .recover()
        .expect("empty recovery");
    assert!(matches!(
        empty_spool.reconcile_journal(missing_journal.snapshot()),
        Err(InstantError::SpoolCorrupt)
    ));
}

#[test]
fn canonical_journal_codec_round_trips_states_and_rejects_hostile_bytes() {
    let (mut ready, _, publication_receipt) = completed_finalize_journal(session(27));
    ready
        .apply(
            operation(164),
            InstantJournalCommand::Publish {
                receipt: publication_receipt,
            },
            3_000,
        )
        .expect("ready");
    let encoded = InstantJournalCodec::encode(ready.snapshot()).expect("encode ready");
    let decoded = InstantJournalCodec::decode(&encoded).expect("decode ready");
    assert_eq!(&decoded, ready.snapshot());

    let (mut inflight, _, _) = seeded_journal(session(28), 1);
    let plan =
        InstantUploadPlanner::plan_claim(inflight.snapshot(), upload_policy(), worker(5), 100)
            .expect("plan")
            .expect("claim");
    inflight
        .apply(operation(165), plan.command, 100)
        .expect("inflight");
    let encoded_inflight =
        InstantJournalCodec::encode(inflight.snapshot()).expect("encode inflight");
    assert_eq!(
        InstantJournalCodec::decode(&encoded_inflight).expect("decode inflight"),
        *inflight.snapshot()
    );

    let mut truncated = encoded.clone();
    truncated.pop();
    assert!(matches!(
        InstantJournalCodec::decode(&truncated),
        Err(InstantError::MalformedJournalEncoding)
    ));
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(matches!(
        InstantJournalCodec::decode(&trailing),
        Err(InstantError::MalformedJournalEncoding)
    ));
    let mut wrong_version = encoded.clone();
    wrong_version[8] = 1;
    let payload_length = wrong_version.len() - 32;
    let integrity = strong_sha256(&wrong_version[..payload_length]).to_hex();
    for index in 0..32 {
        wrong_version[payload_length + index] =
            u8::from_str_radix(&integrity[index * 2..index * 2 + 2], 16).expect("integrity byte");
    }
    assert!(matches!(
        InstantJournalCodec::decode(&wrong_version),
        Err(InstantError::UnsupportedJournalVersion)
    ));
    let mut tampered = encoded;
    let last = tampered.len().saturating_sub(1);
    tampered[last] ^= 1;
    assert!(InstantJournalCodec::decode(&tampered).is_err());
}

#[test]
fn multipart_minimum_maximum_and_part_count_are_enforced_before_complete() {
    let session_id = session(29);
    let initial = InstantJournalSnapshot::new(session_id, InstantFence::new(1).expect("fence"))
        .expect("snapshot");
    let port = FakeJournalPort::default();
    let mut journal = DurableInstantJournal::create(port, initial).expect("journal");
    journal
        .apply(
            operation(166),
            InstantJournalCommand::Begin {
                network_available: true,
            },
            1,
        )
        .expect("begin");
    let constrained = InstantMultipartBinding {
        session_id,
        upload_id: upload(29),
        expires_at_ns: 10_000_000_000,
        generation: 1,
        minimum_part_bytes: 300,
        maximum_part_bytes: 1_000,
        maximum_parts: 2,
    }
    .validate()
    .expect("binding");
    journal
        .apply(
            operation(167),
            InstantJournalCommand::BindMultipart {
                binding: constrained,
            },
            2,
        )
        .expect("bind");
    let descriptors = [
        segment_descriptor(session_id, 0),
        segment_descriptor(session_id, 1),
    ];
    for (offset, descriptor) in descriptors.iter().enumerate() {
        journal
            .apply(
                operation(168 + offset as u8),
                InstantJournalCommand::CommitSegment {
                    descriptor: descriptor.clone(),
                    spool_receipt: spool_receipt(descriptor),
                },
                3 + offset as u64,
            )
            .expect("segment within maximum");
        let receipt = InstantPartReceipt::new(
            constrained,
            descriptor,
            strong_sha256(format!("size-part-{offset}").as_bytes()),
        )
        .expect("part");
        journal
            .apply(
                operation(170 + offset as u8),
                InstantJournalCommand::VerifyPart {
                    index: descriptor.index(),
                    receipt,
                },
                5 + offset as u64,
            )
            .expect("verify");
    }
    assert!(matches!(
        journal.apply(
            operation(172),
            InstantJournalCommand::RequestFinalize { job_generation: 1 },
            7,
        ),
        Err(InstantError::MultipartPartSizeMismatch)
    ));

    let over_count = segment_descriptor(session_id, 2);
    assert!(matches!(
        journal.apply(
            operation(173),
            InstantJournalCommand::CommitSegment {
                descriptor: over_count.clone(),
                spool_receipt: spool_receipt(&over_count),
            },
            8,
        ),
        Err(InstantError::MultipartPartSizeMismatch)
    ));
}
